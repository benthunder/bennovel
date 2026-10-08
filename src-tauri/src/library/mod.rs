//! Books the user imported into their on-device library (`db.rs`). Importing reads
//! the file one section at a time and stores each one as a chapter, so the file is
//! never held in memory whole; reading then loads chapters from SQLite on demand
//! through the same windowed reader as a book file (`DbBook`).

pub mod commands;
pub mod db;
pub mod folder;

use crate::book::image::{self, Image};
use crate::book::{BookError, BookFormat, BookSource, Section, SectionMeta};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

/// A book in the library list.
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryBook {
    pub novel_id: i64,
    pub title: String,
    pub lang: String,
    pub format: Option<String>,
    pub source_name: Option<String>,
    pub chapter_count: i64,
    pub created_at: String,
    pub last_read: Option<LastRead>,
}

/// Where the reader stopped, from `reading_history`.
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastRead {
    /// 0-based section index (`chapter_number - 1`).
    pub index: usize,
    /// Scroll position inside the chapter, 0..1.
    pub progress: f64,
    pub last_read_at: String,
}

/// Guesses the book language from a sample of its text: Hangul → ko, Han/kana → zh,
/// Vietnamese letters → vi, anything else → en (the languages the schema allows).
pub fn detect_lang(sample: &str) -> &'static str {
    const VI: &str = "ăâđêôơưạảấầẩẫậắằẳẵặẹẻẽếềểễệỉịọỏốồổỗộớờởỡợụủứừửữựỳỵỷỹ";
    let (mut letters, mut hangul, mut han, mut vi) = (0usize, 0usize, 0usize, 0usize);
    for c in sample.chars().filter(|c| c.is_alphabetic()) {
        letters += 1;
        match c as u32 {
            0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => hangul += 1,
            0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF => han += 1,
            _ => {
                if c.to_lowercase().any(|l| VI.contains(l)) {
                    vi += 1;
                }
            }
        }
    }
    if letters == 0 {
        "en"
    } else if hangul * 5 >= letters {
        "ko"
    } else if han * 5 >= letters {
        "zh"
    } else if vi * 50 >= letters {
        "vi"
    } else {
        "en"
    }
}

/// Words for `word_count`: whitespace-separated words, with each Han/kana/Hangul
/// character counted as one (those scripts don't put spaces between words).
fn word_count(text: &str) -> i64 {
    let mut n = 0i64;
    for word in text.split_whitespace() {
        let cjk = word
            .chars()
            .filter(|&c| matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF))
            .count();
        n += if cjk > 0 { cjk as i64 } else { 1 };
    }
    n
}

fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
        if s.len() >= 60 {
            break;
        }
    }
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let s = s.trim_matches('-');
    if s.is_empty() {
        format!("book-{millis}")
    } else {
        format!("{s}-{millis}")
    }
}

