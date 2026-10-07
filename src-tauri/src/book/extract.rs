//! For formats whose text is compressed or wrapped in markup (MOBI, FB2, DOCX, …):
//! the text is extracted once, streaming, into a plain UTF-8 file in the cache
//! folder (one paragraph per line), and the reader then loads sections of that
//! file on demand like any other book.

use super::txt::SECTION_BYTES;
use super::{BookError, BookSource, Section, SectionMeta};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct Range {
    start: u64,
    len: u64,
    label: Option<String>,
}

/// Collects extracted paragraphs into the cache file and decides where sections end:
/// at each chapter, and inside long chapters after ~64 KB.
pub struct Sink {
    path: PathBuf,
    out: BufWriter<File>,
    pos: u64,
    start: u64,
    label: Option<String>,
    sections: Vec<Range>,
    pub title: Option<String>,
}

impl Sink {
    pub fn new(cache_dir: &Path) -> Result<Self, BookError> {
        std::fs::create_dir_all(cache_dir)?;
        let n = NEXT_FILE.fetch_add(1, Ordering::SeqCst);
        let path = cache_dir.join(format!("book-{}-{n}.txt", std::process::id()));
        let out = BufWriter::new(File::create(&path)?);
        Ok(Self {
            path,
            out,
            pos: 0,
            start: 0,
            label: None,
            sections: Vec::new(),
            title: None,
        })
    }

    /// Adds a paragraph; whitespace (including line breaks) is collapsed.
    pub fn paragraph(&mut self, text: &str) -> Result<(), BookError> {
        let mut line = String::with_capacity(text.len() + 1);
        for word in text.split_whitespace() {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if line.is_empty() {
            return Ok(());
        }
        line.push('\n');
        self.out.write_all(line.as_bytes())?;
        self.pos += line.len() as u64;
        if self.pos - self.start >= SECTION_BYTES {
            self.close_section();
        }
        Ok(())
    }

    /// Starts a new section named `label`; the label is also kept as its first paragraph.
    pub fn chapter(&mut self, label: &str) -> Result<(), BookError> {
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        if label.is_empty() {
            return Ok(());
        }
        self.close_section();
        self.label = Some(label.clone());
        self.paragraph(&label)
    }

    fn close_section(&mut self) {
        if self.pos > self.start {
            self.sections.push(Range {
                start: self.start,
                len: self.pos - self.start,
                label: self.label.take(),
            });
            self.start = self.pos;
        }
    }

    pub fn finish(mut self) -> Result<ExtractedBook, BookError> {
        self.close_section();
        if self.sections.is_empty() {
            self.sections.push(Range {
                start: 0,
                len: 0,
                label: None,
            });
        }
        self.out.flush()?;
        drop(self.out);
        let file = File::open(&self.path)?;
        Ok(ExtractedBook {
            path: self.path,
            file,
            sections: self.sections,
            title: self.title,
        })
    }
}

/// A book backed by extracted text in the cache folder; the file is deleted on close.
pub struct ExtractedBook {
    path: PathBuf,
    file: File,
    sections: Vec<Range>,
    title: Option<String>,
}

impl Drop for ExtractedBook {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl BookSource for ExtractedBook {
    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn sections(&self) -> Vec<SectionMeta> {
        self.sections
            .iter()
            .map(|r| SectionMeta {
                label: r.label.clone(),
                pages: None,
            })
            .collect()
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let r = self
            .sections
            .get(index)
            .ok_or(BookError::NoSection(index))?;
        self.file.seek(SeekFrom::Start(r.start))?;
        let mut text = String::with_capacity(r.len as usize);
        (&mut self.file).take(r.len).read_to_string(&mut text)?;
        Ok(Section {
            index,
            paragraphs: text.lines().map(String::from).collect(),
        })
    }
}

/// Writes HTML blocks into the sink: headings start chapters.
pub fn html_block(sink: &mut Sink, block: super::text::Block) -> Result<(), BookError> {
    match block {
        super::text::Block::Heading(t) => sink.chapter(&t),
        super::text::Block::Text(t) => sink.paragraph(&t),
    }
}

/// A copy of the opened file in the cache folder, for readers that need a real path
/// (Android hands out `content://` URIs). Deleted on drop.
pub struct TempCopy(pub PathBuf);

impl TempCopy {
    pub fn of(mut file: File, cache_dir: &Path, ext: &str) -> Result<Self, BookError> {
        std::fs::create_dir_all(cache_dir)?;
        let n = NEXT_FILE.fetch_add(1, Ordering::SeqCst);
        let path = cache_dir.join(format!("copy-{}-{n}.{ext}", std::process::id()));
        let guard = Self(path);
        std::io::copy(&mut file, &mut BufWriter::new(File::create(&guard.0)?))?;
        Ok(guard)
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
pub(crate) fn test_dir() -> PathBuf {
    std::env::temp_dir().join("bennovel-tests")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_and_long_text_become_sections() {
        let mut sink = Sink::new(&test_dir()).unwrap();
        sink.paragraph("Lời nói đầu").unwrap();
        sink.chapter("Chương 1").unwrap();
        let long = "chữ ".repeat(5000); // ~25 KB per paragraph
        for _ in 0..4 {
            sink.paragraph(&long).unwrap();
        }
        sink.chapter("Chương 2").unwrap();
        sink.paragraph("Hết.").unwrap();
        let mut book = sink.finish().unwrap();
        let path = book.path.clone();

        let labels: Vec<_> = book.sections().into_iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            vec![None, Some("Chương 1".into()), None, Some("Chương 2".into())]
        );
        assert_eq!(book.load(0).unwrap().paragraphs, vec!["Lời nói đầu"]);
        assert_eq!(book.load(3).unwrap().paragraphs, vec!["Chương 2", "Hết."]);
        assert_eq!(book.load(1).unwrap().paragraphs.len(), 4); // heading + 3 paragraphs reach 64 KB
        drop(book);
        assert!(!path.exists(), "cache file is removed on close");
    }
}
