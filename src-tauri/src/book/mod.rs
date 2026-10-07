//! Offline EPUB/PDF reading that keeps only the part being read in memory.
//!
//! A book is split into sections (EPUB: one spine item, PDF: a few pages). The file
//! stays on disk; a section is read from it only when the reader reaches it or is
//! about to, and dropped again once the reader has moved away.

pub mod commands;
mod epub;
mod pdf;
mod text;
mod window;

use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use window::WindowCache;

/// Sections kept before the current one.
pub const KEEP_BEHIND: usize = 1;
/// Sections loaded ahead of the current one.
pub const KEEP_AHEAD: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum BookError {
    #[error("unsupported file type")]
    Unsupported,
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
}

impl Serialize for BookError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BookFormat {
    Epub,
    Pdf,
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

/// Opens an EPUB or PDF by its extension (or, for content URIs, by its leading bytes).
pub fn open_source(
    file: std::fs::File,
    name_hint: &str,
    pdfium_dirs: &[std::path::PathBuf],
) -> Result<(BookFormat, Box<dyn BookSource>), BookError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    let mut magic = [0u8; 5];
    let n = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    let lower = name_hint.to_ascii_lowercase();

    if lower.ends_with(".pdf") || magic[..n].starts_with(b"%PDF-") {
        Ok((
            BookFormat::Pdf,
            Box::new(pdf::PdfBook::open(file, pdfium_dirs)?),
        ))
    } else if lower.ends_with(".epub") || magic[..n].starts_with(b"PK") {
        Ok((
            BookFormat::Epub,
            Box::new(epub::EpubBook::open(std::io::BufReader::new(file))?),
        ))
    } else {
        Err(BookError::Unsupported)
    }
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
