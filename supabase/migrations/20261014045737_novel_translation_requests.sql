-- Batch entry points for AI translation: "Translate from this chapter" in the reader and
-- "Translate the whole novel" on the detail page. They queue one job per chapter, like
-- request_translation() does for a single chapter, and report progress per language.

-- Queues every released chapter of a published novel from `p_from_number` on that has no
-- published text in `p_target_lang` yet. Chapters with a live job for the same language
-- and style join it. Returns how many chapters are queued or running after the call
-- (0 when all of them already read in that language, the original one included).
create or replace function public.request_novel_translation(
  p_novel_id       bigint,
  p_target_lang    text,
  p_from_number    integer default 1,
  p_style          public.translation_style default 'natural',
  p_apply_glossary boolean default true
)
returns integer
language plpgsql
security definer
set search_path = ''
as $$
declare
  live integer;
begin
  if (select auth.uid()) is null then
    raise exception 'sign in to request a translation' using errcode = '42501';
  end if;

  perform 1 from public.novels n where n.id = p_novel_id and n.is_published;
  if not found then
    raise exception 'novel % not found', p_novel_id using errcode = 'P0002';
  end if;

  perform 1 from public.languages l where l.code = p_target_lang and l.enabled;
  if not found then
    raise exception 'language % is not available', p_target_lang using errcode = '22023';
  end if;

  insert into public.translation_jobs (chapter_id, target_lang, style, apply_glossary, requested_by)
  select c.id, p_target_lang, p_style, p_apply_glossary, (select auth.uid())
    from public.chapters c
   where c.novel_id = p_novel_id
     and c.number >= greatest(p_from_number, 1)
     and c.published_at is not null and c.published_at <= now()
     and not exists (
       select 1 from public.chapter_translations t
        where t.chapter_id = c.id and t.lang = p_target_lang and t.state = 'published'
     )
  on conflict (chapter_id, target_lang, style) where status in ('queued', 'running') do nothing;

  select count(*) into live
    from public.translation_jobs j join public.chapters c on c.id = j.chapter_id
   where c.novel_id = p_novel_id and c.number >= greatest(p_from_number, 1)
     and j.target_lang = p_target_lang and j.style = p_style
     and j.status in ('queued', 'running');
  return live;
end;
$$;

revoke execute on function public.request_novel_translation(bigint, text, integer, public.translation_style, boolean) from public, anon;
grant execute on function public.request_novel_translation(bigint, text, integer, public.translation_style, boolean) to authenticated;

-- Per content language other than the novel's original: how many released chapters read in
-- it, and how many wait on a job. Runs as the caller, so the usual policies apply (job
-- counts are 0 for a reader who is not signed in).
create or replace function public.novel_translation_progress(p_novel_id bigint)
returns table (lang text, total integer, translated integer, queued integer, running integer, failed integer)
language sql
stable
set search_path = ''
as $$
  with ch as (
    select c.id from public.chapters c
     where c.novel_id = p_novel_id and c.published_at is not null and c.published_at <= now()
  )
  select l.code,
         (select count(*) from ch)::integer,
         (select count(distinct t.chapter_id) from public.chapter_translations t
           where t.chapter_id in (select id from ch) and t.lang = l.code and t.state = 'published')::integer,
         (select count(distinct j.chapter_id) from public.translation_jobs j
           where j.chapter_id in (select id from ch) and j.target_lang = l.code and j.status = 'queued')::integer,
         (select count(distinct j.chapter_id) from public.translation_jobs j
           where j.chapter_id in (select id from ch) and j.target_lang = l.code and j.status = 'running')::integer,
         -- Failed with no newer live job for that chapter.
         (select count(distinct j.chapter_id) from public.translation_jobs j
           where j.chapter_id in (select id from ch) and j.target_lang = l.code and j.status = 'failed'
             and not exists (select 1 from public.translation_jobs k
                              where k.chapter_id = j.chapter_id and k.target_lang = l.code
                                and k.status in ('queued', 'running', 'succeeded')))::integer
    from public.languages l
   where l.enabled
     and l.code is distinct from (select n.original_lang from public.novels n where n.id = p_novel_id)
   order by l.sort_order;
$$;

grant execute on function public.novel_translation_progress(bigint) to anon, authenticated;
