//! Backups of the library: the library file is exported as a gzipped snapshot, and a
//! backup brought back is merged into it row by row, so nothing made since is lost
//! (s3.rs carries backups to the user's own storage). The merge rules:
//! - favourites: the later of "added" and "removed" wins (a tie keeps it);
//! - history and progress: the most recent read wins; a removal newer than the read wins;
//! - imported books: matched by `book_key`; a book missing here is copied whole
//!   (chapters, pictures…), newer details (title, author) win, a deleted book stays deleted;
//! - replace rules: per novel and language, the list changed last wins.

use super::db;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
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

/// A consistent copy of the library file (the live one is in WAL mode), gzipped into
/// `out`; returns its size. `tmp` is a scratch path that must not exist.
#[cfg(test)]
pub fn export_to(conn: &Connection, tmp: &Path, out: &Path) -> Result<u64, SyncError> {
    snapshot(conn, tmp, out, false)
}

/// A backup: like the synced copy, with the downloaded online novels kept too. The
/// LLM settings and the AI queue are still left out.
pub fn backup_to(conn: &Connection, tmp: &Path, out: &Path) -> Result<u64, SyncError> {
    snapshot(conn, tmp, out, true)
}

fn snapshot(conn: &Connection, tmp: &Path, out: &Path, downloads: bool) -> Result<u64, SyncError> {
    let _ = std::fs::remove_file(tmp);
    conn.execute("vacuum into ?1", [tmp.to_string_lossy()])?;
    let result = (|| {
        let copy = Connection::open(tmp)?;
        // Downloaded online novels stay on this device (a backup keeps them).
        if !downloads {
            super::offline::strip(&copy)?;
        }
        // The LLM settings (with their API key) and the AI queue never leave the device.
        super::translate::strip(&copy)?;
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
    merge_with(conn, snapshot, tmp, false)
}

/// Brings a backup (`backup_to`) back: merged like a sync, except that what the backup
/// holds comes back even if it was removed here since. A book deleted here returns
/// as a new book, so the deletion other devices know of doesn't remove it again.
pub fn restore_from(
    conn: &mut Connection,
    snapshot: impl Read,
    tmp: &Path,
) -> Result<Vec<i64>, SyncError> {
    merge_with(conn, snapshot, tmp, true)
}

fn merge_with(
    conn: &mut Connection,
    snapshot: impl Read,
    tmp: &Path,
    restore: bool,
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
            let tx = conn.transaction()?;
            if restore {
                tx.execute_batch(
                    "delete from r.tombstones;
                     update r.novels set book_key = lower(hex(randomblob(16))) where book_key in (
                       select key from main.tombstones where kind = 'book');",
                )?;
            }
            let gone = merge_attached(&tx)?;
            super::reading::bump_rev(&tx)?;
            tx.commit()?;
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

        -- Snapshots of online novels; a real one replaces a placeholder.
        insert into main.online_novels (id, data, updated_at)
          select id, data, updated_at from r.online_novels where true
          on conflict (id) do update set data = excluded.data, updated_at = excluded.updated_at
          where online_novels.data = 'null' and excluded.data <> 'null';

        -- Favourites from the other device go after the ones here.
        insert into main.online_favorites (novel_id, position, added_at)
          select f.novel_id, f.position + 1 + (select coalesce(max(position), -1) from main.online_favorites), f.added_at
          from r.online_favorites f where true
          on conflict (novel_id) do update set added_at = max(added_at, excluded.added_at);
        delete from main.online_favorites where exists (
          select 1 from main.tombstones t
          where t.kind = 'fav' and t.key = cast(online_favorites.novel_id as text)
            and t.deleted_at > online_favorites.added_at);

        insert into main.online_history (novel_id, chapter_number, last_read_at)
          select novel_id, chapter_number, last_read_at from r.online_history where true
          on conflict (novel_id) do update set
            chapter_number = excluded.chapter_number, last_read_at = excluded.last_read_at
          where excluded.last_read_at > online_history.last_read_at;
        delete from main.online_history where exists (
          select 1 from main.tombstones t
          where t.kind = 'history' and t.key = cast(online_history.novel_id as text)
            and t.deleted_at >= online_history.last_read_at);

        delete from main.online_novels
          where id not in (select novel_id from main.online_favorites)
            and id not in (select novel_id from main.online_history);

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

    // Downloaded online novels: only a backup carries them (sync leaves them out).
    tx.execute_batch(
        "insert or ignore into main.offline_novels select * from r.offline_novels;
         insert or ignore into main.offline_chapters select * from r.offline_chapters;
         insert or ignore into main.offline_glossary select * from r.offline_glossary;",
    )?;

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
        insert into main.chapters (novel_id, number, display_number, published_at, created_at, updated_at)
          select {lid}, number, display_number, published_at, created_at, updated_at
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
    use super::super::reading::{self, HistoryEntry, ReadingState, Snapshot};
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

    fn state(conn: &Connection) -> ReadingState {
        reading::load(conn).unwrap().unwrap_or_default()
    }

    fn save(conn: &mut Connection, favs: Vec<i64>, history: Vec<HistoryEntry>) {
        // Each change gets its own millisecond, as on real devices.
        std::thread::sleep(std::time::Duration::from_millis(3));
        let rev = state(conn).rev;
        let novels = favs
            .iter()
            .chain(history.iter().map(|h| &h.id))
            .map(|&id| Snapshot {
                id,
                data: serde_json::json!({ "id": id }),
            })
            .collect();
        assert!(reading::save(
            conn,
            &ReadingState {
                favs,
                history,
                novels,
                rev
            }
        )
        .unwrap());
    }

    fn h(id: i64, ch: i64, at: i64) -> HistoryEntry {
        HistoryEntry { id, ch, at }
    }

    #[test]
    fn merges_favourites_and_history_both_ways() {
        let (mut a, pa) = device("fav-a");
        let (mut b, pb) = device("fav-b");
        save(&mut a, vec![1, 2], vec![h(1, 5, 100)]);
        send(&a, &pa, &mut b, &pb);
        assert_eq!(state(&b).favs, vec![1, 2]);

        // A drops 2 and reads on; B adds 3 and reads 1 further, later.
        save(&mut a, vec![1], vec![h(1, 6, 200)]);
        save(&mut b, vec![1, 2, 3], vec![h(1, 9, 300), h(4, 1, 150)]);
        send(&a, &pa, &mut b, &pb);
        send(&b, &pb, &mut a, &pa);
        for conn in [&a, &b] {
            let s = state(conn);
            let mut favs = s.favs.clone();
            favs.sort();
            assert_eq!(favs, vec![1, 3]);
            assert_eq!(s.history, vec![h(1, 9, 300), h(4, 1, 150)]);
        }
    }

    #[test]
    fn downloads_stay_on_the_device() {
        use super::super::offline;
        let (mut a, pa) = device("dl-a");
        let (mut b, pb) = device("dl-b");
        let started = offline::start(&a, 9, &serde_json::json!({ "id": 9 }), 1).unwrap();
        offline::add(
            &mut a,
            9,
            &[offline::ChapterRow {
                number: 1,
                lang: "en".into(),
                source: "original".into(),
                title: "One".into(),
                content: "Text.".into(),
                words: 1,
            }],
        )
        .unwrap();
        offline::finish(&mut a, 9, started).unwrap();
        send(&a, &pa, &mut b, &pb);
        assert!(offline::list(&b).unwrap().is_empty());
        // Merging a snapshot back doesn't drop A's own download either.
        send(&b, &pb, &mut a, &pa);
        assert_eq!(offline::list(&a).unwrap().len(), 1);
    }

    #[test]
    fn a_save_from_before_the_sync_is_refused() {
        let (mut a, pa) = device("rev-a");
        let (mut b, pb) = device("rev-b");
        save(&mut a, vec![7], vec![]);
        let stale = state(&b).rev;
        send(&a, &pa, &mut b, &pb);
        let refused = ReadingState {
            favs: vec![],
            history: vec![],
            novels: vec![],
            rev: stale,
        };
        assert!(!reading::save(&mut b, &refused).unwrap());
        assert_eq!(state(&b).favs, vec![7]);
    }

    #[test]
    fn replace_rules_follow_the_novel_and_the_latest_edit_wins() {
        use super::super::rules::{self, Kind, Rule};
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
        rules::set(&a, Kind::Online, 9, "vi", &r("a1")).unwrap();
        rules::set(&a, Kind::Book, book, "vi", &r("book")).unwrap();
        send(&a, &pa, &mut b, &pb);
        // The book has another id on B; its rules came along by book_key.
        let b_book = super::super::list(&b).unwrap()[0].novel_id;
        assert_eq!(
            rules::get(&b, Kind::Book, b_book, "vi").unwrap(),
            Some(r("book"))
        );
        assert_eq!(
            rules::get(&b, Kind::Online, 9, "vi").unwrap(),
            Some(r("a1"))
        );

        // A edits, then B empties the list later: B's empty list wins on both.
        rules::set(&a, Kind::Online, 9, "vi", &r("a2")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        rules::set(&b, Kind::Online, 9, "vi", &[]).unwrap();
        send(&a, &pa, &mut b, &pb);
        send(&b, &pb, &mut a, &pa);
        assert_eq!(rules::get(&a, Kind::Online, 9, "vi").unwrap(), Some(vec![]));
        assert_eq!(rules::get(&b, Kind::Online, 9, "vi").unwrap(), Some(vec![]));

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
        super::super::offline::start(&a, 9, &serde_json::json!({ "id": 9 }), 1).unwrap();
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
        backup_to(&a, &pa.with_extension("bak"), &backup).unwrap();
        // The key never goes into a backup.
        let raw = {
            let mut s = Vec::new();
            GzDecoder::new(std::fs::File::open(&backup).unwrap()).read_to_end(&mut s).unwrap();
            s
        };
        assert!(!raw.windows(9).any(|w| w == b"sk-secret"));

        // Deleted, and the deletion synced: a plain sync doesn't bring it back…
        super::super::delete(&a, id).unwrap();
        super::super::offline::delete(&a, 9).unwrap();
        send(&a, &pa, &mut b, &pb);
        assert!(super::super::list(&b).unwrap().is_empty());
        // …the backup does, translation and download included.
        restore_from(&mut a, std::fs::File::open(&backup).unwrap(), &pa.with_extension("in")).unwrap();
        assert_eq!(super::super::list(&a).unwrap().len(), 1);
        assert_eq!(ai_texts(&a), vec!["Bản dịch 2"]);
        assert_eq!(super::super::offline::list(&a).unwrap().len(), 1);
        // And it stays when synced with the device that saw the deletion.
        send(&b, &pb, &mut a, &pa);
        assert_eq!(super::super::list(&a).unwrap().len(), 1);
        send(&a, &pa, &mut b, &pb);
        assert_eq!(super::super::list(&b).unwrap().len(), 1);
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
