//! Importing a folder of chapter files as one book: every file becomes one chapter,
//! in the order of the file paths (subfolders and names compared with their digit
//! runs as numbers, so "Chương 2" comes before "Chương 10"). Nothing is guessed from
//! the text: the chapter number is the file's place in that order and its title is
//! the file name.

use crate::book::image::{self, Image};
use crate::book::{BookError, BookFormat, BookSource, Section, SectionMeta};
use std::cmp::Ordering;

/// File extensions picked up from a folder (the formats the reader opens).
pub const BOOK_EXTENSIONS: &[&str] = &[
    "epub", "pdf", "txt", "text", "mobi", "azw3", "azw", "prc", "pdb", "fb2", "djvu", "djv", "chm",
    "docx", "odt", "rtf", "html", "htm", "xhtml", "mht", "mhtml", "md", "markdown",
];

/// Whether a file name has one of the book extensions.
pub fn is_book_file(name: &str) -> bool {
    let name = name.to_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or_default();
    BOOK_EXTENSIONS.contains(&ext)
}

/// Collects the book files in `dir` and, recursively, its subfolders. Hidden entries
/// and symbolic links (which could loop) are skipped.
pub fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(&entry.path(), out)?;
        } else if kind.is_file() && is_book_file(&name) {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// Opens file `i` (in the order given to `FolderBook::new`) as a book.
pub type Opener =
    Box<dyn FnMut(usize) -> Result<(BookFormat, Box<dyn BookSource>), BookError> + Send>;

/// One chapter file.
struct Chapter {
    /// Index for the opener.
    file: usize,
    /// The folder part of the file's path: subfolders are read in order
    /// ("Quyển 1/…" before "Quyển 2/…"), files right in the folder first.
    dir: String,
    /// File name without extension: the chapter title, and the order in its folder.
    name: String,
}

/// A folder of chapter files read as one book. Only the file being read is open.
pub struct FolderBook {
    title: String,
    chapters: Vec<Chapter>,
    open: Opener,
    /// The file opened last, by chapter index; pictures are read from it.
    current: Option<(usize, Box<dyn BookSource>)>,
}

impl FolderBook {
    /// `names` are the files' paths or URIs, `title` the book title (the folder name).
    /// Returns the format of the first chapter with the book.
    pub fn new(
        title: String,
        names: &[String],
        mut open: Opener,
    ) -> Result<(BookFormat, Self), BookError> {
        if names.is_empty() {
            return Err(BookError::Parse("no book files in this folder".into()));
        }
        let mut chapters: Vec<Chapter> = names
            .iter()
            .enumerate()
            .map(|(file, name)| {
                // Android document URIs carry the path encoded ("primary%3ANovels%2FC1.txt").
                let path = super::urlish_decode(name);
                let dir = match path.rsplit_once(['/', '\\']) {
                    Some((dir, _)) => dir.to_string(),
                    None => String::new(),
                };
                Chapter {
                    file,
                    dir,
                    name: super::file_stem(name),
                }
            })
            .collect();
        chapters.sort_by(|a, b| {
            natural_cmp(&a.dir, &b.dir)
                .then_with(|| natural_cmp(&a.name, &b.name))
                .then(a.file.cmp(&b.file))
        });
        let (format, first) = open(chapters[0].file)?;
        Ok((
            format,
            Self {
                title,
                chapters,
                open,
                current: Some((0, first)),
            },
        ))
    }

    fn source(&mut self, index: usize) -> Result<&mut Box<dyn BookSource>, BookError> {
        if self.current.as_ref().map(|(i, _)| *i) != Some(index) {
            let file = self
                .chapters
                .get(index)
                .ok_or(BookError::NoSection(index))?
                .file;
            self.current = None; // close the previous file first
            let (_, source) = (self.open)(file)?;
            self.current = Some((index, source));
        }
        Ok(&mut self.current.as_mut().unwrap().1)
    }
}

impl BookSource for FolderBook {
    fn title(&self) -> Option<String> {
        Some(self.title.clone())
    }

    fn sections(&self) -> Vec<SectionMeta> {
        self.chapters
            .iter()
            .enumerate()
            .map(|(i, c)| SectionMeta {
                label: title_of(&c.name),
                pages: None,
                number: Some(i as f64 + 1.0),
            })
            .collect()
    }

    /// The whole file as one chapter; picture keys get the chapter's index in front
    /// (`3/cover.jpg`), so files using the same picture names don't clash.
    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let source = self.source(index)?;
        let mut paragraphs = Vec::new();
        for i in 0..source.sections().len() {
            for p in source.load(i)?.paragraphs {
                match image::key_of(&p) {
                    Some(key) => paragraphs.push(format!("{}{index}/{key}", image::IMAGE_MARK)),
                    None => paragraphs.push(p),
                }
            }
        }
        Ok(Section { index, paragraphs })
    }

    fn section_title(&self, index: usize) -> Option<String> {
        title_of(&self.chapters.get(index)?.name)
    }

    fn image(&mut self, key: &str) -> Result<Image, BookError> {
        let (index, key) = key
            .split_once('/')
            .and_then(|(i, k)| Some((i.parse::<usize>().ok()?, k)))
            .ok_or_else(|| BookError::NoImage(key.to_string()))?;
        self.source(index)?.image(key)
    }
}

