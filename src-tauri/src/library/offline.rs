//! Online novels downloaded to read offline: the chapters (every published language)
//! and glossary are copied from Supabase into the user's library file. The app (JS)
//! fetches them; here they are stored and read back. Downloads stay on this device:
//! they are left out of the sync file (see `sync::export`).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One chapter in one language, as fetched from `chapter_translations`.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ChapterRow {
    pub number: i64,
    pub lang: String,
    /// original, human or ai.
    pub source: String,
    pub title: String,
    /// Plain text, paragraphs separated by a blank line (as in Supabase).
    pub content: String,
    pub words: i64,
}

/// A chapter in the downloaded list (without its text).
#[derive(Debug, Serialize, PartialEq)]
pub struct ChapterInfo {
    pub number: i64,
    pub lang: String,
    pub source: String,
    pub title: String,
    pub words: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ChapterText {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct GlossaryRow {
    pub key: String,
    pub lang: String,
    pub value: String,
}

/// A downloaded (or partly downloaded) novel.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub id: i64,
    /// The app's snapshot of the novel (title, author, cover…).
    pub data: Value,
    /// Chapters stored.
    pub chapters: i64,
    /// Chapters the novel had when the download started.
    pub total: i64,
    pub langs: Vec<String>,
    /// Language of the original text, when stored.
    pub orig_lang: Option<String>,
    /// False while downloading, or when a download was interrupted.
    pub complete: bool,
    /// Epoch ms of the last finished download.
    pub downloaded_at: Option<i64>,
}

/// Starts (or restarts) a download. Chapters already stored stay readable until
/// `finish` drops those the new download didn't bring. Returns the start time to pass
/// to `finish`.
pub fn start(conn: &Connection, id: i64, data: &Value, total: i64) -> rusqlite::Result<i64> {
    let now = super::db::now_ms();
    conn.execute(
        "insert into offline_novels (id, data, total, complete, started_at) values (?1, ?2, ?3, 0, ?4)
         on conflict (id) do update set data = excluded.data, total = excluded.total,
           complete = 0, started_at = excluded.started_at",
        params![id, data.to_string(), total, now],
    )?;
    Ok(now)
}

pub fn add(conn: &mut Connection, id: i64, chapters: &[ChapterRow]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let now = super::db::now_ms();
    {
        let mut ins = tx.prepare(
            "insert into offline_chapters (novel_id, number, lang, source, title, content, word_count, saved_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             on conflict (novel_id, number, lang) do update set source = excluded.source,
               title = excluded.title, content = excluded.content,
               word_count = excluded.word_count, saved_at = excluded.saved_at",
        )?;
        for c in chapters {
            ins.execute(params![
                id, c.number, c.lang, c.source, c.title, c.content, c.words, now
            ])?;
        }
    }
    tx.commit()
}

/// Replaces the novel's glossary.
pub fn set_glossary(conn: &mut Connection, id: i64, rows: &[GlossaryRow]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("delete from offline_glossary where novel_id = ?1", [id])?;
    {
        let mut ins = tx.prepare(
            "insert or replace into offline_glossary (novel_id, term_key, lang, value) values (?1, ?2, ?3, ?4)",
        )?;
        for g in rows {
            ins.execute(params![id, g.key, g.lang, g.value])?;
        }
    }
    tx.commit()
}

/// Marks the download started at `started` complete and drops chapters it didn't bring
/// (removed or unpublished since the last download).
pub fn finish(conn: &mut Connection, id: i64, started: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "delete from offline_chapters where novel_id = ?1 and saved_at < ?2",
        params![id, started],
    )?;
    tx.execute(
        "update offline_novels set complete = 1, downloaded_at = ?2 where id = ?1",
        params![id, super::db::now_ms()],
    )?;
    tx.commit()
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Download>> {
    let rows = conn
        .prepare(
            "select id, data, total, complete, downloaded_at,
               (select count(distinct number) from offline_chapters c where c.novel_id = n.id),
               (select group_concat(lang) from (select distinct lang from offline_chapters c
                  where c.novel_id = n.id order by lang)),
               (select lang from offline_chapters c where c.novel_id = n.id and source = 'original' limit 1)
             from offline_novels n order by coalesce(downloaded_at, started_at) desc",
        )?
        .query_map([], |r| {
            Ok(Download {
                id: r.get(0)?,
                data: serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or(Value::Null),
                total: r.get(2)?,
                complete: r.get(3)?,
                downloaded_at: r.get(4)?,
                chapters: r.get(5)?,
                langs: r
                    .get::<_, Option<String>>(6)?
                    .map(|s| s.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
                orig_lang: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Every stored chapter in every language, in reading order.
pub fn chapters(conn: &Connection, id: i64) -> rusqlite::Result<Vec<ChapterInfo>> {
    conn.prepare(
        "select number, lang, source, title, word_count from offline_chapters
         where novel_id = ?1 order by number, lang",
    )?
    .query_map([id], |r| {
        Ok(ChapterInfo {
            number: r.get(0)?,
            lang: r.get(1)?,
            source: r.get(2)?,
            title: r.get(3)?,
            words: r.get(4)?,
        })
    })?
    .collect()
}

pub fn chapter(
    conn: &Connection,
    id: i64,
    number: i64,
    lang: &str,
) -> rusqlite::Result<Option<ChapterText>> {
    conn.query_row(
        "select title, content from offline_chapters where novel_id = ?1 and number = ?2 and lang = ?3",
        params![id, number, lang],
        |r| {
            Ok(ChapterText {
                title: r.get(0)?,
                content: r.get(1)?,
            })
        },
    )
    .optional()
}

pub fn glossary(conn: &Connection, id: i64) -> rusqlite::Result<Vec<GlossaryRow>> {
    conn.prepare("select term_key, lang, value from offline_glossary where novel_id = ?1")?
        .query_map([id], |r| {
            Ok(GlossaryRow {
                key: r.get(0)?,
                lang: r.get(1)?,
                value: r.get(2)?,
            })
        })?
        .collect()
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("delete from offline_novels where id = ?1", [id])?;
    Ok(())
}

/// Empties the download tables of a copy of the library file (the sync snapshot).
pub fn strip(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "delete from offline_glossary; delete from offline_chapters; delete from offline_novels;",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("bennovel-offline-{}-{nanos}", std::process::id()));
        super::super::db::open(&dir.join("t.sqlite3")).unwrap()
    }

    fn ch(number: i64, lang: &str, source: &str) -> ChapterRow {
        ChapterRow {
            number,
            lang: lang.into(),
            source: source.into(),
            title: format!("{lang} {number}"),
            content: format!("Text {number} in {lang}.\n\nSecond paragraph."),
            words: 5,
        }
    }

    #[test]
    fn download_and_read_back() {
        let mut c = conn();
        assert!(list(&c).unwrap().is_empty());
        let started = start(&c, 7, &serde_json::json!({ "id": 7, "title": "Tale" }), 2).unwrap();
        add(&mut c, 7, &[ch(1, "zh", "original"), ch(1, "en", "ai")]).unwrap();
        let got = list(&c).unwrap();
        assert_eq!(got.len(), 1);
        assert!(!got[0].complete);
        assert_eq!(got[0].chapters, 1);

        add(&mut c, 7, &[ch(2, "zh", "original")]).unwrap();
        set_glossary(
            &mut c,
            7,
            &[GlossaryRow {
                key: "hero".into(),
                lang: "en".into(),
                value: "Lin".into(),
            }],
        )
        .unwrap();
        finish(&mut c, 7, started).unwrap();

        let d = &list(&c).unwrap()[0];
        assert!(d.complete);
        assert_eq!((d.chapters, d.total), (2, 2));
        assert_eq!(d.langs, vec!["en", "zh"]);
        assert_eq!(d.orig_lang.as_deref(), Some("zh"));
        assert_eq!(d.data["title"], "Tale");
        assert!(d.downloaded_at.is_some());

        assert_eq!(chapters(&c, 7).unwrap().len(), 3);
        let text = chapter(&c, 7, 1, "en").unwrap().unwrap();
        assert_eq!(text.title, "en 1");
        assert!(chapter(&c, 7, 2, "en").unwrap().is_none());
        assert_eq!(glossary(&c, 7).unwrap()[0].value, "Lin");
    }

    #[test]
    fn a_new_download_drops_what_it_did_not_bring() {
        let mut c = conn();
        let s = start(&c, 1, &Value::Null, 2).unwrap();
        add(
            &mut c,
            1,
            &[ch(1, "en", "original"), ch(2, "en", "original")],
        )
        .unwrap();
        finish(&mut c, 1, s).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));

        let s = start(&c, 1, &Value::Null, 1).unwrap();
        // Until it finishes, the old chapters stay readable.
        assert!(chapter(&c, 1, 2, "en").unwrap().is_some());
        add(&mut c, 1, &[ch(1, "en", "original")]).unwrap();
        finish(&mut c, 1, s).unwrap();
        assert!(chapter(&c, 1, 2, "en").unwrap().is_none());
        assert_eq!(list(&c).unwrap()[0].chapters, 1);
    }

    #[test]
    fn delete_removes_chapters_and_glossary() {
        let mut c = conn();
        let s = start(&c, 3, &Value::Null, 1).unwrap();
        add(&mut c, 3, &[ch(1, "en", "original")]).unwrap();
        set_glossary(
            &mut c,
            3,
            &[GlossaryRow {
                key: "k".into(),
                lang: "en".into(),
                value: "v".into(),
            }],
        )
        .unwrap();
        finish(&mut c, 3, s).unwrap();
        delete(&c, 3).unwrap();
        assert!(list(&c).unwrap().is_empty());
        assert!(chapters(&c, 3).unwrap().is_empty());
        assert!(glossary(&c, 3).unwrap().is_empty());
    }
}
