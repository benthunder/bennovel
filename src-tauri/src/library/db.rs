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
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(done as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
