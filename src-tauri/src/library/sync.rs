//! Backups of the library: the library file is exported as a gzipped snapshot, and a
//! backup brought back is merged into it row by row, so nothing made since is lost
//! (the file goes where the user picks). The merge rules:
//! - history and progress: the most recent read wins;
//! - imported books: matched by `book_key`; a book missing here is copied whole
//!   (chapters, pictures…), newer details (title, author) win, a deleted book stays deleted;
//! - replace rules: per novel and language, the list changed last wins.
//!
//! A backup also carries the kept PDF/DjVu originals (table `book_files` of the backup
//! copy), so a restored book shows its pages, grouped into the same chapters.

use super::db;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("{0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The synced file was written by a newer version of the app.
    #[error("sync-outdated")]
    Outdated,
}

/// Where a backup keeps the app's own settings (language, reader settings…), as JSON.
const APP_SETTINGS: &str = "app_settings";

/// A consistent copy of the library file (the live one is in WAL mode), gzipped into
/// `out`; returns its size. `tmp` is a scratch path that must not exist.
#[cfg(test)]
pub fn export_to(conn: &Connection, tmp: &Path, out: &Path) -> Result<u64, SyncError> {
    snapshot(conn, tmp, out, None)
}

/// What a backup holds besides the library: the app's settings (JSON) and the folder
/// of kept originals.
struct Extras<'a> {
    settings: Option<&'a str>,
    originals: Option<&'a Path>,
}

/// Originals bigger than this stay out of a backup (SQLite's limit for one value).
const MAX_ORIGINAL: u64 = 1_000_000_000;

/// A backup: everything in the library (books, history, favourites, replace rules,
/// glossaries, downloaded novels) with the LLM settings, API key included, and the
/// app's `settings` (JSON), and the kept originals in `originals` (PDF/DjVu files
/// named by novel id). Only the AI queue is left out.
pub fn backup_to(
    conn: &Connection,
    tmp: &Path,
    out: &Path,
    settings: Option<&str>,
    originals: Option<&Path>,
) -> Result<u64, SyncError> {
    snapshot(conn, tmp, out, Some(Extras { settings, originals }))
}

/// `backup`: `Some` for a backup, `None` for a synced copy.
fn snapshot(
    conn: &Connection,
    tmp: &Path,
    out: &Path,
    backup: Option<Extras>,
) -> Result<u64, SyncError> {
    let _ = std::fs::remove_file(tmp);
    conn.execute("vacuum into ?1", [tmp.to_string_lossy()])?;
    let result = (|| {
        let copy = Connection::open(tmp)?;
        match backup {
            Some(extras) => {
                // The queue's jobs and the languages found point at this library's ids.
                copy.execute_batch("delete from ai_jobs; delete from ai_books;")?;
                copy.execute("delete from sync_state where key = ?1", [APP_SETTINGS])?;
                if let Some(settings) = extras.settings {
                    db::set_state(&copy, APP_SETTINGS, settings)?;
                }
                if let Some(dir) = extras.originals {
                    pack_originals(&copy, dir)?;
                }
            }
            None => {
                // The LLM settings (with their API key) and the AI queue stay on this device.
                super::translate::strip(&copy)?;
            }
        }
        // Vectors and the wiki's per-book state point at this library's ids.
        super::wiki::strip(&copy)?;
        copy.execute("vacuum", [])?;
        drop(copy);
        let file = std::io::BufWriter::new(std::fs::File::create(out)?);
        let mut enc = GzEncoder::new(file, Compression::default());
        std::io::copy(&mut std::fs::File::open(tmp)?, &mut enc)?;
        enc.finish()?
            .into_inner()
            .map_err(|e| e.into_error())?
            .sync_all()?;
        Ok::<_, SyncError>(std::fs::metadata(out)?.len())
    })();
    let _ = std::fs::remove_file(tmp);
    result
}

