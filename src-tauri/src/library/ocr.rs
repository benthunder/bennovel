//! Text for scanned pages. A PDF/DjVu book keeps its original file; pages whose text
//! layer is (nearly) empty are drawn as pictures and read by Tesseract in the app
//! (tesseract.js), one chapter at a time, and the chapter's text is replaced with
//! the result. Readers then only ever see text.
//!
//! Progress per book is kept in `sync_state` under `ocr:{novel_id}`: the next chapter
//! index to check, or `done`. Books with a kept original and no entry (imported
//! before this existed, or just now) are queued when the next task is asked for.

use super::{db, original_file, originals_dir};
use crate::book::text::pdf_text_to_paragraphs;
use crate::book::{BookError, BookSource};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

/// Fewer letters than this on a page means it is a scan (page numbers and running
/// heads alone don't count as a text layer).
const MIN_LETTERS: usize = 40;

/// Pixel width pages are drawn at for Tesseract (about 200 dpi for A5–A4 pages).
pub const OCR_WIDTH: u32 = 1700;

const DONE: &str = "done";

fn key(novel_id: i64) -> String {
    format!("ocr:{novel_id}")
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrPage {
    pub page: u32,
    /// The page's own text layer.
    pub text: String,
    /// The text layer is missing: Tesseract should read the picture.
    pub ocr: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrTask {
    pub novel_id: i64,
    pub title: String,
    /// Chapter (0-based) whose pages are listed.
    pub index: usize,
    pub chapters: usize,
    pub pages: Vec<OcrPage>,
}

fn is_scan(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphabetic()).count() < MIN_LETTERS
}

/// Queues the books that have a kept original and were never checked.
fn queue_new(conn: &Connection, db_file: &Path) -> Result<(), BookError> {
    let Ok(entries) = std::fs::read_dir(originals_dir(db_file)) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|n| n.split_once('.'))
            .and_then(|(id, _)| id.parse::<i64>().ok())
        else {
            continue;
        };
        if db::get_state(conn, &key(id))?.is_none() {
            db::set_state(conn, &key(id), "0")?;
        }
    }
    Ok(())
}