/// Copies a book into the library, one section at a time, in a single transaction
/// (a failed import leaves nothing behind). `on_progress(done, total)` is called
/// after each chapter. Returns the new novel id.
pub fn import(
    conn: &mut Connection,
    format: BookFormat,
    source: &mut dyn BookSource,
    file_name: &str,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<i64, BookError> {
    let sections = source.sections();
    let total = sections.len();

    // The language is decided from the first chapters before anything is written.
    let mut sample = String::new();
    let mut first = Vec::new();
    for i in 0..total.min(3) {
        let s = source.load(i)?;
        for p in &s.paragraphs {
            if sample.len() < 20_000 {
                sample.push_str(p);
                sample.push('\n');
            }
        }
        first.push(s);
    }
    let lang = detect_lang(&sample);
    let title = source
        .title()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| file_stem(file_name));
    let format_name = serde_json::to_value(format)
        .ok()
        .and_then(|v| v.as_str().map(String::from));

    let tx = conn.transaction()?;
    tx.execute(
        "insert or ignore into authors (slug, name) values ('unknown', 'Unknown')",
        [],
    )?;
    let author_id: i64 =
        tx.query_row("select id from authors where slug = 'unknown'", [], |r| {
            r.get(0)
        })?;
    tx.execute(
        "insert into novels (slug, author_id, original_lang, chapter_count, source_format, source_name)
         values (?1, ?2, ?3, ?4, ?5, ?6)",
        params![slug(&title), author_id, lang, total as i64, format_name, file_name],
    )?;
    let novel_id = tx.last_insert_rowid();
    tx.execute(
        "insert into novel_translations (novel_id, lang, title) values (?1, ?2, ?3)",
        params![novel_id, lang, title],
    )?;
    {
        let mut chapter = tx.prepare(
            "insert into chapters (novel_id, number, published_at) values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        )?;
        let mut text = tx.prepare(
            "insert into chapter_translations (chapter_id, lang, source, state, title, content, word_count)
             values (?1, ?2, 'original', 'published', ?3, ?4, ?5)",
        )?;
        let mut picture = tx.prepare(
            "insert or ignore into chapter_images (novel_id, key, mime, data) values (?1, ?2, ?3, ?4)",
        )?;
        let mut stored = std::collections::HashSet::new();
        let mut first = first.into_iter();
        for (i, meta) in sections.iter().enumerate() {
            let section = match first.next() {
                Some(s) => s,
                None => source.load(i)?,
            };
            // Pictures are copied in as they are met, one at a time; a picture the
            // book can't produce is skipped (the reader hides a missing one).
            for key in section.paragraphs.iter().filter_map(|p| image::key_of(p)) {
                if stored.insert(key.to_string()) {
                    if let Ok(img) = source.image(key) {
                        picture.execute(params![novel_id, key, img.mime, img.bytes])?;
                    }
                }
            }
            let words: i64 = section
                .paragraphs
                .iter()
                .filter(|p| image::key_of(p).is_none())
                .map(|p| word_count(p))
                .sum();
            let content = section.paragraphs.join("\n\n");
            chapter.execute(params![novel_id, i as i64 + 1])?;
            let chapter_id = tx.last_insert_rowid();
            text.execute(params![
                chapter_id,
                lang,
                meta.label.clone().unwrap_or_default(),
                content,
                words
            ])?;
            on_progress(i + 1, total);
        }
    }
    tx.commit()?;
    Ok(novel_id)
}

fn file_stem(name: &str) -> String {
    // `name` can be a path or a content:// URI; keep the last segment, minus extension.
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let last = urlish_decode(last);
    match last.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => last,
    }
}

/// Decodes %XX escapes (Android document URIs encode the file name).
fn urlish_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const LIST_SQL: &str = "
    select n.id, t.title, n.original_lang, n.source_format, n.source_name, n.chapter_count,
           n.created_at, h.chapter_number, h.progress, h.last_read_at
    from novels n
    join novel_translations t on t.novel_id = n.id and t.lang = n.original_lang
    left join reading_history h on h.novel_id = n.id";

fn row_to_book(r: &rusqlite::Row) -> rusqlite::Result<LibraryBook> {
    let number: Option<i64> = r.get(7)?;
    Ok(LibraryBook {
        novel_id: r.get(0)?,
        title: r.get(1)?,
        lang: r.get(2)?,
        format: r.get(3)?,
        source_name: r.get(4)?,
        chapter_count: r.get(5)?,
        created_at: r.get(6)?,
        last_read: match number {
            Some(n) => Some(LastRead {
                index: (n - 1).max(0) as usize,
                progress: r.get(8)?,
                last_read_at: r.get(9)?,
            }),
            None => None,
        },
    })
}

/// Library books, most recently read (or imported) first.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<LibraryBook>> {
    let sql = format!("{LIST_SQL} order by coalesce(h.last_read_at, n.created_at) desc, n.id desc");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_book)?;
    rows.collect()
}