/// Copies the kept originals of the books in `copy` into its `book_files` table,
/// streamed (a PDF can be hundreds of MB).
fn pack_originals(copy: &Connection, dir: &Path) -> Result<(), SyncError> {
    copy.execute_batch(
        "create table book_files (novel_id integer primary key, ext text not null, data blob not null)",
    )?;
    let ids: Vec<i64> = copy
        .prepare("select id from novels")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in ids {
        let Some(path) = super::original_in(dir, id) else {
            continue;
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("pdf");
        let mut file = std::fs::File::open(&path)?;
        let len = file.metadata()?.len();
        if len > MAX_ORIGINAL {
            continue;
        }
        copy.execute(
            "insert into book_files (novel_id, ext, data) values (?1, ?2, zeroblob(?3))",
            params![id, ext, len as i64],
        )?;
        let mut blob = copy.blob_open("main", "book_files", "data", id, false)?;
        std::io::copy(&mut file, &mut blob)?;
    }
    Ok(())
}

/// After a restore: writes the originals the backup carries into `dir` for the books
/// that came with it (replacing a stale file left under a reused id) and for books
/// here that lost theirs. `before` holds the ids of the books here before the merge.
fn unpack_originals(
    conn: &Connection,
    dir: &Path,
    before: &HashSet<i64>,
) -> Result<(), SyncError> {
    let new: Vec<i64> = conn
        .prepare("select id from main.novels")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    for id in &new {
        while let Some(stale) = super::original_in(dir, *id) {
            std::fs::remove_file(stale)?;
        }
    }
    let packed: bool = conn.query_row(
        "select exists (select 1 from r.sqlite_master where type = 'table' and name = 'book_files')",
        [],
        |r| r.get(0),
    )?;
    if !packed {
        return Ok(());
    }
    let files: Vec<(i64, String, i64)> = conn
        .prepare(
            "select f.novel_id, f.ext, m.id from r.book_files f
             join r.novels n on n.id = f.novel_id join main.novels m on m.book_key = n.book_key",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (rid, ext, lid) in files {
        if !super::ORIGINAL_EXTS.contains(&ext.as_str())
            || (before.contains(&lid) && super::original_in(dir, lid).is_some())
        {
            continue;
        }
        std::fs::create_dir_all(dir)?;
        let part = dir.join(format!("restoring-{lid}.{ext}"));
        let written = (|| {
            let mut blob =
                conn.blob_open("r", "book_files", "data", rid, true)?;
            let mut file = std::fs::File::create(&part)?;
            std::io::copy(&mut blob, &mut file)?;
            file.sync_all()?;
            std::fs::rename(&part, dir.join(format!("{lid}.{ext}")))?;
            Ok::<_, SyncError>(())
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        written?;
    }
    Ok(())
}

/// `export_to`, in memory.
#[cfg(test)]
pub fn export(conn: &Connection, tmp: &Path) -> Result<Vec<u8>, SyncError> {
    let out = tmp.with_extension("gz");
    let result = export_to(conn, tmp, &out).and_then(|_| Ok(std::fs::read(&out)?));
    let _ = std::fs::remove_file(&out);
    result
}

/// Merges a downloaded snapshot (gzipped, from `export` on any device) into `conn`.
/// `tmp` is a scratch path for the unpacked file. Returns the ids of local books
/// removed because they were deleted elsewhere (their kept originals can go).
#[cfg(test)]
pub fn merge(conn: &mut Connection, snapshot: &[u8], tmp: &Path) -> Result<Vec<i64>, SyncError> {
    merge_from(conn, snapshot, tmp)
}

/// `merge`, reading the snapshot from `snapshot` as it goes (a downloaded file).
#[cfg(test)]
pub fn merge_from(
    conn: &mut Connection,
    snapshot: impl Read,
    tmp: &Path,
) -> Result<Vec<i64>, SyncError> {
    merge_with(conn, snapshot, tmp, None)
}

/// What a restore brought back besides the library.
#[derive(Debug, Default)]
pub struct Restored {
    /// Ids of local books removed because the backup knows them as deleted.
    pub gone: Vec<i64>,
    /// The app's settings kept in the backup (JSON), for the app to apply.
    pub settings: Option<String>,
}

/// Brings a backup (`backup_to`) back: merged like a sync, except that what the backup
/// holds comes back even if it was removed here since. A book deleted here returns
/// as a new book, so the deletion other devices know of doesn't remove it again.
/// The backup's LLM settings replace these when they are newer (or there are none).
/// The kept originals it carries are written into `originals`.
pub fn restore_from(
    conn: &mut Connection,
    snapshot: impl Read,
    tmp: &Path,
    originals: Option<&Path>,
) -> Result<Restored, SyncError> {
    let mut settings = None;
    let gone = merge_with(conn, snapshot, tmp, Some((&mut settings, originals)))?;
    Ok(Restored { gone, settings })
}

fn merge_with(
    conn: &mut Connection,
    snapshot: impl Read,
    tmp: &Path,
    // A restore, with where to put the backup's app settings and its originals.
    restore: Option<(&mut Option<String>, Option<&Path>)>,
) -> Result<Vec<i64>, SyncError> {
    let _ = std::fs::remove_file(tmp);
    let result = (|| {
        let mut file = std::io::BufWriter::new(std::fs::File::create(tmp)?);
        std::io::copy(&mut GzDecoder::new(snapshot), &mut file)?;
        file.flush()?;
        drop(file);
        // Brings a file from an older app up to this schema.
        let remote = db::open(tmp)?;
        let version: i64 = remote.pragma_query_value(None, "user_version", |r| r.get(0))?;
        remote.pragma_update(None, "journal_mode", "DELETE")?;
        drop(remote);
        if version > db::schema_version() {
            return Err(SyncError::Outdated);
        }
        conn.execute("attach database ?1 as r", [tmp.to_string_lossy()])?;
        let merged = (|| {
            let before: HashSet<i64> = conn
                .prepare("select id from main.novels")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let originals = restore.as_ref().and_then(|r| r.1);
            let tx = conn.transaction()?;
            if let Some((settings, _)) = restore {
                tx.execute_batch(
                    "delete from r.tombstones;
                     update r.novels set book_key = lower(hex(randomblob(16))) where book_key in (
                       select key from main.tombstones where kind = 'book');
                     insert into main.llm_config (id, provider, base_url, api_key, model, updated_at)
                       select id, provider, base_url, api_key, model, updated_at from r.llm_config where true
                       on conflict (id) do update set provider = excluded.provider,
                         base_url = excluded.base_url, api_key = excluded.api_key,
                         model = excluded.model, updated_at = excluded.updated_at
                       where excluded.updated_at > llm_config.updated_at;",
                )?;
                *settings = tx
                    .query_row(
                        "select value from r.sync_state where key = ?1",
                        [APP_SETTINGS],
                        |r| r.get(0),
                    )
                    .optional()?;
            }
            let gone = merge_attached(&tx)?;
            tx.commit()?;
            if let Some(dir) = originals {
                unpack_originals(conn, dir, &before)?;
            }
            Ok::<_, SyncError>(gone)
        })();
        conn.execute("detach database r", [])?;
        merged
    })();
    let _ = std::fs::remove_file(tmp);
    result
}

fn merge_attached(tx: &Transaction) -> Result<Vec<i64>, SyncError> {
    tx.execute_batch(
        "
        insert into main.tombstones (kind, key, deleted_at)
          select kind, key, deleted_at from r.tombstones where true
          on conflict (kind, key) do update set deleted_at = max(deleted_at, excluded.deleted_at);

        insert or ignore into main.languages select * from r.languages;

        insert into main.rule_lists (scope, key, lang, rules, updated_at)
          select scope, key, lang, rules, updated_at from r.rule_lists where true
          on conflict (scope, key, lang) do update set rules = excluded.rules, updated_at = excluded.updated_at
          where excluded.updated_at > rule_lists.updated_at;
        delete from main.rule_lists where scope = 'book' and key in (
          select key from main.tombstones where kind = 'book');
        ",
    )?;

    let remote_books: Vec<(i64, String, String)> = tx
        .prepare(
            "select n.id, n.book_key, n.updated_at from r.novels n
             where n.book_key is not null and not exists (
               select 1 from main.tombstones t where t.kind = 'book' and t.key = n.book_key)",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (rid, key, updated_at) in remote_books {
        let local: Option<(i64, String)> = tx
            .query_row(
                "select id, updated_at from main.novels where book_key = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let lid = match local {
            None => copy_book(tx, rid)?,
            Some((lid, local_updated)) => {
                if updated_at > local_updated {
                    copy_details(tx, rid, lid)?;
                }
                merge_ai_texts(tx, rid, lid)?;
                lid
            }
        };
        tx.execute(
            "insert into main.reading_history (novel_id, chapter_id, chapter_number, lang, progress, last_read_at)
             select ?2, c.id, h.chapter_number, h.lang, h.progress, h.last_read_at
             from r.reading_history h join main.chapters c on c.novel_id = ?2 and c.number = h.chapter_number
             where h.novel_id = ?1
             on conflict (novel_id) do update set
               chapter_id = excluded.chapter_id, chapter_number = excluded.chapter_number,
               lang = excluded.lang, progress = excluded.progress, last_read_at = excluded.last_read_at
             where excluded.last_read_at > reading_history.last_read_at",
            params![rid, lid],
        )?;
    }

    super::wiki::merge_attached(tx)?;

    // Books deleted on another device.
    let gone: Vec<i64> = tx
        .prepare(
            "select id from main.novels n where exists (
               select 1 from main.tombstones t where t.kind = 'book' and t.key = n.book_key)",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &gone {
        tx.execute("delete from main.novels where id = ?1", [id])?;
    }
    tx.execute(
        "delete from main.authors where id not in (select author_id from main.novels)",
        [],
    )?;
    Ok(gone)
}

/// The local author for remote book `rid`'s author, added when missing.
fn author_for(tx: &Transaction, rid: i64) -> rusqlite::Result<i64> {
    tx.execute(
        "insert or ignore into main.authors (slug, name, created_at)
         select a.slug, a.name, a.created_at from r.authors a join r.novels n on n.author_id = a.id
         where n.id = ?1",
        [rid],
    )?;
    tx.query_row(
        "select m.id from main.authors m
         join r.authors a on a.slug = m.slug join r.novels n on n.author_id = a.id
         where n.id = ?1",
        [rid],
        |r| r.get(0),
    )
}

/// AI translations made on the other device for a book both have: per chapter,
/// language and style, the newer one wins.
fn merge_ai_texts(tx: &Transaction, rid: i64, lid: i64) -> rusqlite::Result<()> {
    tx.execute(
        "insert into main.chapter_translations (chapter_id, lang, style, source, state, title, content,
            word_count, model, created_at, updated_at)
         select c.id, t.lang, t.style, t.source, t.state, t.title, t.content,
           t.word_count, t.model, t.created_at, t.updated_at
         from r.chapter_translations t
         join r.chapters rc on rc.id = t.chapter_id
         join main.chapters c on c.novel_id = ?2 and c.number = rc.number
         where rc.novel_id = ?1 and t.source = 'ai'
         on conflict (chapter_id, lang, style) do update set
           title = excluded.title, content = excluded.content, word_count = excluded.word_count,
           model = excluded.model, updated_at = excluded.updated_at
         where excluded.updated_at > chapter_translations.updated_at",
        params![rid, lid],
    )?;
    Ok(())
}

/// Copies remote book `rid` with everything in it; returns the new local id.
fn copy_book(tx: &Transaction, rid: i64) -> rusqlite::Result<i64> {
    let author = author_for(tx, rid)?;
    tx.execute(
        "insert into main.novels (slug, author_id, category_slug, original_lang, status, published_year,
           cover_url, cover_palette, chapter_count, created_at, updated_at, source_format, source_name, book_key)
         select case when exists (select 1 from main.novels m where m.slug = n.slug)
                  then n.slug || '-' || n.book_key else n.slug end,
           ?2, category_slug, original_lang, status, published_year, cover_url, cover_palette,
           chapter_count, created_at, updated_at, source_format, source_name, book_key
         from r.novels n where n.id = ?1",
        params![rid, author],
    )?;
    let lid = tx.last_insert_rowid();
    tx.execute_batch(&format!(
        "
        insert into main.novel_translations (novel_id, lang, title, description, source, updated_at)
          select {lid}, lang, title, description, source, updated_at
          from r.novel_translations where novel_id = {rid};
        insert into main.chapters (novel_id, number, display_number, page_from, page_to,
            published_at, created_at, updated_at)
          select {lid}, number, display_number, page_from, page_to, published_at, created_at, updated_at
          from r.chapters where novel_id = {rid};
        insert into main.chapter_translations (chapter_id, lang, style, source, state, title, content,
            word_count, model, created_at, updated_at)
          select c.id, t.lang, t.style, t.source, t.state, t.title, t.content,
            t.word_count, t.model, t.created_at, t.updated_at
          from r.chapter_translations t
          join r.chapters rc on rc.id = t.chapter_id
          join main.chapters c on c.novel_id = {lid} and c.number = rc.number
          where rc.novel_id = {rid};
        insert into main.chapter_images (novel_id, key, mime, data)
          select {lid}, key, mime, data from r.chapter_images where novel_id = {rid};
        insert into main.glossary_entries (novel_id, term_key, lang, value, note, created_at, updated_at)
          select {lid}, term_key, lang, value, note, created_at, updated_at
          from r.glossary_entries where novel_id = {rid};
        insert into main.replace_rules (novel_id, lang, find, replace, position)
          select {lid}, lang, find, replace, position from r.replace_rules where novel_id = {rid};
        "
    ))?;
    Ok(lid)
}

