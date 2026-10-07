-- 1. Chapters are offered in four languages only: English, Vietnamese, Chinese and Korean.
-- 2. Readers keep their own find → replace rules per novel and language, applied
--    to the chapter text when it is shown.

-- ---------------------------------------------------------------------------
-- Content languages
-- ---------------------------------------------------------------------------

-- Rows in other languages (the Spanish sample text) are copied into a private
-- `legacy_20261007` schema (not exposed through the API) before they are removed,
-- so nothing is lost. Drop that schema by hand once nobody needs it.
create schema if not exists legacy_20261007;
revoke all on schema legacy_20261007 from public, anon, authenticated;

create table legacy_20261007.languages as
  select * from public.languages where code not in ('en', 'vi', 'zh', 'ko');
create table legacy_20261007.chapter_translations as
  select * from public.chapter_translations where lang not in ('en', 'vi', 'zh', 'ko');
create table legacy_20261007.glossary_entries as
  select * from public.glossary_entries where lang not in ('en', 'vi', 'zh', 'ko');
create table legacy_20261007.reading_history as
  select * from public.reading_history where lang not in ('en', 'vi', 'zh', 'ko');
create table legacy_20261007.translation_jobs as
  select * from public.translation_jobs
  where target_lang not in ('en', 'vi', 'zh', 'ko') or source_lang not in ('en', 'vi', 'zh', 'ko');

delete from public.chapter_translations where lang not in ('en', 'vi', 'zh', 'ko');
delete from public.glossary_entries where lang not in ('en', 'vi', 'zh', 'ko');
delete from public.reading_history where lang not in ('en', 'vi', 'zh', 'ko');
delete from public.translation_jobs
  where target_lang not in ('en', 'vi', 'zh', 'ko') or source_lang not in ('en', 'vi', 'zh', 'ko');
update public.profiles set default_content_lang = null
  where default_content_lang not in ('en', 'vi', 'zh', 'ko');
delete from public.languages where code not in ('en', 'vi', 'zh', 'ko');

insert into public.languages (code, name, native_name, sort_order) values
  ('en', 'English', 'English', 0),
  ('vi', 'Vietnamese', 'Tiếng Việt', 1),
  ('zh', 'Chinese', '中文', 2),
  ('ko', 'Korean', '한국어', 3)
on conflict (code) do update
  set name = excluded.name, native_name = excluded.native_name, sort_order = excluded.sort_order;

-- Every table with a language column references public.languages, so this one
-- check limits chapters, glossaries, history and translation jobs alike.
alter table public.languages
  add constraint languages_supported check (code in ('en', 'vi', 'zh', 'ko'));

-- ---------------------------------------------------------------------------
-- Replace rules: "find → replace" a reader applies to one novel in one language,
-- e.g. to fix a character name the way they prefer it. Applied in order of position.
-- ---------------------------------------------------------------------------

create table public.replace_rules (
  id         bigint generated always as identity primary key,
  user_id    uuid not null default auth.uid() references auth.users (id) on delete cascade,
  novel_id   bigint not null references public.novels (id) on delete cascade,
  lang       text not null references public.languages (code),
  find       text not null check (char_length(find) between 1 and 200),
  replace    text not null default '' check (char_length(replace) <= 200),
  position   smallint not null default 0,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (user_id, novel_id, lang, find)
);

create index replace_rules_novel_id_idx on public.replace_rules (novel_id);
create index replace_rules_lang_idx on public.replace_rules (lang);

create trigger replace_rules_set_updated_at
before update on public.replace_rules
for each row execute function public.set_updated_at();

alter table public.replace_rules enable row level security;

create policy "users read own replace rules" on public.replace_rules
  for select to authenticated using (user_id = (select auth.uid()));

create policy "users add own replace rules" on public.replace_rules
  for insert to authenticated with check (user_id = (select auth.uid()));

create policy "users update own replace rules" on public.replace_rules
  for update to authenticated
  using (user_id = (select auth.uid()))
  with check (user_id = (select auth.uid()));

create policy "users delete own replace rules" on public.replace_rules
  for delete to authenticated using (user_id = (select auth.uid()));
