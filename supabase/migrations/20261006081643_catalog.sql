-- Catalog: everything a reader browses (novels, chapters and their translations).
-- Readable by everyone (anon + authenticated) once published. Writes go through
-- the service role (admin tools, import scripts, the AI translation worker),
-- which bypasses RLS, so no write policies are defined here on purpose.

-- ---------------------------------------------------------------------------
-- Shared helpers
-- ---------------------------------------------------------------------------

create or replace function public.set_updated_at()
returns trigger
language plpgsql
set search_path = ''
as $$
begin
  new.updated_at := now();
  return new;
end;
$$;

-- ---------------------------------------------------------------------------
-- Enums
-- ---------------------------------------------------------------------------

create type public.novel_status as enum ('ongoing', 'completed');

-- Matches TRANSLATE_STYLES in the app.
create type public.translation_style as enum ('literal', 'natural', 'literary', 'casual');

-- Where a chapter text came from: the author's original, a human translator, or the AI pipeline.
create type public.translation_source as enum ('original', 'human', 'ai');

create type public.publish_state as enum ('draft', 'published');

-- ---------------------------------------------------------------------------
-- Lookup tables
-- ---------------------------------------------------------------------------

-- Content languages. A table rather than an enum so a new language is a row, not a migration.
create table public.languages (
  code        text primary key check (code ~ '^[a-z]{2,3}(-[A-Z]{2})?$'),
  name        text not null,
  native_name text not null,
  enabled     boolean not null default true,
  sort_order  smallint not null default 0
);

create table public.categories (
  slug       text primary key check (slug ~ '^[a-z0-9-]+$'),
  name       text not null,
  sort_order smallint not null default 0
);

create table public.authors (
  id         bigint generated always as identity primary key,
  slug       text not null unique check (slug ~ '^[a-z0-9-]+$'),
  name       text not null,
  created_at timestamptz not null default now()
);

-- ---------------------------------------------------------------------------
-- Novels
-- ---------------------------------------------------------------------------

create table public.novels (
  id             bigint generated always as identity primary key,
  slug           text not null unique check (slug ~ '^[a-z0-9-]+$'),
  author_id      bigint not null references public.authors (id) on delete restrict,
  category_slug  text not null references public.categories (slug) on update cascade on delete restrict,
  original_lang  text not null references public.languages (code),
  status         public.novel_status not null default 'ongoing',
  published_year smallint,
  cover_url      text,
  -- Placeholder palette ({bg, fg, deco}) used until real cover art exists.
  cover_palette  jsonb,
  rating_avg     numeric(2, 1) not null default 0 check (rating_avg between 0 and 5),
  rating_count   integer not null default 0 check (rating_count >= 0),
  read_count     bigint not null default 0 check (read_count >= 0),
  -- Kept in sync by a trigger on chapters.
  chapter_count  integer not null default 0 check (chapter_count >= 0),
  is_published   boolean not null default false,
  created_at     timestamptz not null default now(),
  updated_at     timestamptz not null default now()
);

create index novels_author_id_idx on public.novels (author_id);
create index novels_category_slug_idx on public.novels (category_slug);
create index novels_read_count_idx on public.novels (read_count desc) where is_published;

create trigger novels_set_updated_at
before update on public.novels
for each row execute function public.set_updated_at();

-- Title and blurb per language. The original language row should always exist.
create table public.novel_translations (
  novel_id    bigint not null references public.novels (id) on delete cascade,
  lang        text not null references public.languages (code),
  title       text not null,
  description text not null default '',
  source      public.translation_source not null default 'original',
  updated_at  timestamptz not null default now(),
  primary key (novel_id, lang)
);

create index novel_translations_lang_idx on public.novel_translations (lang);

create trigger novel_translations_set_updated_at
before update on public.novel_translations
for each row execute function public.set_updated_at();