pub fn get(conn: &Connection, novel_id: i64) -> rusqlite::Result<Option<LibraryBook>> {
    let sql = format!("{LIST_SQL} where n.id = ?1");
    conn.query_row(&sql, [novel_id], row_to_book).optional()
}

/// Records where the reader is: section `index`, scrolled `progress` (0..1) into it.
pub fn save_progress(
    conn: &Connection,
    novel_id: i64,
    index: usize,
    progress: f64,
) -> rusqlite::Result<()> {
    conn.execute(
        "insert into reading_history (novel_id, chapter_id, chapter_number, lang, progress, last_read_at)
         select n.id, c.id, c.number, n.original_lang, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         from novels n join chapters c on c.novel_id = n.id and c.number = ?2
         where n.id = ?1
         on conflict (novel_id) do update set
           chapter_id = excluded.chapter_id, chapter_number = excluded.chapter_number,
           lang = excluded.lang, progress = excluded.progress, last_read_at = excluded.last_read_at",
        params![novel_id, index as i64 + 1, progress.clamp(0.0, 1.0)],
    )?;
    Ok(())
}

/// Removes a book with its chapters, pictures and history; drops authors left with
/// no books. The caller removes a kept original file.
pub fn delete(conn: &Connection, novel_id: i64) -> rusqlite::Result<()> {
    conn.execute("delete from novels where id = ?1", [novel_id])?;
    conn.execute(
        "delete from authors where id not in (select author_id from novels)",
        [],
    )?;
    Ok(())
}

/// A library book opened for reading. Only chapter titles are read up front; each
/// chapter's text is queried when the reader's window reaches it.
pub struct DbBook {
    conn: Connection,
    novel_id: i64,
    title: String,
    lang: String,
    chapters: Vec<(i64, String)>,
    /// The kept original of a PDF/DjVu book, for drawing its pages.
    original: Option<Box<dyn BookSource>>,
}

/// Folder next to the library file holding kept originals (`{novel_id}.pdf`).
pub fn originals_dir(db_path: &Path) -> std::path::PathBuf {
    db_path.with_extension("files")
}

/// The kept original of a book, if there is one.
pub fn original_file(db_path: &Path, novel_id: i64) -> Option<std::path::PathBuf> {
    ["pdf", "djvu"]
        .iter()
        .map(|ext| originals_dir(db_path).join(format!("{novel_id}.{ext}")))
        .find(|p| p.is_file())
}