/// The next chapter with scanned pages, skipping (and marking as checked) chapters
/// whose pages all have text. `open` opens a book's kept original.
pub fn next(
    conn: &Connection,
    db_file: &Path,
    mut open: impl FnMut(i64) -> Option<Box<dyn BookSource>>,
) -> Result<Option<OcrTask>, BookError> {
    queue_new(conn, db_file)?;
    let queued: Vec<(i64, usize)> = {
        let mut q = conn.prepare(
            "select cast(substr(key, 5) as integer), cast(value as integer) from sync_state
             where key like 'ocr:%' and value <> ?1 order by 1",
        )?;
        let rows = q.query_map([DONE], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (novel_id, from) in queued {
        let title: Option<String> = conn
            .query_row(
                "select t.title from novels n
                 join novel_translations t on t.novel_id = n.id and t.lang = n.original_lang
                 where n.id = ?1",
                [novel_id],
                |r| r.get(0),
            )
            .optional()?;
        let source = original_file(db_file, novel_id).and_then(|_| open(novel_id));
        let (Some(title), Some(mut source)) = (title, source) else {
            // Deleted, or the original can't be opened (no Pdfium): nothing to do.
            db::set_state(conn, &key(novel_id), DONE)?;
            continue;
        };
        let sections = source.sections();
        for (index, meta) in sections.iter().enumerate().skip(from) {
            let Some([first, last]) = meta.pages else {
                continue;
            };
            let mut pages = Vec::new();
            for page in first..=last {
                let text = source.page_text(page)?.unwrap_or_default();
                pages.push(OcrPage {
                    page,
                    ocr: is_scan(&text),
                    text,
                });
            }
            if pages.iter().any(|p| p.ocr) {
                return Ok(Some(OcrTask {
                    novel_id,
                    title,
                    index,
                    chapters: sections.len(),
                    pages,
                }));
            }
            db::set_state(conn, &key(novel_id), &(index + 1).to_string())?;
        }
        db::set_state(conn, &key(novel_id), DONE)?;
    }
    Ok(None)
}

/// Replaces chapter `index`'s text with `pages` (each page's text, in order) and
/// moves the book on to the next chapter.
pub fn save(
    conn: &mut Connection,
    novel_id: i64,
    index: usize,
    chapters: usize,
    pages: &[String],
) -> Result<(), BookError> {
    let paragraphs: Vec<String> = pages
        .iter()
        .flat_map(|p| pdf_text_to_paragraphs(p))
        .collect();
    let words: i64 = paragraphs.iter().map(|p| super::word_count(p)).sum();
    let tx = conn.transaction()?;
    tx.execute(
        "update chapter_translations set content = ?3, word_count = ?4,
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where source = 'original'
           and chapter_id = (select id from chapters where novel_id = ?1 and number = ?2)",
        params![novel_id, index as i64 + 1, paragraphs.join("\n\n"), words],
    )?;
    let next = if index + 1 >= chapters {
        DONE.to_string()
    } else {
        (index + 1).to_string()
    };
    db::set_state(&tx, &key(novel_id), &next)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::{image::Image, Section, SectionMeta};

    /// Two chapters of two pages: page 3 is a scan.
    struct Scanned;
    impl BookSource for Scanned {
        fn title(&self) -> Option<String> {
            None
        }
        fn sections(&self) -> Vec<SectionMeta> {
            [[1, 2], [3, 4]]
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
                paragraphs: vec![],
            })
        }
        fn render_page(&mut self, _page: u32, _width: u32) -> Result<Image, BookError> {
            unreachable!()
        }
        fn page_text(&mut self, page: u32) -> Result<Option<String>, BookError> {
            Ok(Some(if page == 3 {
                "  12 ".into()
            } else {
                format!(
                    "Trang {page} có lớp chữ đầy đủ, đủ dài để không phải nhận dạng lại từ ảnh."
                )
            }))
        }
    }

    #[test]
    fn finds_scanned_pages_and_saves_their_text() {
        let dir = std::env::temp_dir().join(format!("bennovel-ocr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db_file = dir.join("guest.sqlite3");
        let mut conn = db::open(&db_file).unwrap();
        conn.execute_batch(
            "insert into authors (id, slug, name) values (1, 'unknown', 'Unknown');
             insert into novels (id, slug, author_id, original_lang, chapter_count) values (7, 'b', 1, 'vi', 2);
             insert into novel_translations (novel_id, lang, title) values (7, 'vi', 'Sách');
             insert into chapters (id, novel_id, number) values (70, 7, 1), (71, 7, 2);
             insert into chapter_translations (chapter_id, lang, source, state, title, content)
               values (70, 'vi', 'original', 'published', '', 'a'), (71, 'vi', 'original', 'published', '', 'b');",
        )
        .unwrap();
        std::fs::create_dir_all(originals_dir(&db_file)).unwrap();
        std::fs::write(originals_dir(&db_file).join("7.pdf"), b"").unwrap();

        let open = |_| Some(Box::new(Scanned) as Box<dyn BookSource>);
        let task = next(&conn, &db_file, open).unwrap().unwrap();
        assert_eq!((task.novel_id, task.index, task.chapters), (7, 1, 2));
        assert_eq!(task.title, "Sách");
        assert_eq!(
            task.pages.iter().map(|p| p.ocr).collect::<Vec<_>>(),
            [true, false]
        );
        // Chapter 1 had text on every page and is not asked again.
        assert_eq!(db::get_state(&conn, "ocr:7").unwrap().as_deref(), Some("1"));

        save(
            &mut conn,
            7,
            1,
            2,
            &["Dòng một\ncủa trang ba.".into(), "Trang bốn.".into()],
        )
        .unwrap();
        let content: String = conn
            .query_row(
                "select content from chapter_translations where chapter_id = 71",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "Dòng một của trang ba.\n\nTrang bốn.");
        assert!(next(&conn, &db_file, open).unwrap().is_none());
        assert_eq!(
            db::get_state(&conn, "ocr:7").unwrap().as_deref(),
            Some("done")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
