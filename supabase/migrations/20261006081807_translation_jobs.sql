-- Queue for the AI translation pipeline (built later).
-- A reader (or an admin) asks for a chapter in a language and style; a worker
-- running with the service role picks the job up, writes the result into
-- chapter_translations and links it back here.

create type public.translation_job_status as enum ('queued', 'running', 'succeeded', 'failed', 'cancelled');

create table public.translation_jobs (
  id              uuid primary key default gen_random_uuid(),
  chapter_id      bigint not null references public.chapters (id) on delete cascade,
  source_lang     text references public.languages (code),
  target_lang     text not null references public.languages (code),
  style           public.translation_style not null default 'natural',
  -- "Apply dictionary" in the reader: feed the novel's glossary to the model.
  apply_glossary  boolean not null default true,
  status          public.translation_job_status not null default 'queued',
  -- Higher runs first.
  priority        smallint not null default 0,
  attempts        smallint not null default 0 check (attempts >= 0),
  requested_by    uuid default auth.uid() references auth.users (id) on delete set null,
  model           text,
  error           text,
  result_id       bigint references public.chapter_translations (id) on delete set null,
  created_at      timestamptz not null default now(),
  started_at      timestamptz,
  finished_at     timestamptz,
  updated_at      timestamptz not null default now(),
  constraint translation_jobs_langs_differ check (source_lang is null or source_lang <> target_lang)
);

-- One live job per chapter version: asking twice joins the existing job.
create unique index translation_jobs_one_active_idx
  on public.translation_jobs (chapter_id, target_lang, style)
  where status in ('queued', 'running');

-- Worker poll: next queued job by priority, then age.
create index translation_jobs_queue_idx
  on public.translation_jobs (priority desc, created_at)
  where status = 'queued';

create index translation_jobs_requested_by_idx on public.translation_jobs (requested_by, created_at desc);
create index translation_jobs_result_id_idx on public.translation_jobs (result_id);

create trigger translation_jobs_set_updated_at
before update on public.translation_jobs
for each row execute function public.set_updated_at();

-- Fill source_lang from the novel when the caller does not say.
create or replace function public.translation_jobs_fill()
returns trigger
language plpgsql
set search_path = ''
as $$
begin
  if new.source_lang is null then
    select n.original_lang into new.source_lang
      from public.chapters c join public.novels n on n.id = c.novel_id
     where c.id = new.chapter_id;
  end if;
  return new;
end;
$$;

create trigger translation_jobs_fill
before insert on public.translation_jobs
for each row execute function public.translation_jobs_fill();

-- Worker helper: atomically claim the next queued job. Service role only.
create or replace function public.claim_translation_job(worker_model text default null)
returns public.translation_jobs
language plpgsql
set search_path = ''
as $$
declare
  job public.translation_jobs;
begin
  update public.translation_jobs j
     set status = 'running',
         started_at = now(),
         attempts = j.attempts + 1,
         model = coalesce(worker_model, j.model)
   where j.id = (
     select q.id from public.translation_jobs q
      where q.status = 'queued'
      order by q.priority desc, q.created_at
      limit 1
      for update skip locked
   )
  returning j.* into job;
  return job;
end;
$$;

revoke execute on function public.claim_translation_job(text) from public, anon, authenticated;
grant execute on function public.claim_translation_job(text) to service_role;

-- Reader entry point (AI Translate in the reader tools). Returns the live job
-- for this chapter/language/style, creating it if none is queued or running,
-- so two readers asking for the same thing share one job.
create or replace function public.request_translation(
  p_chapter_id     bigint,
  p_target_lang    text,
  p_style          public.translation_style default 'natural',
  p_apply_glossary boolean default true
)
returns public.translation_jobs
language plpgsql
security definer
set search_path = ''
as $$
declare
  job public.translation_jobs;
begin
  if (select auth.uid()) is null then
    raise exception 'sign in to request a translation' using errcode = '42501';
  end if;

  -- Same visibility rule as the chapters policy: released chapter of a published novel.
  perform 1
     from public.chapters c join public.novels n on n.id = c.novel_id
    where c.id = p_chapter_id and n.is_published
      and c.published_at is not null and c.published_at <= now();
  if not found then
    raise exception 'chapter % not found', p_chapter_id using errcode = 'P0002';
  end if;

  perform 1 from public.languages l where l.code = p_target_lang and l.enabled;
  if not found then
    raise exception 'language % is not available', p_target_lang using errcode = '22023';
  end if;

  insert into public.translation_jobs (chapter_id, target_lang, style, apply_glossary, requested_by)
  values (p_chapter_id, p_target_lang, p_style, p_apply_glossary, (select auth.uid()))
  on conflict (chapter_id, target_lang, style) where status in ('queued', 'running') do nothing
  returning * into job;

  if job.id is null then
    select * into job from public.translation_jobs j
     where j.chapter_id = p_chapter_id and j.target_lang = p_target_lang and j.style = p_style
       and j.status in ('queued', 'running');
  end if;
  return job;
end;
$$;

revoke execute on function public.request_translation(bigint, text, public.translation_style, boolean) from public, anon;
grant execute on function public.request_translation(bigint, text, public.translation_style, boolean) to authenticated;

-- ---------------------------------------------------------------------------
-- Row level security
-- ---------------------------------------------------------------------------

alter table public.translation_jobs enable row level security;

-- Jobs are created through request_translation() and moved along by the worker
-- (service role), so readers only get select. Any signed-in reader may watch a
-- job, since jobs are shared between everyone who asked for the same chapter.
create policy "signed-in users read translation jobs" on public.translation_jobs
  for select to authenticated using (true);
