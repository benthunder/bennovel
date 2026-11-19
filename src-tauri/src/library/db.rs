//! The on-device library: one SQLite file per user, holding the imported books, their
//! chapters and translations, history, replace rules and the wiki.

use rusqlite::Connection;
use std::path::Path;

/// Bump when adding a migration below; `PRAGMA user_version` records what ran.
const MIGRATIONS: &[&str] = &[
    r#"
-- The novel catalog tables (once shaped like a server catalog, minus its
-- server-only parts: RLS, published flags, counters).
create table languages (
  code        text primary key,
  name        text not null,
  native_name text not null,
  enabled     integer not null default 1,
  sort_order  integer not null default 0
);
insert into languages (code, name, native_name, sort_order) values
  ('en', 'English', 'English', 1),
  ('vi', 'Vietnamese', 'Tiếng Việt', 2),
  ('zh', 'Chinese', '中文', 3),
  ('ko', 'Korean', '한국어', 4);

create table authors (
  id         integer primary key,
  slug       text not null unique,
  name       text not null,
  created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

create table novels (
  id             integer primary key,
  slug           text not null unique,
  author_id      integer not null references authors (id) on delete restrict,
  category_slug  text,
  original_lang  text not null references languages (code),
  status         text not null default 'completed' check (status in ('ongoing', 'completed')),
  published_year integer,
  cover_url      text,
  cover_palette  text,
  chapter_count  integer not null default 0,
  created_at     text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at     text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  -- Local only: where the book was imported from.
  source_format  text,
  source_name    text
);

create table novel_translations (
  novel_id    integer not null references novels (id) on delete cascade,
  lang        text not null references languages (code),
  title       text not null,
  description text not null default '',
  source      text not null default 'original' check (source in ('original', 'human', 'ai')),
  updated_at  text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  primary key (novel_id, lang)
);

create table chapters (
  id           integer primary key,
  novel_id     integer not null references novels (id) on delete cascade,
  number       integer not null check (number > 0),
  published_at text,
  created_at   text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at   text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  unique (novel_id, number)
);

create table chapter_translations (
  id         integer primary key,
  chapter_id integer not null references chapters (id) on delete cascade,
  lang       text not null references languages (code),
  style      text check (style in ('literal', 'natural', 'literary', 'casual')),
  source     text not null check (source in ('original', 'human', 'ai')),
  state      text not null default 'draft' check (state in ('draft', 'published')),
  title      text not null,
  -- Plain text, paragraphs separated by a blank line; JSON blocks since migration 18
  -- (blocks.rs).
  content    text not null,
  word_count integer not null default 0,
  model      text,
  created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  check ((source = 'ai') = (style is not null)),
  unique (chapter_id, lang, style)
);

create table glossary_entries (
  id         integer primary key,
  novel_id   integer not null references novels (id) on delete cascade,
  term_key   text not null,
  lang       text not null references languages (code),
  value      text not null,
  note       text,
  created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  unique (novel_id, term_key, lang)
);

-- No user_id: the whole file belongs to one user.
create table reading_history (
  novel_id       integer primary key references novels (id) on delete cascade,
  chapter_id     integer not null references chapters (id) on delete cascade,
  chapter_number integer not null check (chapter_number > 0),
  lang           text not null references languages (code),
  progress       real not null default 0 check (progress between 0 and 1),
  last_read_at   text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

create table replace_rules (
  id         integer primary key,
  novel_id   integer not null references novels (id) on delete cascade,
  lang       text not null references languages (code),
  find       text not null check (length(find) between 1 and 200),
  replace    text not null default '' check (length(replace) <= 200),
  position   integer not null default 0
);
"#,
    r#"
-- Local only: pictures of imported books, named by the image paragraphs
-- (U+FFFC + key) in chapter_translations.content.
create table chapter_images (
  novel_id integer not null references novels (id) on delete cascade,
  key      text not null,
  mime     text not null,
  data     blob not null,
  primary key (novel_id, key)
);
"#,
    r#"
-- Local only: the chapter number to show ("Chương 12"), from the file name for
-- folder imports or a "Chương 12: …" heading. `number` stays the reading order.
alter table chapters add column display_number real;
"#,
    r#"
-- Local only: online novels the reader saved or read, so favourites and
-- history show without a connection. Ids are server novel ids, a separate space
-- from the imported books in `novels`; `data` is the app's snapshot of the catalog
-- row (title, author, cover…) as JSON.
create table online_novels (
  id         integer primary key,
  data       text not null,
  updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Favourites and reading history for online novels. Times are
-- epoch milliseconds, as the app keeps them.
create table online_favorites (
  novel_id   integer primary key references online_novels (id) on delete cascade,
  position   integer not null
);

create table online_history (
  novel_id       integer primary key references online_novels (id) on delete cascade,
  chapter_number integer not null check (chapter_number > 0),
  last_read_at   integer not null
);
"#,
    r#"
-- Sync (see sync.rs): the library file is merged with another copy of it.
-- A book is the same on every device by its key, since ids are only local.
alter table novels add column book_key text;
update novels set book_key = lower(hex(randomblob(16)));
create unique index novels_book_key on novels (book_key);

-- When a favourite was added (epoch ms; 0 for those saved before sync existed).
alter table online_favorites add column added_at integer not null default 0;

-- Removals, so a merge doesn't bring back what was removed on another device.
-- `key` is the novel id (fav, history) or book_key (book); time in epoch ms.
create table tombstones (
  kind       text not null check (kind in ('fav', 'history', 'book')),
  key        text not null,
  deleted_at integer not null,
  primary key (kind, key)
);

-- Local only: sync bookkeeping (last synced version, reading_rev…).
create table sync_state (
  key   text primary key,
  value text not null
);
"#,
    r#"
-- Local only, never synced (see offline.rs): online novels downloaded to read
-- offline. Ids are server novel ids; `data` is the app's snapshot of the novel.
create table offline_novels (
  id            integer primary key,
  data          text not null,
  total         integer not null default 0,
  complete      integer not null default 0,
  started_at    integer not null,
  downloaded_at integer
);

-- One chapter in one language, like `chapter_translations`.
create table offline_chapters (
  novel_id   integer not null references offline_novels (id) on delete cascade,
  number     integer not null,
  lang       text not null,
  source     text not null,
  title      text not null,
  content    text not null,
  word_count integer not null default 0,
  saved_at   integer not null,
  primary key (novel_id, number, lang)
);

create table offline_glossary (
  novel_id integer not null references offline_novels (id) on delete cascade,
  term_key text not null,
  lang     text not null,
  value    text not null,
  primary key (novel_id, term_key, lang)
);
"#,
    r#"
-- The reader's replace rules (see rules.rs), one list per novel and language, synced.
-- `scope` 'online': `key` is a server novel id (no longer used); 'book': `key` is the imported
-- book's book_key. `rules` is a JSON array of {find, replace}; an emptied list stays
-- as '[]' so the removal reaches other devices. `updated_at` is epoch ms.
create table rule_lists (
  scope      text not null check (scope in ('online', 'book')),
  key        text not null,
  lang       text not null,
  rules      text not null,
  updated_at integer not null,
  primary key (scope, key, lang)
);
"#,
    r#"
-- Local only, never synced (see llm.rs): the user's own LLM, any OpenAI-compatible
-- chat API, for AI translation on this device. One row.
create table llm_config (
  id         integer primary key check (id = 1),
  provider   text not null,
  base_url   text not null,
  api_key    text not null,
  model      text not null,
  updated_at integer not null
);

-- Local only (see translate.rs): the language the LLM found in an imported book,
-- so it is asked once per book.
create table ai_books (
  novel_id    integer primary key references novels (id) on delete cascade,
  source_lang text not null
);

-- Local only (see queue.rs): chapters waiting for AI translation, one row per
-- chapter, target language and style. A finished chapter is marked by its
-- chapter_translations row (source 'ai'), which is never used as input again.
create table ai_jobs (
  id         integer primary key,
  novel_id   integer not null references novels (id) on delete cascade,
  chapter_id integer not null references chapters (id) on delete cascade,
  lang       text not null,
  style      text not null,
  glossary   integer not null default 1,
  status     text not null default 'queued' check (status in ('queued', 'running', 'done', 'failed')),
  error      text,
  created_at integer not null,
  unique (chapter_id, lang, style)
);
"#,
    r#"
-- The wiki of an imported book (see wiki.rs): characters, places, factions, items and
-- terms with the relations between them, filled by the AI as it translates and edited
-- by the user. Synced: `uid` is the same on every device, the newer `updated_at` (epoch
-- ms) wins and a removal is kept as `deleted = 1` so it reaches other devices.
create table wiki_entries (
  id            integer primary key,
  uid           text not null unique,
  novel_id      integer not null references novels (id) on delete cascade,
  kind          text not null check (kind in ('character', 'place', 'faction', 'item', 'term', 'other')),
  -- As written in the original text.
  name          text not null,
  -- JSON object: the name to use per language, {"vi": "Lâm Động"}.
  names         text not null default '{}',
  -- JSON array of other names in the original text.
  aliases       text not null default '[]',
  summary       text not null default '',
  first_chapter integer,
  source        text not null default 'ai' check (source in ('ai', 'user')),
  deleted       integer not null default 0,
  updated_at    integer not null,
  unique (novel_id, name)
);

create table wiki_relations (
  id            integer primary key,
  uid           text not null unique,
  novel_id      integer not null references novels (id) on delete cascade,
  from_uid      text not null references wiki_entries (uid) on delete cascade,
  to_uid        text not null references wiki_entries (uid) on delete cascade,
  -- Short, in the language it was written in ("sư phụ của").
  kind          text not null,
  note          text not null default '',
  first_chapter integer,
  deleted       integer not null default 0,
  updated_at    integer not null,
  unique (from_uid, to_uid, kind)
);
create index wiki_relations_novel on wiki_relations (novel_id);

-- Local only, never synced: each entry's embedding (float32, read by sqlite-vec) from
-- the embedding model set with the LLM, and the text it was made from.
create table wiki_vectors (
  entry_id  integer primary key references wiki_entries (id) on delete cascade,
  model     text not null,
  text      text not null,
  embedding blob not null
);

-- Local only: per book, whether translating updates the wiki, its last failure, and
-- the chapters already read into it.
create table wiki_books (
  novel_id   integer primary key references novels (id) on delete cascade,
  auto       integer not null default 1,
  last_error text
);
create table wiki_chapters (
  chapter_id integer primary key references chapters (id) on delete cascade,
  done_at    integer not null
);

alter table llm_config add column embed_model text not null default '';
"#,
    r#"
-- The wiki in every language (see wiki.rs). What an entry is and how entries link stays
-- the same in all languages; its words are kept per language and the reader's language
-- picks them, falling back to the raw text. Rebuilt to add the `level` kind (ranks,
-- cultivation realms), ordered by `rank`; `migrate` runs this with foreign keys off so
-- the relations and vectors keep pointing at the same rows.
create table wiki_entries_new (
  id            integer primary key,
  uid           text not null unique,
  novel_id      integer not null references novels (id) on delete cascade,
  kind          text not null check (kind in ('character', 'place', 'faction', 'item', 'level', 'term', 'other')),
  -- As written in the original text (the raw name).
  name          text not null,
  -- JSON object: the name to use per language, {"vi": "Lâm Động"}.
  names         text not null default '{}',
  -- JSON array of other names in the original text.
  aliases       text not null default '[]',
  -- Unused since the per-language `summaries`; kept for older files.
  summary       text not null default '',
  -- JSON object: the description per language; "raw" is in the book's own language.
  summaries     text not null default '{}',
  -- A level's place in its ladder, lowest first.
  rank          integer,
  first_chapter integer,
  source        text not null default 'ai' check (source in ('ai', 'user')),
  deleted       integer not null default 0,
  updated_at    integer not null,
  unique (novel_id, name)
);
-- A description was written in the language of the names the AI gave with it.
insert into wiki_entries_new (id, uid, novel_id, kind, name, names, aliases, summary, summaries,
    first_chapter, source, deleted, updated_at)
  select id, uid, novel_id, kind, name, names, aliases, '',
    case when summary = '' then '{}'
      else json_object(coalesce((select key from json_each(names) order by key = 'vi' desc limit 1), 'raw'), summary) end,
    first_chapter, source, deleted, updated_at
  from wiki_entries;
drop table wiki_entries;
alter table wiki_entries_new rename to wiki_entries;

-- `kind` is a relation type (wiki.rs REL_TYPES: master_of, member_of, located_in,
-- has_level…), shown in each language by the app; any other text is the user's or the
-- AI's own words, with `labels` holding them per language ({"vi": "nghĩa huynh"}).
alter table wiki_relations add column labels text not null default '{}';
"#,
    r#"
-- The description the AI wrote of an entry from each chapter, per language (see
-- wiki.rs `at_chapter`): the reader shows an entry as it was known by the chapter being
-- read, without what comes later. Synced; a chapter's note is only ever added again.
create table wiki_notes (
  entry_uid text not null references wiki_entries (uid) on delete cascade,
  lang      text not null,
  chapter   integer not null,
  summary   text not null,
  primary key (entry_uid, lang, chapter)
);
"#,
    r#"
-- The online-novel feature (browsing and reading novels from a server) and its
-- download-for-offline companion were removed: the tables they used are no longer
-- read or written (see offline.rs, reading.rs). Imported books, history, replace
-- rules and the wiki are untouched.
drop table online_history;
drop table online_favorites;
drop table online_novels;
drop table offline_glossary;
drop table offline_chapters;
drop table offline_novels;
"#,
    r#"
-- Local only: the wiki facts the translation requests found in a chapter (one JSON
-- array of the replies' JSON objects, see translate.rs WIKI_MARK), kept whether or not
-- the book's wiki learns, so turning it on later fills the wiki from them with no
-- request.
create table chapter_wiki_facts (
  chapter_id integer not null references chapters (id) on delete cascade,
  lang       text not null,
  facts      text not null,
  created_at integer not null,
  primary key (chapter_id, lang)
);

-- Local only (cast.rs): who each character is and how two characters address each
-- other in a translation, from the same facts; put back in later chapters' prompts so
-- genders, ages and forms of address stay the same.
create table cast_people (
  novel_id   integer not null references novels (id) on delete cascade,
  -- As written in the original text.
  name       text not null,
  gender     text not null default '',
  age        text not null default '',
  chapter    integer,
  updated_at integer not null,
  primary key (novel_id, name)
);
-- How `from_name` refers to themself when speaking to `to_name`, and calls them: as
-- the original text writes it (raw), and as each translation writes it.
create table cast_address_raw (
  novel_id   integer not null references novels (id) on delete cascade,
  from_name  text not null,
  to_name    text not null,
  self_term  text not null default '',
  other_term text not null default '',
  tone       text not null default '',
  chapter    integer,
  updated_at integer not null,
  primary key (novel_id, from_name, to_name)
);
create table cast_address (
  novel_id   integer not null references novels (id) on delete cascade,
  lang       text not null,
  from_name  text not null,
  to_name    text not null,
  self_term  text not null default '',
  other_term text not null default '',
  chapter    integer,
  updated_at integer not null,
  primary key (novel_id, lang, from_name, to_name)
);
"#,
    r#"
-- No more embeddings: the wiki is found by name (names.rs) from what the translations
-- keep in SQLite.
drop table wiki_vectors;
alter table llm_config drop column embed_model;
"#,
    r#"
-- Rebuilt to add the `skill` kind (techniques, martial arts, spells, abilities), linked
-- to who has them by the `has_skill` relation. Same columns and rows as before.
create table wiki_entries_new (
  id            integer primary key,
  uid           text not null unique,
  novel_id      integer not null references novels (id) on delete cascade,
  kind          text not null check (kind in ('character', 'place', 'faction', 'item', 'level', 'skill', 'term', 'other')),
  name          text not null,
  names         text not null default '{}',
  aliases       text not null default '[]',
  summary       text not null default '',
  summaries     text not null default '{}',
  rank          integer,
  first_chapter integer,
  source        text not null default 'ai' check (source in ('ai', 'user')),
  deleted       integer not null default 0,
  updated_at    integer not null,
  unique (novel_id, name)
);
insert into wiki_entries_new (id, uid, novel_id, kind, name, names, aliases, summary, summaries,
    rank, first_chapter, source, deleted, updated_at)
  select id, uid, novel_id, kind, name, names, aliases, summary, summaries,
    rank, first_chapter, source, deleted, updated_at
  from wiki_entries;
drop table wiki_entries;
alter table wiki_entries_new rename to wiki_entries;
"#,
    r#"
-- Local only (see queue.rs): rebuilt to add the `paused` status, a chapter the user
-- paused or one the LLM could not be used for (then `error` says why). Same rows.
create table ai_jobs_new (
  id         integer primary key,
  novel_id   integer not null references novels (id) on delete cascade,
  chapter_id integer not null references chapters (id) on delete cascade,
  lang       text not null,
  style      text not null,
  glossary   integer not null default 1,
  status     text not null default 'queued' check (status in ('queued', 'running', 'paused', 'done', 'failed')),
  error      text,
  created_at integer not null,
  unique (chapter_id, lang, style)
);
insert into ai_jobs_new select id, novel_id, chapter_id, lang, style, glossary, status, error, created_at
  from ai_jobs;
drop table ai_jobs;
alter table ai_jobs_new rename to ai_jobs;

-- Local only (see translate.rs): the LLM's reply to each part of a chapter as it comes
-- back, so a chapter stopped or failed half way picks up from the parts it has. `part`
-- is a hash of the part's text; the rows go once the chapter is saved.
create table ai_parts (
  chapter_id integer not null references chapters (id) on delete cascade,
  lang       text not null,
  style      text not null,
  part       text not null,
  reply      text not null,
  created_at integer not null,
  primary key (chapter_id, lang, style, part)
);
"#,
    r#"
-- First and last page (1-based) of a PDF/DjVu chapter, kept with the chapter so its
-- pages are the same on a device without the kept original (a restored backup) or
-- with an app that groups pages differently.
alter table chapters add column page_from integer;
alter table chapters add column page_to integer;
"#,
    r#"
-- `chapter_translations.content` becomes a JSON array of blocks and sentences
-- (blocks.rs): [{"type":"text","content":["…","…"]},{"type":"img","content":"img_0003"}].
-- The rows kept as plain text are rewritten by `blocks::convert_all` (see `migrate`).
select 1;
"#,
    r#"
-- A chapter's content, one row per block (segments.rs). The book's own text (raw) has
-- style '' and `position` is the block's place in the chapter, from 0; a translation
-- is other rows with its language and AI style, each at the position of the raw
-- segment it translates. `content`: a JSON array of sentences for text, the key in
-- chapter_images for img. `chapter_translations` keeps the title, state and model;
-- its `content` is moved here by `segments::migrate_all` (see `migrate`) and left ''.
create table segments (
  id         integer primary key,
  chapter_id integer not null references chapters (id) on delete cascade,
  lang       text not null,
  style      text not null default '',
  position   integer not null check (position >= 0),
  type       text not null check (type in ('text', 'img')),
  content    text not null,
  unique (chapter_id, lang, style, position)
);
"#,
    r#"
-- Local only (wiki.rs `index_names`): the vector table of a book's proper nouns. One
-- row per name as the original writes it (an entry's name or one of its aliases) and
-- target language: the name the translation uses and its kind. `embedding` (float32,
-- read by sqlite-vec) is made by the embedding model set with the LLM from the raw
-- name and the translated one; null while there is none. Names that land close
-- together are the same entity written two ways, and their entries are merged.
create table wiki_names (
  id         integer primary key,
  novel_id   integer not null references novels (id) on delete cascade,
  raw        text not null,
  lang       text not null,
  translated text not null default '',
  kind       text not null,
  entry_uid  text not null,
  model      text,
  embedding  blob,
  updated_at integer not null,
  unique (novel_id, raw, lang)
);
create index wiki_names_entry on wiki_names (entry_uid);
alter table llm_config add column embed_model text not null default '';

-- Local only (translate.rs): how a book is translated besides language and style: its
-- genre (forms of address follow it) and the user's own extra instructions for the
-- prompt.
create table ai_book_prefs (
  novel_id     integer primary key references novels (id) on delete cascade,
  genre        text not null default '',
  instructions text not null default ''
);
"#,
    r#"
-- What places a level in its ladder and a character on it (wiki.rs `Meta`), as JSON:
-- a level's ladder (`system`, beside its `rank`) and stage, a character's current level
-- and stage. Copied into the vector table so the prompt and the merge can use it.
alter table wiki_entries add column meta text not null default '{}';
alter table wiki_names add column meta text not null default '{}';
"#,
    r#"
-- The wiki is only Vietnamese now (wiki.rs `WIKI_LANG`): where a name, description or
-- relation already has its Vietnamese words, the other languages' go. Entries with
-- none yet keep theirs until the wiki screen has them translated (then `only_vi`).
update wiki_entries set summaries = json_object('vi', json_extract(summaries, '$.vi'))
 where json_valid(summaries) and json_extract(summaries, '$.vi') is not null
   and (select count(*) from json_each(wiki_entries.summaries)) > 1;
update wiki_entries set names = json_object('vi', json_extract(names, '$.vi'))
 where json_valid(names) and json_extract(names, '$.vi') is not null
   and (select count(*) from json_each(wiki_entries.names)) > 1;
update wiki_relations set labels = json_object('vi', json_extract(labels, '$.vi'))
 where json_valid(labels) and json_extract(labels, '$.vi') is not null
   and (select count(*) from json_each(wiki_relations.labels)) > 1;
"#,
    r#"
-- Local only (merge_queue.rs): the AI step of a book's smart merge as a queue, like
-- ai_jobs. The wiki is cut in blocks (`uids`, a JSON array of entry uids); each block's
-- answer (`groups`, JSON, smart_merge_ai.rs `Answered`) is kept, so a run cut short by
-- closing the app or a lost connection carries on from the blocks left.
create table wiki_merge_blocks (
  novel_id   integer not null references novels (id) on delete cascade,
  idx        integer not null,
  uids       text not null,
  status     text not null default 'queued'
             check (status in ('queued', 'running', 'done', 'failed', 'paused')),
  groups     text,
  error      text,
  created_at integer not null,
  primary key (novel_id, idx)
);
"#,
    r#"
-- The wiki drops entries that are only a chapter's heading ("Chapter 320" → "Chương 320")
-- or title ("Those Who Leave Must Return (2)"), and an entry whose original name is
-- Vietnamese in a book that is not takes its original from its other names. Older
-- versions let the AI write those (`wiki::tidy_all` runs after this one).
select 1;
"#,
    r#"
-- Smart merge suggestions the user set aside as already right (smart_merge.rs `skip`):
-- one row per set of entries (`uids`, sorted, joined by '+'), never suggested again.
create table wiki_merge_skips (
  novel_id   integer not null references novels (id) on delete cascade,
  uids       text not null,
  created_at integer not null,
  primary key (novel_id, uids)
);
"#,
];

/// The migration (1-based) after which `content` holds blocks.
const BLOCKS_VERSION: usize = 18;
/// The migration (1-based) that moves `content` into table `segments`.
const SEGMENTS_VERSION: usize = 19;
/// The migration (1-based) after which the wiki only keeps proper nouns: common nouns
/// and chapter headings older versions let in are cleaned out by `wiki::tidy_all`.
const TIDY_VERSION: usize = 24;

/// Epoch milliseconds now.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Schema version this app writes (`PRAGMA user_version` after migrating).
pub fn schema_version() -> i64 {
    MIGRATIONS.len() as i64
}

pub fn get_state(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row("select value from sync_state where key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

pub fn set_state(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "insert into sync_state (key, value) values (?1, ?2)
         on conflict (key) do update set value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Builds sqlite-vec (`vec_distance_cosine`…) into every connection opened from here
/// on; it is compiled into the app, so it works on Android too.
fn load_vec() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    load_vec();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut conn = Connection::open(path)?;
    {
        // Turning a new file to WAL and migrating it, one connection at a time.
        let _one = MIGRATING.lock().unwrap_or_else(|e| e.into_inner());
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn)?;
    }
    Ok(conn)
}

/// One migration at a time in this process: the app opens the library file from many
/// tasks at once (the queues, the commands), and two of them migrating together left
/// some libraries with tables a schema version below what they have (`effective`).
static MIGRATING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let version = |c: &Connection| -> rusqlite::Result<usize> {
        Ok(c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize)
    };
    let mut done = version(conn)?;
    if done >= MIGRATIONS.len() {
        return Ok(());
    }
    // A library whose version went back below the tables it has (two connections
    // migrating at once, in an older app) goes on from what it really has.
    let real = effective(conn)?;
    if real > done {
        conn.pragma_update(None, "user_version", real as i64)?;
        done = real;
    }
    // Some migrations rebuild a table; with foreign keys on, dropping the old one would
    // delete the rows that point at it.
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| {
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(done) {
            // Taken before anything is read, so another process can't migrate meanwhile;
            // a migration already made by one is skipped.
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if version(&tx)? > i {
                continue;
            }
            tx.execute_batch(sql)?;
            if i + 1 == BLOCKS_VERSION {
                super::blocks::convert_all(&tx)?;
            }
            if i + 1 == SEGMENTS_VERSION {
                super::segments::migrate_all(&tx)?;
            }
            // Run on the latest schema, which the wiki's code reads.
            if i + 1 == MIGRATIONS.len() && done < TIDY_VERSION {
                super::wiki::tidy_all(&tx)?;
            }
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(())
    })();
    conn.pragma_update(None, "foreign_keys", true)?;
    result
}

/// What a migration leaves that can be seen in the schema: the tables and indexes it
/// creates and the columns it adds ((table, Some(column)) for a column). A table built
/// to replace another (`…_new`, renamed after) is not one.
fn effects(sql: &str) -> Vec<(String, Option<String>)> {
    let text: String = sql
        .lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || "(),;".contains(c))
        .filter(|w| !w.is_empty())
        .collect();
    let name = |at: usize| -> Option<(String, usize)> {
        let mut k = at;
        if words.get(k) == Some(&"if") {
            k += 3;
        }
        words.get(k).map(|w| (w.trim_matches('"').to_string(), k))
    };
    let mut out = Vec::new();
    for i in 0..words.len() {
        match (words[i], words.get(i + 1).copied()) {
            ("create", Some("table")) => {
                if let Some((t, _)) = name(i + 2) {
                    if !t.ends_with("_new") {
                        out.push((t, None));
                    }
                }
            }
            ("create", Some("virtual")) => {
                if let Some((t, _)) = name(i + 3) {
                    out.push((t, None));
                }
            }
            ("create", Some("index")) => {
                if let Some((t, _)) = name(i + 2) {
                    out.push((t, None));
                }
            }
            ("create", Some("unique")) if words.get(i + 2) == Some(&"index") => {
                if let Some((t, _)) = name(i + 3) {
                    out.push((t, None));
                }
            }
            ("alter", Some("table")) => {
                let Some((t, k)) = name(i + 2) else { continue };
                if words.get(k + 1) != Some(&"add") {
                    continue;
                }
                let c = if words.get(k + 2) == Some(&"column") {
                    k + 3
                } else {
                    k + 2
                };
                if let Some(c) = words.get(c) {
                    if !t.ends_with("_new") {
                        out.push((t, Some(c.trim_matches('"').to_string())));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The schema version a library really has: the last migration all of whose tables,
/// indexes and columns are there (0 for a new file).
fn effective(conn: &Connection) -> rusqlite::Result<usize> {
    let has = |(t, c): &(String, Option<String>)| -> rusqlite::Result<bool> {
        Ok(match c {
            None => {
                conn.query_row(
                    "select count(*) from sqlite_master where name = ?1",
                    [t],
                    |r| r.get::<_, i64>(0),
                )? > 0
            }
            Some(c) => {
                conn.query_row(
                    "select count(*) from pragma_table_info(?1) where name = ?2",
                    [t, c],
                    |r| r.get::<_, i64>(0),
                )? > 0
            }
        })
    };
    let mut real = 0;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let seen = effects(sql);
        if seen.is_empty() {
            continue;
        }
        let mut all = true;
        for e in &seen {
            if !has(e)? {
                all = false;
                break;
            }
        }
        if all {
            real = i + 1;
        }
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_plain_text_becomes_blocks() {
        assert!(MIGRATIONS[BLOCKS_VERSION - 1].contains("blocks::convert_all"));
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..BLOCKS_VERSION - 1] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", BLOCKS_VERSION as i64 - 1)
            .unwrap();
        conn.execute_batch(
            "insert into authors (id, slug, name) values (1, 'a', 'A');
             insert into novels (id, slug, author_id, original_lang, book_key) values (1, 'n', 1, 'vi', 'k');
             insert into chapters (id, novel_id, number) values (1, 1, 1);
             insert into chapter_translations (chapter_id, lang, source, title, content)
               values (1, 'vi', 'original', 'C1', 'Câu một. Câu hai.' || char(10, 10, 65532) || 'img_1');",
        )
        .unwrap();
        // An older app's AI text, half way: its second block was still to come.
        conn.execute(
            "insert into chapter_translations (chapter_id, lang, style, source, state, title, content)
             values (1, 'en', 'natural', 'ai', 'draft', 'C1', ?1)",
            [r#"[null,{"type":"img","content":"img_1"}]"#],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert!(MIGRATIONS[SEGMENTS_VERSION - 1].contains("create table segments"));
        let rows: Vec<(String, String, i64, String, String)> = conn
            .prepare("select lang, style, position, type, content from segments order by lang desc, position")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "vi".into(),
                    "".into(),
                    0,
                    "text".into(),
                    r#"["Câu một.","Câu hai."]"#.into()
                ),
                ("vi".into(), "".into(), 1, "img".into(), "img_1".into()),
                (
                    "en".into(),
                    "natural".into(),
                    1,
                    "img".into(),
                    "img_1".into()
                ),
            ]
        );
        let left: i64 = conn
            .query_row(
                "select count(*) from chapter_translations where content <> ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn the_multilingual_wiki_keeps_older_entries_and_relations() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        for sql in &MIGRATIONS[..9] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 9).unwrap();
        conn.execute_batch(
            "insert into authors (id, slug, name) values (1, 'a', 'A');
             insert into novels (id, slug, author_id, original_lang, book_key) values (1, 'n', 1, 'zh', 'k');
             insert into wiki_entries (id, uid, novel_id, kind, name, names, summary, updated_at) values
               (1, 'a', 1, 'character', '林动', '{\"vi\":\"Lâm Động\"}', 'Nhân vật chính.', 1),
               (2, 'b', 1, 'place', '炎城', '{}', 'A city.', 1),
               (3, 'c', 1, 'term', '元力', '{}', '', 1);
             insert into wiki_relations (uid, novel_id, from_uid, to_uid, kind, updated_at)
               values ('r', 1, 'a', 'b', 'sống ở', 1);
             insert into wiki_vectors (entry_id, model, text, embedding) values (1, 'm', 't', x'00');",
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        let rows: Vec<(String, String)> = conn
            .prepare("select uid, summaries from wiki_entries order by id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows[0], ("a".into(), r#"{"vi":"Nhân vật chính."}"#.into()));
        assert_eq!(rows[1], ("b".into(), r#"{"raw":"A city."}"#.into()));
        assert_eq!(rows[2], ("c".into(), "{}".into()));
        let kept: (i64, String) = conn
            .query_row(
                "select (select count(*) from wiki_relations), (select labels from wiki_relations)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kept, (1, "{}".into()));
        conn.execute(
            "insert into wiki_entries (uid, novel_id, kind, name, rank, updated_at) values ('d', 1, 'level', '斗师', 2, 1)",
            [],
        )
        .unwrap();
        let fk: bool = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(fk);
    }

    #[test]
    fn opening_a_new_library_from_many_tasks_at_once_migrates_it_once() {
        for round in 0..4 {
            let dir = std::env::temp_dir().join(format!("bennovel-race-{round}-{}", now_ms()));
            let path = dir.join("lib.sqlite3");
            let errors: Vec<String> = std::thread::scope(|s| {
                let all: Vec<_> = (0..6)
                    .map(|_| s.spawn(|| open(&path).err().map(|e| e.to_string())))
                    .collect();
                all.into_iter().filter_map(|h| h.join().unwrap()).collect()
            });
            assert!(errors.is_empty(), "{errors:?}");
            let conn = open(&path).unwrap();
            let v: i64 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(v, schema_version());
        }
    }

    #[test]
    fn a_library_whose_version_went_back_goes_on_from_the_tables_it_has() {
        let dir = std::env::temp_dir().join(format!("bennovel-back-{}", now_ms()));
        let path = dir.join("lib.sqlite3");
        let conn = open(&path).unwrap();
        conn.execute_batch(
            "insert into authors (id, slug, name) values (1, 'a', 'A');
             insert into novels (id, slug, author_id, original_lang, book_key) values (1, 'n', 1, 'en', 'k');",
        )
        .unwrap();
        // As the race in older apps left it: the tables of every migration, the version of the 6th.
        conn.pragma_update(None, "user_version", 6).unwrap();
        drop(conn);
        let conn = open(&path).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, schema_version());
        let books: i64 = conn
            .query_row("select count(*) from novels", [], |r| r.get(0))
            .unwrap();
        assert_eq!(books, 1);
        // Only the last migration missing: it alone runs.
        conn.execute_batch("drop table wiki_merge_skips").unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        drop(conn);
        let conn = open(&path).unwrap();
        let n: i64 = conn
            .query_row(
                "select count(*) from sqlite_master where name = 'wiki_merge_skips'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn every_migration_with_a_table_or_column_is_seen() {
        let e = effects(MIGRATIONS[22]);
        assert_eq!(e, vec![("wiki_merge_blocks".to_string(), None)]);
        let seen = MIGRATIONS.iter().filter(|m| !effects(m).is_empty()).count();
        assert!(seen >= 15, "{seen}");
    }

    #[test]
    fn the_last_wiki_cleanup_is_the_last_migration() {
        assert_eq!(TIDY_VERSION, 24);
        assert!(MIGRATIONS[TIDY_VERSION - 1].contains("tidy_all"));
    }
}