/// The file name as the chapter title; none for Android's opaque names ("msf:1234").
fn title_of(name: &str) -> Option<String> {
    let name = name.trim();
    let opaque = name.split_once(':').is_some_and(|(k, v)| {
        !k.is_empty()
            && k.chars().all(|c| c.is_ascii_alphanumeric())
            && v.chars().all(|c| c.is_ascii_digit())
    });
    (!name.is_empty() && !opaque).then(|| name.to_string())
}

/// The folder holding every file, when the names tell (paths, and Android document
/// URIs like `…/document/primary%3ANovels%2FTruyen%2FChuong%201.txt`).
pub fn common_parent(names: &[String]) -> Option<String> {
    let parent = |name: &str| -> Option<String> {
        let name = match name.strip_prefix("content://") {
            Some(uri) => super::urlish_decode(uri.rsplit_once("/document/")?.1),
            None => name.to_string(),
        };
        let mut parts = name.rsplit(['/', '\\']);
        parts.next()?;
        let dir = parts.next()?.rsplit(':').next()?.trim().to_string();
        (!dir.is_empty()).then_some(dir)
    };
    let first = parent(names.first()?)?;
    names[1..]
        .iter()
        .all(|n| parent(n).as_deref() == Some(&first))
        .then_some(first)
}

