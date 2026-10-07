//! Offline book reading that keeps only the part being read in memory.
//!
//! A book is split into sections (EPUB: one spine item, PDF/DjVu: a few pages, TXT:
//! ~64 KB, CHM: one page). The file stays on disk; a section is read from it only
//! when the reader reaches it or is about to, and dropped again once the reader has
//! moved away. Formats whose text is compressed or wrapped in markup (MOBI, FB2,
//! DOCX, …) are first extracted, streaming, to a plain text file in the cache folder.

mod chm;
pub mod commands;
mod djvu;
mod epub;
mod extract;
mod html;
mod markdown;
mod mobi;
mod pdf;
mod rtf;
mod text;
mod txt;
mod window;
mod xmldoc;

use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use window::WindowCache;

#[cfg(test)]
pub(crate) use extract::test_dir;

/// Sections kept before the current one.
pub const KEEP_BEHIND: usize = 1;
/// Sections loaded ahead of the current one.
pub const KEEP_AHEAD: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum BookError {
    #[error("{0} files are not supported yet")]
    NotYet(&'static str),
    #[error("cannot open file: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot read book: {0}")]
    Parse(String),
    #[error("PDF support is unavailable: {0}")]
    PdfUnavailable(String),
    #[error("section {0} does not exist")]
    NoSection(usize),
    #[error("book {0} is not open")]
    NotOpen(u32),
    #[error("library error: {0}")]
    Db(#[from] rusqlite::Error),
}

impl Serialize for BookError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BookFormat {
    Epub,
    Pdf,
    Txt,
    Html,
    Mht,
    Markdown,
    Fb2,
    Docx,
    Odt,
    Rtf,
    Mobi,
    Chm,
    Djvu,
}

#[derive(Clone, Debug, Serialize)]
pub struct SectionMeta {
    /// Title from the table of contents, when the book has one for this section.
    pub label: Option<String>,
    /// First and last page (1-based) for PDF sections.
    pub pages: Option<[u32; 2]>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Section {
    pub index: usize,
    pub paragraphs: Vec<String>,
}

/// Reads sections of one book straight from its file.
pub trait BookSource: Send {
    fn title(&self) -> Option<String>;
    fn sections(&self) -> Vec<SectionMeta>;
    fn load(&mut self, index: usize) -> Result<Section, BookError>;
}

#[derive(Clone, Debug, Serialize)]
pub struct BookInfo {
    pub id: u32,
    pub format: BookFormat,
    pub title: Option<String>,
    pub sections: Vec<SectionMeta>,
}

/// An open book: the file handle plus the few sections around the reading position.
pub struct OpenBook {
    pub info: BookInfo,
    source: Mutex<Box<dyn BookSource>>,
    cache: Mutex<WindowCache<Section>>,
    center: AtomicUsize,
}

impl OpenBook {
    pub fn new(id: u32, format: BookFormat, source: Box<dyn BookSource>) -> Self {
        let info = BookInfo {
            id,
            format,
            title: source.title(),
            sections: source.sections(),
        };
        Self {
            info,
            source: Mutex::new(source),
            cache: Mutex::new(WindowCache::new(KEEP_BEHIND, KEEP_AHEAD)),
            center: AtomicUsize::new(0),
        }
    }

    fn len(&self) -> usize {
        self.info.sections.len()
    }

    /// Returns section `index`, moves the reading window there and starts loading the
    /// neighbours in the background.
    pub fn section(self: &Arc<Self>, index: usize) -> Result<Arc<Section>, BookError> {
        if index >= self.len() {
            return Err(BookError::NoSection(index));
        }
        self.center.store(index, Ordering::SeqCst);
        self.cache.lock().unwrap().retain_around(index, self.len());
        let section = self.get_or_load(index)?;

        let book = Arc::clone(self);
        std::thread::spawn(move || book.prefetch(index));
        Ok(section)
    }

    fn prefetch(&self, center: usize) {
        let order = self
            .cache
            .lock()
            .unwrap()
            .prefetch_order(center, self.len());
        for i in order {
            // The reader moved on; the new request starts its own prefetch.
            if self.center.load(Ordering::SeqCst) != center {
                return;
            }
            // A broken neighbour only matters once the reader opens it.
            let _ = self.get_or_load(i);
        }
    }

    fn get_or_load(&self, index: usize) -> Result<Arc<Section>, BookError> {
        if let Some(s) = self.cache.lock().unwrap().get(index) {
            return Ok(s);
        }
        let mut source = self.source.lock().unwrap();
        // Another thread may have loaded it while we waited for the file.
        if let Some(s) = self.cache.lock().unwrap().get(index) {
            return Ok(s);
        }
        let section = Arc::new(source.load(index)?);
        drop(source);

        let mut cache = self.cache.lock().unwrap();
        let center = self.center.load(Ordering::SeqCst);
        if cache.window(center, self.len()).contains(&index) {
            cache.insert(index, Arc::clone(&section));
        }
        Ok(section)
    }

    #[cfg(test)]
    fn cached(&self) -> Vec<usize> {
        self.cache.lock().unwrap().cached()
    }
}

/// Where a book comes from and where it may put cache files.
pub struct OpenCtx {
    /// The path or URI as picked, used for the extension and the fallback title.
    pub name: String,
    /// A real file path, when there is one (not for Android content URIs).
    pub path: Option<std::path::PathBuf>,
    pub cache_dir: std::path::PathBuf,
    pub pdfium_dirs: Vec<std::path::PathBuf>,
}

impl OpenCtx {
    /// File name without extension, for books that carry no title.
    fn file_title(&self) -> Option<String> {
        if self.name.starts_with("content://") {
            return None;
        }
        let file = self.name.rsplit(['/', '\\']).next()?;
        let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
        (!stem.is_empty()).then(|| stem.to_string())
    }

    fn extension(&self) -> String {
        let file = self.name.rsplit(['/', '\\']).next().unwrap_or_default();
        file.rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default()
    }
}

/// Picks the format from the extension, or from the first bytes when there is none.
pub fn detect(ext: &str, head: &[u8]) -> Result<BookFormat, BookError> {
    use BookFormat::*;
    let by_ext = match ext {
        "epub" => Some(Epub),
        "pdf" => Some(Pdf),
        "txt" | "text" => Some(Txt),
        "html" | "htm" | "xhtml" => Some(Html),
        "mht" | "mhtml" => Some(Mht),
        "md" | "markdown" => Some(Markdown),
        "fb2" => Some(Fb2),
        "docx" => Some(Docx),
        "odt" => Some(Odt),
        "rtf" => Some(Rtf),
        "mobi" | "azw" | "azw3" | "prc" | "pdb" => Some(Mobi),
        "chm" => Some(Chm),
        "djvu" | "djv" => Some(Djvu),
        "umd" => return Err(BookError::NotYet("UMD")),
        _ => None,
    };
    if let Some(f) = by_ext {
        return Ok(f);
    }
    let lower = String::from_utf8_lossy(&head[..head.len().min(2048)]).to_ascii_lowercase();
    Ok(if head.starts_with(b"%PDF-") {
        Pdf
    } else if head.starts_with(b"PK") {
        zip_kind(head)
    } else if head
        .get(60..68)
        .is_some_and(|k| k == b"BOOKMOBI" || k == b"TEXtREAd")
    {
        Mobi
    } else if head.starts_with(b"ITSF") {
        Chm
    } else if head.starts_with(b"AT&TFORM") {
        Djvu
    } else if head.starts_with(&[0x89, 0x9B, 0x9A, 0xDE]) {
        return Err(BookError::NotYet("UMD"));
    } else if head.starts_with(b"{\\rtf") {
        Rtf
    } else if lower.contains("<fictionbook") {
        Fb2
    } else if lower.contains("multipart/related") || lower.starts_with("mime-version:") {
        Mht
    } else if lower.contains("<html") || lower.contains("<!doctype html") {
        Html
    } else {
        Txt
    })
}

/// EPUB and ODT start with a stored `mimetype` entry; DOCX has `word/`.
fn zip_kind(head: &[u8]) -> BookFormat {
    let name_len = head
        .get(26..28)
        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let extra_len = head
        .get(28..30)
        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as usize);
    if head.get(30..30 + name_len) == Some(b"mimetype") {
        let at = 30 + name_len + extra_len;
        if head
            .get(at..)
            .is_some_and(|m| m.starts_with(b"application/vnd.oasis.opendocument.text"))
        {
            return BookFormat::Odt;
        }
        return BookFormat::Epub;
    }
    if head.windows(5).any(|w| w == b"word/")
        || head.windows(19).any(|w| w == b"[Content_Types].xml")
    {
        return BookFormat::Docx;
    }
    BookFormat::Epub
}

/// Opens a book in any supported format.
pub fn open_source(
    file: std::fs::File,
    ctx: &OpenCtx,
) -> Result<(BookFormat, Box<dyn BookSource>), BookError> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    let mut file = file;
    let mut head = vec![0u8; 4096];
    let mut n = 0;
    while n < head.len() {
        match file.read(&mut head[n..])? {
            0 => break,
            k => n += k,
        }
    }
    head.truncate(n);
    file.seek(SeekFrom::Start(0))?;

    let format = detect(&ctx.extension(), &head)?;
    let title = ctx.file_title();
    let cache = &ctx.cache_dir;
    // CHM and DjVu readers need a file path; content URIs are copied to the cache first.
    let local = |file: std::fs::File,
                 ext: &str|
     -> Result<(std::path::PathBuf, Option<extract::TempCopy>), BookError> {
        match &ctx.path {
            Some(p) => Ok((p.clone(), None)),
            None => {
                let copy = extract::TempCopy::of(file, cache, ext)?;
                Ok((copy.0.clone(), Some(copy)))
            }
        }
    };
    let source: Box<dyn BookSource> = match format {
        BookFormat::Pdf => Box::new(pdf::PdfBook::open(file, &ctx.pdfium_dirs)?),
        BookFormat::Epub => Box::new(epub::EpubBook::open(BufReader::new(file))?),
        BookFormat::Txt => Box::new(txt::TxtBook::open(file, title)?),
        BookFormat::Html => Box::new(html::extract_html(file, cache, title)?),
        BookFormat::Mht => Box::new(html::extract_mht(file, cache, title)?),
        BookFormat::Markdown => Box::new(markdown::extract_markdown(file, cache, title)?),
        BookFormat::Fb2 => Box::new(xmldoc::extract_fb2(file, cache, title)?),
        BookFormat::Docx => Box::new(xmldoc::extract_docx(BufReader::new(file), cache, title)?),
        BookFormat::Odt => Box::new(xmldoc::extract_odt(BufReader::new(file), cache, title)?),
        BookFormat::Rtf => Box::new(rtf::extract_rtf(file, cache, title)?),
        BookFormat::Mobi => Box::new(mobi::extract_mobi(BufReader::new(file), cache, title)?),
        BookFormat::Chm => {
            let (path, copy) = local(file, "chm")?;
            Box::new(chm::ChmBook::open(&path, copy, title)?)
        }
        BookFormat::Djvu => {
            let (path, copy) = local(file, "djvu")?;
            Box::new(djvu::DjvuBook::open(&path, copy, title)?)
        }
    };
    Ok((format, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    struct Counting {
        loads: Arc<AtomicUsize>,
    }

    impl BookSource for Counting {
        fn title(&self) -> Option<String> {
            None
        }
        fn sections(&self) -> Vec<SectionMeta> {
            (0..10)
                .map(|_| SectionMeta {
                    label: None,
                    pages: None,
                })
                .collect()
        }
        fn load(&mut self, index: usize) -> Result<Section, BookError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(Section {
                index,
                paragraphs: vec![format!("part {index}")],
            })
        }
    }

    fn wait_for(book: &OpenBook, want: Vec<usize>) {
        let start = Instant::now();
        while book.cached() != want {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "cached {:?}, want {want:?}",
                book.cached()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn keeps_only_the_window_around_the_reader() {
        let loads = Arc::new(AtomicUsize::new(0));
        let book = Arc::new(OpenBook::new(
            1,
            BookFormat::Epub,
            Box::new(Counting {
                loads: loads.clone(),
            }),
        ));

        // Reading part 2 (index 1): parts 1-4 end up in memory, nothing else.
        assert_eq!(book.section(1).unwrap().paragraphs, vec!["part 1"]);
        wait_for(&book, vec![0, 1, 2, 3]);

        // Moving to the next part reuses what was prefetched and drops part 1.
        let before = loads.load(Ordering::SeqCst);
        book.section(2).unwrap();
        wait_for(&book, vec![1, 2, 3, 4]);
        assert_eq!(
            loads.load(Ordering::SeqCst),
            before + 1,
            "only part 5 should be read from the file"
        );

        // Jumping far away drops everything from before.
        book.section(8).unwrap();
        wait_for(&book, vec![7, 8, 9]);
    }

    #[test]
    fn detects_format_by_extension_then_content() {
        use BookFormat::*;
        assert_eq!(detect("azw3", b"").unwrap(), Mobi);
        assert_eq!(detect("htm", b"").unwrap(), Html);
        assert!(matches!(detect("umd", b""), Err(BookError::NotYet("UMD"))));
        // Content URIs have no extension: sniff the first bytes.
        assert_eq!(detect("", b"%PDF-1.7").unwrap(), Pdf);
        assert_eq!(detect("", b"AT&TFORM\0\0").unwrap(), Djvu);
        assert_eq!(detect("", b"ITSF\x03").unwrap(), Chm);
        assert_eq!(detect("", b"{\\rtf1\\ansi").unwrap(), Rtf);
        assert_eq!(
            detect("", b"<?xml version=\"1.0\"?><FictionBook>").unwrap(),
            Fb2
        );
        assert_eq!(detect("", b"<!DOCTYPE html><html>").unwrap(), Html);
        assert_eq!(detect("", "Chương 1\nNội dung".as_bytes()).unwrap(), Txt);
        let mut mobi = vec![0u8; 78];
        mobi[60..68].copy_from_slice(b"BOOKMOBI");
        assert_eq!(detect("", &mobi).unwrap(), Mobi);
        let mut odt = b"PK\x03\x04".to_vec();
        odt.resize(26, 0);
        odt.extend_from_slice(&[8, 0, 0, 0]);
        odt.extend_from_slice(b"mimetypeapplication/vnd.oasis.opendocument.text");
        assert_eq!(detect("", &odt).unwrap(), Odt);
    }

    #[test]
    fn rejects_missing_section() {
        let book = Arc::new(OpenBook::new(
            1,
            BookFormat::Epub,
            Box::new(Counting {
                loads: Default::default(),
            }),
        ));
        assert!(matches!(book.section(10), Err(BookError::NoSection(10))));
    }
}