-- ---------------------------------------------------------------------------
-- Collections (home shelves: trending, new, completed, editor's picks)
-- ---------------------------------------------------------------------------

create table public.collections (
  key        text primary key check (key ~ '^[a-z0-9-]+$'),
  name       text not null,
  sort_order smallint not null default 0
);

create table public.collection_items (
  collection_key text not null references public.collections (key) on update cascade on delete cascade,
  novel_id       bigint not null references public.novels (id) on delete cascade,
  position       smallint not null default 0,
  primary key (collection_key, novel_id)
);

create index collection_items_novel_id_idx on public.collection_items (novel_id);

-- ---------------------------------------------------------------------------
-- Chapters
-- ---------------------------------------------------------------------------

create table public.chapters (
  id           bigint generated always as identity primary key,
  novel_id     bigint not null references public.novels (id) on delete cascade,
  number       integer not null check (number > 0),
  -- null = not released yet. Scheduling a release is setting a future timestamp.
  published_at timestamptz,
  created_at   timestamptz not null default now(),
  updated_at   timestamptz not null default now(),
  unique (novel_id, number)
);

create trigger chapters_set_updated_at
before update on public.chapters
for each row execute function public.set_updated_at();

create or replace function public.sync_novel_chapter_count()
returns trigger
language plpgsql
set search_path = ''
as $$
declare
  target bigint := coalesce(new.novel_id, old.novel_id);
begin
  update public.novels n
     set chapter_count = (select count(*) from public.chapters c where c.novel_id = target)
   where n.id = target;
  -- A chapter moved to another novel: fix the old one too.
  if tg_op = 'UPDATE' and old.novel_id <> new.novel_id then
    update public.novels n
       set chapter_count = (select count(*) from public.chapters c where c.novel_id = old.novel_id)
     where n.id = old.novel_id;
  end if;
  return null;
end;
$$;

create trigger chapters_sync_count
after insert or delete or update of novel_id on public.chapters
for each row execute function public.sync_novel_chapter_count();

-- The readable text of a chapter in one language. A chapter can have several AI
-- versions of the same language, one per style; original/human rows have no style.
create table public.chapter_translations (
  id         bigint generated always as identity primary key,
  chapter_id bigint not null references public.chapters (id) on delete cascade,
  lang       text not null references public.languages (code),
  style      public.translation_style,
  source     public.translation_source not null,
  state      public.publish_state not null default 'draft',
  title      text not null,
  -- Plain text, paragraphs separated by a blank line. [[key]] marks a glossary term.
  content    text not null,
  word_count integer generated always as (
    coalesce(array_length(regexp_split_to_array(btrim(content), '\s+'), 1), 0)
  ) stored,
  -- For AI output: which model produced it, for auditing and re-runs.
  model      text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint chapter_translations_style_matches_source
    check ((source = 'ai') = (style is not null)),
  constraint chapter_translations_unique_version
    unique nulls not distinct (chapter_id, lang, style)
);

create index chapter_translations_lang_idx on public.chapter_translations (lang);

create trigger chapter_translations_set_updated_at
before update on public.chapter_translations
for each row execute function public.set_updated_at();

-- ---------------------------------------------------------------------------
-- Glossary: per-novel names and terms, fed to the AI translator so a name is
-- rendered the same way in every chapter ("Apply dictionary" in the reader).
-- ---------------------------------------------------------------------------

create table public.glossary_entries (
  id         bigint generated always as identity primary key,
  novel_id   bigint not null references public.novels (id) on delete cascade,
  term_key   text not null check (term_key ~ '^[a-z0-9_-]+$'),
  lang       text not null references public.languages (code),
  value      text not null,
  note       text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (novel_id, term_key, lang)
);

create index glossary_entries_lang_idx on public.glossary_entries (lang);

create trigger glossary_entries_set_updated_at
before update on public.glossary_entries
for each row execute function public.set_updated_at();

-- ---------------------------------------------------------------------------
-- Row level security: public read of published content only.
-- ---------------------------------------------------------------------------

alter table public.languages            enable row level security;
alter table public.categories           enable row level security;
alter table public.authors              enable row level security;
alter table public.novels               enable row level security;
alter table public.novel_translations   enable row level security;
alter table public.collections          enable row level security;
alter table public.collection_items     enable row level security;
alter table public.chapters             enable row level security;
alter table public.chapter_translations enable row level security;
alter table public.glossary_entries     enable row level security;

create policy "languages are public" on public.languages
  for select to anon, authenticated using (enabled);

create policy "categories are public" on public.categories
  for select to anon, authenticated using (true);

create policy "authors are public" on public.authors
  for select to anon, authenticated using (true);

create policy "published novels are public" on public.novels
  for select to anon, authenticated using (is_published);

create policy "translations of published novels are public" on public.novel_translations
  for select to anon, authenticated
  using (exists (select 1 from public.novels n where n.id = novel_id and n.is_published));

create policy "collections are public" on public.collections
  for select to anon, authenticated using (true);

create policy "collection items of published novels are public" on public.collection_items
  for select to anon, authenticated
  using (exists (select 1 from public.novels n where n.id = novel_id and n.is_published));

create policy "released chapters are public" on public.chapters
  for select to anon, authenticated
  using (
    published_at is not null
    and published_at <= now()
    and exists (select 1 from public.novels n where n.id = novel_id and n.is_published)
  );

-- chapters' own policy already hides unreleased chapters and unpublished novels.
create policy "published chapter texts are public" on public.chapter_translations
  for select to anon, authenticated
  using (state = 'published' and exists (select 1 from public.chapters c where c.id = chapter_id));

create policy "glossary of published novels is public" on public.glossary_entries
  for select to anon, authenticated
  using (exists (select 1 from public.novels n where n.id = novel_id and n.is_published));
