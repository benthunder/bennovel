//! For formats whose text is compressed or wrapped in markup (MOBI, FB2, DOCX, …):
//! the text is extracted once, streaming, into a plain UTF-8 file in the cache
//! folder (one paragraph per line), and the reader then loads sections of that
//! file on demand like any other book.

use super::image::{self, Image};
use super::txt::SECTION_BYTES;
use super::{BookError, BookSource, Section, SectionMeta};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// Pictures larger than this are not copied out of a book.
pub const MAX_IMAGE: u64 = 32 * 1024 * 1024;

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
    /// Pictures saved next to the text file, by key.
    images: HashMap<String, PathBuf>,
    /// Picture keys used by image paragraphs, in order.
    referenced: Vec<String>,
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
            images: HashMap::new(),
            referenced: Vec::new(),
            title: None,
        })
    }

    /// Adds an image paragraph for picture `key` (saved now or later with `save_image`).
    pub fn image(&mut self, key: &str) -> Result<(), BookError> {
        if !self.referenced.iter().any(|k| k == key) {
            self.referenced.push(key.to_string());
        }
        let mut line = image::paragraph(key);
        line.push('\n');
        self.out.write_all(line.as_bytes())?;
        self.pos += line.len() as u64;
        Ok(())
    }

    /// Saves picture bytes under `key`, in a file next to the extracted text.
    pub fn save_image(&mut self, key: &str, bytes: &[u8]) -> Result<(), BookError> {
        if bytes.is_empty() || self.images.contains_key(key) {
            return Ok(());
        }
        let file = PathBuf::from(format!("{}.img{}", self.path.display(), self.images.len()));
        std::fs::write(&file, bytes)?;
        self.images.insert(key.to_string(), file);
        Ok(())
    }

    /// Saves a picture and adds its paragraph.
    pub fn picture(&mut self, key: &str, bytes: &[u8]) -> Result<(), BookError> {
        self.save_image(key, bytes)?;
        self.image(key)
    }

    /// Keys of image paragraphs whose picture has not been saved.
    pub fn missing_images(&self) -> Vec<String> {
        self.referenced
            .iter()
            .filter(|k| !self.images.contains_key(*k))
            .cloned()
            .collect()
    }

    /// Lets `key` name the same picture as the saved `existing` key.
    pub fn alias(&mut self, key: &str, existing: &str) -> bool {
        match self.images.get(existing).cloned() {
            Some(file) => {
                self.images.insert(key.to_string(), file);
                true
            }
            None => false,
        }
    }

    /// Saves pictures linked by relative path from a document in folder `base`.
    pub fn load_linked_images(&mut self, base: &Path) -> Result<(), BookError> {
        for key in self.missing_images() {
            if key.contains("://") {
                continue;
            }
            let rel = image::percent_decode(key.split(['#', '?']).next().unwrap_or_default());
            let path = base.join(rel.trim_start_matches('/'));
            let small = std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() < MAX_IMAGE);
            if small {
                let bytes = std::fs::read(&path)?;
                self.save_image(&key, &bytes)?;
            }
        }
        Ok(())
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
            images: self.images,
            title: self.title,
        })
    }
}

/// A book backed by extracted text in the cache folder; the file is deleted on close.
pub struct ExtractedBook {
    path: PathBuf,
    file: File,
    sections: Vec<Range>,
    images: HashMap<String, PathBuf>,
    title: Option<String>,
}

impl Drop for ExtractedBook {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        for f in self.images.values() {
            let _ = std::fs::remove_file(f);
        }
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

    fn image(&mut self, key: &str) -> Result<Image, BookError> {
        let file = self
            .images
            .get(key)
            .ok_or_else(|| BookError::NoImage(key.to_string()))?;
        Ok(Image::sniff(std::fs::read(file)?))
    }
}

/// Writes HTML blocks into the sink: headings start chapters.
pub fn html_block(sink: &mut Sink, block: super::text::Block) -> Result<(), BookError> {
    match block {
        super::text::Block::Heading(t) => sink.chapter(&t),
        super::text::Block::Text(t) => sink.paragraph(&t),
        super::text::Block::Image(src) => match image::data_uri(&src) {
            // Inline pictures are saved right away; linked ones are left to the caller.
            Some(bytes) => {
                let key = format!("data{}", sink.images.len());
                sink.picture(&key, &bytes)
            }
            None => sink.image(&src),
        },
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