/// Takes the newer title, description and author of remote book `rid`.
fn copy_details(tx: &Transaction, rid: i64, lid: i64) -> rusqlite::Result<()> {
    let author = author_for(tx, rid)?;
    tx.execute(
        "update main.novels set author_id = ?2, updated_at = (select updated_at from r.novels where id = ?3)
         where id = ?1",
        params![lid, author, rid],
    )?;
    tx.execute(
        "insert into main.novel_translations (novel_id, lang, title, description, source, updated_at)
         select ?1, lang, title, description, source, updated_at from r.novel_translations where novel_id = ?2
         on conflict (novel_id, lang) do update set
           title = excluded.title, description = excluded.description, updated_at = excluded.updated_at",
        params![lid, rid],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::image::Image;
    use crate::book::{BookError, BookFormat, BookSource, Section, SectionMeta};

    /// A two-chapter book with a picture.
    struct Book;

    impl BookSource for Book {
        fn title(&self) -> Option<String> {
            Some("Truyện thử".into())
        }
        fn sections(&self) -> Vec<SectionMeta> {
            (0..2)
                .map(|_| SectionMeta {
                    label: None,
                    pages: None,
                    number: None,
                })
                .collect()
        }
        fn load(&mut self, index: usize) -> Result<Section, BookError> {
            Ok(Section {
                index,
                paragraphs: vec![format!("Chương {}", index + 1), "\u{FFFC}a.png".into()],
            })
        }
        fn image(&mut self, _key: &str) -> Result<Image, BookError> {
            Ok(Image::sniff(b"\x89PNG".to_vec()))
        }
    }

    /// A fresh library file standing for one device.
    fn device(name: &str) -> (Connection, std::path::PathBuf) {
        let path =
            crate::book::test_dir().join(format!("sync-{name}-{}.sqlite3", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        (db::open(&path).unwrap(), path)
    }

    /// Copies `from`'s library into `to` as a sync would (export, then merge).
    fn send(from: &Connection, from_path: &Path, to: &mut Connection, to_path: &Path) {
        let bytes = export(from, &from_path.with_extension("out")).unwrap();
        merge(to, &bytes, &to_path.with_extension("in")).unwrap();
    }

    #[test]
    fn replace_rules_follow_the_novel_and_the_latest_edit_wins() {
        use super::super::rules::{self, Rule};
        let r = |f: &str| {
            vec![Rule {
                find: f.into(),
                replace: "x".into(),
            }]
        };
        let (mut a, pa) = device("rules-a");
        let (mut b, pb) = device("rules-b");
        let book =
            super::super::import(&mut a, BookFormat::Txt, &mut Book, "thu.txt", |_, _| {}).unwrap();
        rules::set(&a, book, "vi", &r("book")).unwrap();
        send(&a, &pa, &mut b, &pb);
        // The book has another id on B; its rules came along by book_key.
        let b_book = super::super::list(&b).unwrap()[0].novel_id;
        assert_eq!(rules::get(&b, b_book, "vi").unwrap(), Some(r("book")));

        // A edits, then B empties the list later: B's empty list wins on both.
        rules::set(&a, book, "vi", &r("a2")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        rules::set(&b, b_book, "vi", &[]).unwrap();
        send(&a, &pa, &mut b, &pb);
        send(&b, &pb, &mut a, &pa);
        assert_eq!(rules::get(&a, book, "vi").unwrap(), Some(vec![]));
        assert_eq!(rules::get(&b, b_book, "vi").unwrap(), Some(vec![]));

        // Deleting the book drops its rules on the other device too.
        super::super::delete(&a, book).unwrap();
        send(&a, &pa, &mut b, &pb);
        let left: i64 = b
            .query_row(
                "select count(*) from rule_lists where scope = 'book'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn copies_books_and_keeps_deletions() {
        let (mut a, pa) = device("book-a");
        let (mut b, pb) = device("book-b");
        let id =
            super::super::import(&mut a, BookFormat::Txt, &mut Book, "thu.txt", |_, _| {}).unwrap();
        super::super::save_progress(&a, id, 1, 0.5).unwrap();
        send(&a, &pa, &mut b, &pb);

        let books = super::super::list(&b).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Truyện thử");
        assert_eq!(
            books[0].last_read.as_ref().map(|l| (l.index, l.progress)),
            Some((1, 0.5))
        );
        let bid = books[0].novel_id;
        let (chapters, pictures): (i64, i64) = b
            .query_row(
                "select (select count(*) from chapter_translations t join chapters c on c.id = t.chapter_id where c.novel_id = ?1),
                        (select count(*) from chapter_images where novel_id = ?1)",
                [bid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((chapters, pictures), (2, 1));

        // Syncing again doesn't copy it twice; a rename on B wins over the older details on A.
        std::thread::sleep(std::time::Duration::from_millis(5));
        super::super::update_details(&mut b, bid, "Tên mới", "Tác giả", "").unwrap();
        send(&b, &pb, &mut a, &pa);
        let books = super::super::list(&a).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Tên mới");
        assert_eq!(books[0].author.as_deref(), Some("Tác giả"));

        // Deleted on A: gone from B too, and not brought back by B's copy.
        super::super::delete(&a, id).unwrap();
        send(&b, &pb, &mut a, &pa);
        assert!(super::super::list(&a).unwrap().is_empty());
        send(&a, &pa, &mut b, &pb);
        assert!(super::super::list(&b).unwrap().is_empty());
    }

    /// An AI translation of chapter `number` of book `id`, as translate.rs saves it.
    fn ai_text(conn: &Connection, id: i64, number: i64, content: &str) {
        conn.execute(
            "insert into chapter_translations (chapter_id, lang, style, source, state, title, content, model)
             select id, 'vi', 'natural', 'ai', 'published', 'T', ?3, 'm' from chapters
             where novel_id = ?1 and number = ?2
             on conflict (chapter_id, lang, style) do update set content = excluded.content,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![id, number, content],
        )
        .unwrap();
    }

    fn ai_texts(conn: &Connection) -> Vec<String> {
        conn.prepare("select content from chapter_translations where source = 'ai' order by chapter_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn ai_translations_made_later_reach_the_other_device() {
        let (mut a, pa) = device("ai-a");
        let (mut b, pb) = device("ai-b");
        let id =
            super::super::import(&mut a, BookFormat::Txt, &mut Book, "thu.txt", |_, _| {}).unwrap();
        send(&a, &pa, &mut b, &pb);
        // Both have the book; A translates a chapter afterwards.
        ai_text(&a, id, 1, "Bản dịch 1");
        send(&a, &pa, &mut b, &pb);
        assert_eq!(ai_texts(&b), vec!["Bản dịch 1"]);
        // A newer translation on B wins back on A.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let bid = super::super::list(&b).unwrap()[0].novel_id;
        ai_text(&b, bid, 1, "Bản dịch mới");
        send(&b, &pb, &mut a, &pa);
        assert_eq!(ai_texts(&a), vec!["Bản dịch mới"]);
    }

    #[test]
    fn a_backup_brings_back_a_deleted_book_with_its_translations() {
        let (mut a, pa) = device("bak-a");
        let (mut b, pb) = device("bak-b");
        let id =
            super::super::import(&mut a, BookFormat::Txt, &mut Book, "thu.txt", |_, _| {}).unwrap();
        ai_text(&a, id, 2, "Bản dịch 2");
        super::super::llm::save(
            &a,
            Some(&super::super::llm::Config {
                provider: "openai".into(),
                base_url: "https://api.openai.com/v1".into(),
                api_key: "sk-secret".into(),
                model: "m".into(),
            }),
        )
        .unwrap();
        send(&a, &pa, &mut b, &pb);

        let backup = pa.with_extension("bak.gz");
        backup_to(&a, &pa.with_extension("bak"), &backup, Some(r#"{"uiLang":"vi"}"#), None).unwrap();

        // Deleted, and the deletion synced: a plain sync doesn't bring it back…
        super::super::delete(&a, id).unwrap();
        send(&a, &pa, &mut b, &pb);
        assert!(super::super::list(&b).unwrap().is_empty());
        // …the backup does, translation included.
        let restored =
            restore_from(&mut a, std::fs::File::open(&backup).unwrap(), &pa.with_extension("in"), None).unwrap();
        assert_eq!(super::super::list(&a).unwrap().len(), 1);
        // The app's settings come back to be applied.
        assert_eq!(restored.settings.as_deref(), Some(r#"{"uiLang":"vi"}"#));
        assert_eq!(ai_texts(&a), vec!["Bản dịch 2"]);
        // And it stays when synced with the device that saw the deletion.
        send(&b, &pb, &mut a, &pa);
        assert_eq!(super::super::list(&a).unwrap().len(), 1);
        send(&a, &pa, &mut b, &pb);
        assert_eq!(super::super::list(&b).unwrap().len(), 1);
    }

    #[test]
    fn a_backup_carries_the_llm_settings_and_app_settings_to_a_new_device() {
        let (a, pa) = device("llm-a");
        let (mut c, _) = device("llm-c");
        let config = super::super::llm::Config {
            provider: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: "sk-secret".into(),
            model: "m".into(),
        };
        super::super::llm::save(&a, Some(&config)).unwrap();
        let backup = pa.with_extension("llm.gz");
        backup_to(&a, &pa.with_extension("llm"), &backup, Some("{}"), None).unwrap();

        let restored =
            restore_from(&mut c, std::fs::File::open(&backup).unwrap(), &pa.with_extension("llm-in"), None).unwrap();
        assert_eq!(restored.settings.as_deref(), Some("{}"));
        let key: String = c
            .query_row("select api_key from llm_config", [], |r| r.get(0))
            .unwrap();
        assert_eq!(key, "sk-secret");
        // A synced copy still never carries the key.
        let bytes = export(&a, &pa.with_extension("out")).unwrap();
        let mut raw = Vec::new();
        GzDecoder::new(&bytes[..]).read_to_end(&mut raw).unwrap();
        assert!(!raw.windows(9).any(|w| w == b"sk-secret"));
    }

    /// A five-page PDF read in chapters of two pages.
    struct Pdf;

    impl BookSource for Pdf {
        fn title(&self) -> Option<String> {
            Some("Sách PDF".into())
        }
        fn sections(&self) -> Vec<SectionMeta> {
            [[1, 2], [3, 4], [5, 5]]
                .into_iter()
                .map(|p| SectionMeta {
                    label: None,
                    pages: Some(p),
                    number: None,
                })
                .collect()
        }
        fn load(&mut self, index: usize) -> Result<Section, BookError> {
            Ok(Section {
                index,
                paragraphs: vec![format!("Phần {}", index + 1)],
            })
        }
        fn image(&mut self, key: &str) -> Result<Image, BookError> {
            Err(BookError::NoImage(key.into()))
        }
    }

    fn pages_of(path: &Path, id: i64) -> Vec<Option<[u32; 2]>> {
        let (_, book) = crate::library::DbBook::open(path, id, None).unwrap();
        book.sections().into_iter().map(|s| s.pages).collect()
    }

    #[test]
    fn a_restored_pdf_keeps_its_original_and_its_chapters() {
        let (mut a, pa) = device("pdf-a");
        let (mut c, pc) = device("pdf-c");
        let (dir_a, dir_c) = (pa.with_extension("files"), pc.with_extension("files"));
        for d in [&dir_a, &dir_c] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).unwrap();
        }
        // C already has a book, and a stale original under the id the restored book gets.
        super::super::import(&mut c, BookFormat::Txt, &mut Book, "khac.txt", |_, _| {}).unwrap();
        std::fs::write(dir_c.join("2.pdf"), b"stale").unwrap();

        let id = super::super::import(&mut a, BookFormat::Pdf, &mut Pdf, "sach.pdf", |_, _| {})
            .unwrap();
        let pdf: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir_a.join(format!("{id}.pdf")), &pdf).unwrap();
        let backup = pa.with_extension("pdf.gz");
        backup_to(&a, &pa.with_extension("pdf-bak"), &backup, None, Some(&dir_a)).unwrap();

        restore_from(
            &mut c,
            std::fs::File::open(&backup).unwrap(),
            &pc.with_extension("in"),
            Some(&dir_c),
        )
        .unwrap();
        let cid = super::super::list(&c)
            .unwrap()
            .into_iter()
            .find(|b| b.title == "Sách PDF")
            .unwrap()
            .novel_id;
        assert_eq!(cid, 2);
        // Same chapters, same pages, and the original to draw them from.
        let want = vec![Some([1, 2]), Some([3, 4]), Some([5, 5])];
        assert_eq!(pages_of(&pa, id), want);
        assert_eq!(pages_of(&pc, cid), want);
        assert_eq!(std::fs::read(dir_c.join(format!("{cid}.pdf"))).unwrap(), pdf);
        // Restoring again leaves the original in place.
        restore_from(
            &mut c,
            std::fs::File::open(&backup).unwrap(),
            &pc.with_extension("in"),
            Some(&dir_c),
        )
        .unwrap();
        assert_eq!(super::super::list(&c).unwrap().len(), 2);
        assert_eq!(std::fs::read(dir_c.join(format!("{cid}.pdf"))).unwrap(), pdf);
    }

    #[test]
    fn a_book_imported_before_page_ranges_takes_them_from_its_original() {
        let (mut a, pa) = device("pdf-old");
        let id = super::super::import(&mut a, BookFormat::Pdf, &mut Pdf, "sach.pdf", |_, _| {})
            .unwrap();
        a.execute("update chapters set page_from = null, page_to = null", [])
            .unwrap();
        assert_eq!(pages_of(&pa, id), vec![None, None, None]);
        crate::library::DbBook::open(&pa, id, Some(Box::new(Pdf))).unwrap();
        assert_eq!(pages_of(&pa, id), vec![Some([1, 2]), Some([3, 4]), Some([5, 5])]);
    }

    #[test]
    fn refuses_a_file_from_a_newer_app() {
        let (a, pa) = device("new-a");
        let (mut b, pb) = device("new-b");
        a.pragma_update(None, "user_version", db::schema_version() + 1)
            .unwrap();
        let bytes = export(&a, &pa.with_extension("out")).unwrap();
        assert!(matches!(
            merge(&mut b, &bytes, &pb.with_extension("in")),
            Err(SyncError::Outdated)
        ));
    }
}