/// A title from what the file names share ("Đấu Phá - Chương 1", "Đấu Phá - Chương 2"
/// → "Đấu Phá"), or else the first file's name.
pub fn common_title(names: &[String]) -> String {
    let stems: Vec<String> = names.iter().map(|n| super::file_stem(n)).collect();
    let first = stems.first().cloned().unwrap_or_default();
    let mut prefix: Vec<char> = first.chars().collect();
    for s in &stems[1..] {
        let n = prefix
            .iter()
            .zip(s.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(n);
    }
    let prefix: String = prefix.into_iter().collect();
    // Drop a trailing chapter word and separators: "Đấu Phá - Chương " → "Đấu Phá".
    let mut title = prefix
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string();
    loop {
        let trimmed = title.trim_end_matches([' ', '-', '_', '.', ':', '#', '(', '[']);
        let lower = trimmed.to_lowercase();
        let word = CHAPTER_WORDS.iter().flat_map(|w| w.iter()).find(|w| {
            lower.ends_with(*w) && !lower[..lower.len() - w.len()].ends_with(char::is_alphanumeric)
        });
        match word {
            Some(w) if trimmed.is_char_boundary(trimmed.len() - w.len()) => {
                title = trimmed[..trimmed.len() - w.len()].to_string()
            }
            _ => {
                title = trimmed.to_string();
                break;
            }
        }
    }
    if title.chars().filter(|c| c.is_alphanumeric()).count() >= 2 {
        title
    } else {
        first
    }
}

/// Words that come before a chapter number in file names, dropped from the end of
/// the book title the names share ("Đấu Phá - Chương " → "Đấu Phá").
const CHAPTER_WORDS: &[&[&str]] = &[
    &[
        "chương", "chuong", "chapter", "chap", "ch", "hồi", "hoi", "c",
    ],
    &["tập", "tap", "episode", "ep", "phần", "phan", "part"],
];

/// Compares names with their digit runs as numbers ("file 2" before "file 10").
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.as_str(), b.as_str());
    loop {
        match (x.chars().next(), y.chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let n = x.bytes().take_while(u8::is_ascii_digit).count();
                let m = y.bytes().take_while(u8::is_ascii_digit).count();
                let (p, q) = (
                    x[..n].trim_start_matches('0'),
                    y[..m].trim_start_matches('0'),
                );
                let ord = p.len().cmp(&q.len()).then_with(|| p.cmp(q));
                if ord != Ordering::Equal {
                    return ord;
                }
                x = &x[n..];
                y = &y[m..];
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x = &x[c.len_utf8()..];
                y = &y[d.len_utf8()..];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("file 2", "file 10"), Ordering::Less);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
    }

    struct File(Vec<&'static str>, Option<&'static str>);
    impl BookSource for File {
        fn title(&self) -> Option<String> {
            self.1.map(String::from)
        }
        fn sections(&self) -> Vec<SectionMeta> {
            vec![
                SectionMeta {
                    label: None,
                    pages: None,
                    number: None,
                },
                SectionMeta {
                    label: None,
                    pages: None,
                    number: None,
                },
            ]
        }
        fn load(&mut self, index: usize) -> Result<Section, BookError> {
            Ok(Section {
                index,
                paragraphs: vec![self.0[index].to_string()],
            })
        }
        fn image(&mut self, key: &str) -> Result<Image, BookError> {
            match key {
                "a.png" => Ok(Image::sniff(b"\x89PNG".to_vec())),
                _ => Err(BookError::NoImage(key.into())),
            }
        }
    }

    pub(crate) fn fake_folder(
        names: &[&str],
        texts: Vec<[&'static str; 2]>,
    ) -> (BookFormat, FolderBook) {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        let open: Opener = Box::new(move |i| {
            Ok((
                BookFormat::Txt,
                Box::new(File(texts[i].to_vec(), None)) as Box<dyn BookSource>,
            ))
        });
        FolderBook::new("Truyện".into(), &names, open).unwrap()
    }

    #[test]
    fn orders_files_by_name_without_reading_them() {
        let (_, mut book) = fake_folder(
            &[
                "content://x/document/primary%3AT%2FCh%C6%B0%C6%A1ng%2010.txt",
                "content://x/document/primary%3AT%2FL%E1%BB%9Di%20b%E1%BA%A1t.txt",
                "content://x/document/primary%3AT%2FCh%C6%B0%C6%A1ng%202.txt",
                "content://x/document/primary%3AT%2FCh%C6%B0%C6%A1ng%201.txt",
            ],
            vec![
                // The text says otherwise; the file names decide.
                ["Chương 99: Mười", "hết"],
                ["Bạt", "hết"],
                ["Hai", "\u{FFFC}a.png"],
                ["Một", "hết"],
            ],
        );
        let sections = book.sections();
        let numbers: Vec<_> = sections.iter().map(|s| s.number).collect();
        assert_eq!(numbers, [Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        let labels: Vec<_> = sections.iter().map(|s| s.label.clone().unwrap()).collect();
        assert_eq!(labels, ["Chương 1", "Chương 2", "Chương 10", "Lời bạt"]);
        assert_eq!(book.load(0).unwrap().paragraphs, ["Một", "hết"]);
        assert_eq!(book.section_title(2).as_deref(), Some("Chương 10"));
        assert_eq!(book.load(1).unwrap().paragraphs, ["Hai", "\u{FFFC}1/a.png"]);
        assert_eq!(book.image("1/a.png").unwrap().mime, "image/png");
        assert!(book.image("a.png").is_err());
        assert!(book.load(5).is_err());
    }

    #[test]
    fn opaque_names_have_no_title() {
        assert_eq!(title_of("msf:55"), None);
        assert_eq!(title_of("Chương 3: Ba"), Some("Chương 3: Ba".into()));
    }

    #[test]
    fn titles_from_names() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            common_parent(&names(&[
                "/a/Đấu Phá/Chương 1.txt",
                "/a/Đấu Phá/Chương 2.txt"
            ])),
            Some("Đấu Phá".into())
        );
        assert_eq!(
            common_parent(&names(&[
                "content://com.android.externalstorage.documents/document/primary%3ANovels%2FTruyen%2FC1.txt",
                "content://com.android.externalstorage.documents/document/primary%3ANovels%2FTruyen%2FC2.txt",
            ])),
            Some("Truyen".into())
        );
        assert_eq!(
            common_parent(&names(&[
                "content://x/document/msf%3A1",
                "content://x/document/msf%3A2"
            ])),
            None
        );
        assert_eq!(common_parent(&names(&["/a/x/1.txt", "/a/y/2.txt"])), None);
        assert_eq!(
            common_title(&names(&[
                "/Đấu Phá - Chương 1.txt",
                "/Đấu Phá - Chương 12.txt"
            ])),
            "Đấu Phá"
        );
        assert_eq!(
            common_title(&names(&["/Chapter 1.txt", "/Chapter 2.txt"])),
            "Chapter 1"
        );
    }

    #[test]
    fn imports_a_folder_as_one_book() {
        let (format, mut book) = fake_folder(
            &["/t/Chương 2.txt", "/t/Chương 1.txt"],
            vec![
                ["Hai", "\u{FFFC}a.png"],
                ["Ngày xưa có một cô bé.", "Hết chương."],
            ],
        );
        let dir = crate::book::test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("folder-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut conn = super::super::db::open(&path).unwrap();
        let id = super::super::import(&mut conn, format, &mut book, "Truyện", |_, _| {}).unwrap();
        let saved = super::super::get(&conn, id).unwrap().unwrap();
        assert_eq!(
            (
                saved.title.as_str(),
                saved.chapter_count,
                saved.lang.as_str()
            ),
            ("Truyện", 2, "vi")
        );
        let (_, mut db_book) = super::super::DbBook::open(&path, id, None).unwrap();
        let sections = db_book.sections();
        assert_eq!(
            sections.iter().map(|s| s.number).collect::<Vec<_>>(),
            [Some(1.0), Some(2.0)]
        );
        // Titles are the file names, never guessed from the text.
        assert_eq!(sections[0].label.as_deref(), Some("Chương 1"));
        assert_eq!(sections[1].label.as_deref(), Some("Chương 2"));
        assert_eq!(
            db_book.load(0).unwrap().paragraphs,
            ["Ngày xưa có một cô bé.", "Hết chương."]
        );
        assert_eq!(db_book.image("1/a.png").unwrap().mime, "image/png");
    }

    #[test]
    fn walks_subfolders_in_order() {
        let root = crate::book::test_dir().join(format!("walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for f in [
            "Quyển 10/Chương 1.txt",
            "Quyển 2/Chương 2.txt",
            "Quyển 2/Chương 1.txt",
            "Mở đầu.txt",
            "Quyển 2/ghi chú.jpg",
            ".an/Chương 9.txt",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let mut found = Vec::new();
        walk(&root, &mut found).unwrap();
        let names: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(names.len(), 4);
        let texts = vec![["x", "y"]; 4];
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (_, book) = fake_folder(&refs, texts);
        let order: Vec<String> = book
            .chapters
            .iter()
            .map(|c| {
                names[c.file]
                    .strip_prefix(&root.display().to_string())
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            order,
            [
                "/Mở đầu.txt",
                "/Quyển 2/Chương 1.txt",
                "/Quyển 2/Chương 2.txt",
                "/Quyển 10/Chương 1.txt"
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