impl DbBook {
    /// `original` is the kept PDF/DjVu file opened as a book, when there is one.
    pub fn open(
        path: &Path,
        novel_id: i64,
        original: Option<Box<dyn BookSource>>,
    ) -> Result<(BookFormat, Self), BookError> {
        let conn = db::open(path)?;
        let (title, lang, format): (String, String, Option<String>) = conn
            .query_row(
                "select t.title, n.original_lang, n.source_format from novels n
                 join novel_translations t on t.novel_id = n.id and t.lang = n.original_lang
                 where n.id = ?1",
                [novel_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| BookError::Parse(format!("book {novel_id} is not in the library")))?;
        let format = format
            .and_then(|f| serde_json::from_value(serde_json::Value::String(f)).ok())
            .unwrap_or(BookFormat::Txt);
        let chapters = {
            let mut stmt = conn.prepare(
                "select c.id, coalesce(t.title, '') from chapters c
                 left join chapter_translations t on t.chapter_id = c.id and t.lang = ?2 and t.source = 'original'
                 where c.novel_id = ?1 order by c.number",
            )?;
            let rows = stmt.query_map(params![novel_id, lang], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok((
            format,
            Self {
                conn,
                novel_id,
                title,
                lang,
                chapters,
                original,
            },
        ))
    }
}

impl BookSource for DbBook {
    fn title(&self) -> Option<String> {
        Some(self.title.clone())
    }

    fn sections(&self) -> Vec<SectionMeta> {
        // Page ranges come from the original; chapters were stored in its order.
        let pages = self
            .original
            .as_ref()
            .map(|o| o.sections())
            .filter(|s| s.len() == self.chapters.len());
        self.chapters
            .iter()
            .enumerate()
            .map(|(i, (_, title))| SectionMeta {
                label: (!title.is_empty()).then(|| title.clone()),
                pages: pages.as_ref().and_then(|p| p[i].pages),
            })
            .collect()
    }

    fn image(&mut self, key: &str) -> Result<Image, BookError> {
        self.conn
            .query_row(
                "select mime, data from chapter_images where novel_id = ?1 and key = ?2",
                params![self.novel_id, key],
                |r| Ok(Image::sniff(r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?
            .ok_or_else(|| BookError::NoImage(key.to_string()))
    }

    fn page_images(&self) -> bool {
        self.original.as_ref().is_some_and(|o| o.page_images())
    }

    fn render_page(&mut self, page: u32, width: u32) -> Result<Image, BookError> {
        match &mut self.original {
            Some(o) => o.render_page(page, width),
            None => Err(BookError::NoImage(format!("page {page}"))),
        }
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let (chapter_id, _) = self
            .chapters
            .get(index)
            .ok_or(BookError::NoSection(index))?;
        // Prefer the original text; otherwise any text in the book language.
        let content: Option<String> = self
            .conn
            .query_row(
                "select content from chapter_translations where chapter_id = ?1 and lang = ?2
                 order by source = 'original' desc, updated_at desc limit 1",
                params![chapter_id, self.lang],
                |r| r.get(0),
            )
            .optional()?;
        let content = content.unwrap_or_default();
        Ok(Section {
            index,
            paragraphs: content
                .split("\n\n")
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<(Option<&'static str>, Vec<&'static str>)>);

    impl BookSource for Fake {
        fn title(&self) -> Option<String> {
            None
        }
        fn sections(&self) -> Vec<SectionMeta> {
            self.0
                .iter()
                .map(|(l, _)| SectionMeta {
                    label: l.map(String::from),
                    pages: None,
                })
                .collect()
        }
        fn load(&mut self, index: usize) -> Result<Section, BookError> {
            Ok(Section {
                index,
                paragraphs: self.0[index].1.iter().map(|p| p.to_string()).collect(),
            })
        }
        fn image(&mut self, key: &str) -> Result<Image, BookError> {
            match key {
                "a.png" => Ok(Image::sniff(b"\x89PNG".to_vec())),
                _ => Err(BookError::NoImage(key.into())),
            }
        }
    }

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = crate::book::test_dir();
        let path = dir.join(format!("{name}-{}.sqlite3", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        path
    }

    #[test]
    fn detects_languages() {
        assert_eq!(detect_lang("Ngày xưa có một cô bé tên là Tấm."), "vi");
        assert_eq!(detect_lang("Once upon a time there was a girl."), "en");
        assert_eq!(detect_lang("很久很久以前，有一个女孩。"), "zh");
        assert_eq!(detect_lang("옛날 옛적에 한 소녀가 살았습니다."), "ko");
        assert_eq!(detect_lang(""), "en");
    }

    #[test]
    fn imports_reads_and_resumes() {
        let path = temp_db("library");
        let mut conn = db::open(&path).unwrap();
        let mut book = Fake(vec![
            (None, vec!["Lời nói đầu của tác giả."]),
            (
                Some("Chương 1"),
                vec!["Chương 1", "Ngày xưa có một cô bé.", "Hết chương."],
            ),
            (Some("Chương 2"), vec![]),
        ]);
        let mut seen = Vec::new();
        let id = import(
            &mut conn,
            BookFormat::Epub,
            &mut book,
            "content://x/document/T%E1%BA%A5m%20C%C3%A1m.epub",
            |d, t| seen.push((d, t)),
        )
        .unwrap();
        assert_eq!(seen, vec![(1, 3), (2, 3), (3, 3)]);

        let listed = list(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Tấm Cám");
        assert_eq!(listed[0].lang, "vi");
        assert_eq!(listed[0].format.as_deref(), Some("epub"));
        assert_eq!(listed[0].chapter_count, 3);
        assert_eq!(listed[0].last_read, None);

        let (format, mut db_book) = DbBook::open(&path, id, None).unwrap();
        assert_eq!(format, BookFormat::Epub);
        let labels: Vec<_> = db_book.sections().into_iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            vec![None, Some("Chương 1".into()), Some("Chương 2".into())]
        );
        assert_eq!(
            db_book.load(1).unwrap().paragraphs,
            vec!["Chương 1", "Ngày xưa có một cô bé.", "Hết chương."]
        );
        assert!(db_book.load(2).unwrap().paragraphs.is_empty());
        assert!(matches!(db_book.load(3), Err(BookError::NoSection(3))));

        save_progress(&conn, id, 1, 0.4).unwrap();
        save_progress(&conn, id, 2, 1.5).unwrap();
        let last = get(&conn, id).unwrap().unwrap().last_read.unwrap();
        assert_eq!((last.index, last.progress), (2, 1.0));

        delete(&conn, id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
        let left: i64 = conn
            .query_row("select (select count(*) from chapters) + (select count(*) from authors) + (select count(*) from reading_history)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn imports_a_txt_file() {
        let dir = crate::book::test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("truyen-{}.txt", std::process::id()));
        let para = "Ngày xưa có một cô bé tên là Tấm, sống cùng dì ghẻ và Cám.\n";
        std::fs::write(&file, para.repeat(3000)).unwrap(); // ~190 KB: several sections
        let ctx = crate::book::OpenCtx {
            name: file.display().to_string(),
            path: Some(file.clone()),
            cache_dir: dir.clone(),
            pdfium_dirs: vec![],
        };
        let (format, mut source) =
            crate::book::open_source(std::fs::File::open(&file).unwrap(), &ctx).unwrap();
        let sections = source.sections().len();
        assert!(sections > 1);

        let path = temp_db("txt");
        let mut conn = db::open(&path).unwrap();
        let id = import(&mut conn, format, source.as_mut(), &ctx.name, |_, _| {}).unwrap();
        let book = get(&conn, id).unwrap().unwrap();
        assert_eq!(
            (book.lang.as_str(), book.chapter_count),
            ("vi", sections as i64)
        );
        assert!(book.title.starts_with("truyen-"));

        let (_, mut db_book) = DbBook::open(&path, id, None).unwrap();
        assert_eq!(
            db_book.load(0).unwrap().paragraphs,
            source.load(0).unwrap().paragraphs
        );
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn imports_pictures_into_the_database() {
        let path = temp_db("pictures");
        let mut conn = db::open(&path).unwrap();
        let mut book = Fake(vec![
            (None, vec!["Mở đầu", "\u{FFFC}a.png", "\u{FFFC}missing.png"]),
            (None, vec!["\u{FFFC}a.png", "Hết"]),
        ]);
        let id = import(&mut conn, BookFormat::Epub, &mut book, "a.epub", |_, _| {}).unwrap();
        let (_, mut db_book) = DbBook::open(&path, id, None).unwrap();
        assert_eq!(db_book.load(0).unwrap().paragraphs[1], "\u{FFFC}a.png");
        assert_eq!(db_book.image("a.png").unwrap().mime, "image/png");
        assert!(db_book.image("missing.png").is_err());
        assert!(!db_book.page_images());
        delete(&conn, id).unwrap();
        let left: i64 = conn
            .query_row("select count(*) from chapter_images", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn migrations_run_once() {
        let path = temp_db("migrate");
        drop(db::open(&path).unwrap());
        let conn = db::open(&path).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        let langs: i64 = conn
            .query_row("select count(*) from languages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(langs, 4);
    }
}
