//! The on-device library: one SQLite file per user, with the novel tables shaped
//! like the Supabase catalog (`supabase/migrations/*_catalog.sql`) so imported books
//! can later be synced or uploaded as they are.

use rusqlite::Connection;
use std::path::Path;

/// Bump when adding a migration below; `PRAGMA user_version` records what ran.
const MIGRATIONS: &[&str] = &[r#"
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
"#];

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
