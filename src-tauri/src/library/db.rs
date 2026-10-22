//! The on-device library: one SQLite file per user, with the novel tables shaped
//! like the Supabase catalog (`supabase/migrations/*_catalog.sql`) so imported books
//! can later be synced or uploaded as they are.

use rusqlite::Connection;
use std::path::Path;

/// Bump when adding a migration below; `PRAGMA user_version` records what ran.
const MIGRATIONS: &[&str] = &[
    r#"
-- Same tables and columns as Supabase (Postgres types mapped to SQLite ones),
-- minus the server-only parts (RLS, published flags for the catalog, counters).
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
  -- Plain text, paragraphs separated by a blank line (as in Supabase).
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

-- Supabase keys these by user_id; here the whole file belongs to one user.
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
-- Local only: online (Supabase) novels the reader saved or read, so favourites and
-- history show without a connection. Ids are Supabase novel ids, a separate space
-- from the imported books in `novels`; `data` is the app's snapshot of the catalog
-- row (title, author, cover…) as JSON.
create table online_novels (
  id         integer primary key,
  data       text not null,
  updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Like Supabase `favorites` and `reading_history`, for online novels. Times are
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
-- Sync (see sync.rs): the library file is merged with the copy in Supabase Storage.
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
-- offline. Ids are Supabase novel ids; `data` is the app's snapshot of the novel.
create table offline_novels (
  id            integer primary key,
  data          text not null,
  total         integer not null default 0,
  complete      integer not null default 0,
  started_at    integer not null,
  downloaded_at integer
);

-- One chapter in one language, as in Supabase `chapter_translations`.
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
-- `scope` 'online': `key` is the Supabase novel id; 'book': `key` is the imported
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
];

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

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let done: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if done as usize >= MIGRATIONS.len() {
        return Ok(());
    }
    // Some migrations rebuild a table; with foreign keys on, dropping the old one would
    // delete the rows that point at it.
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| {
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(done as usize) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(())
    })();
    conn.pragma_update(None, "foreign_keys", true)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
