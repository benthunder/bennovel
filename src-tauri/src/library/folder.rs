//! Importing a folder of chapter files as one book: every file becomes one chapter,
//! ordered by the chapter number found in its name ("Chương 12", "Chapter 12",
//! "第12章", or the first number), or in its first lines when the name has none.
//! Files with no number at all go last, by name.

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
    /// File name without extension, for ordering files that have no number.
    name: String,
    /// Chapter number from the file name (or the first lines when the name has none).
    number: Option<f64>,
    /// Chapter title from the first lines, known once the file has been read.
    title: Option<String>,
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
        let mut chapters = Vec::with_capacity(names.len());
        for (file, name) in names.iter().enumerate() {
            let stem = super::file_stem(name);
            // Android may hand out names like "msf:1234"; then the text has to tell.
            let opaque = name.starts_with("content://") && stem.contains(':');
            let dir = match name.rsplit_once(['/', '\\']) {
                Some((dir, _)) if !name.starts_with("content://") => dir.to_string(),
                _ => String::new(),
            };
            let mut chapter = Chapter {
                file,
                dir,
                number: (!opaque).then(|| chapter_number(&stem)).flatten(),
                name: stem,
                title: None,
            };
            if chapter.number.is_none() {
                if let Some(first) = first_line(open.as_mut(), file) {
                    chapter.number = chapter_number(&first);
                    if opaque || chapter.number.is_some() {
                        chapter.name = first;
                    }
                }
            }
            chapters.push(chapter);
        }
        chapters.sort_by(|a, b| {
            natural_cmp(&a.dir, &b.dir).then_with(|| match (a.number, b.number) {
                (Some(x), Some(y)) => x
                    .partial_cmp(&y)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| natural_cmp(&a.name, &b.name)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => natural_cmp(&a.name, &b.name),
            })
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

/// First non-empty line of a file's text, cut to a title's length.
fn first_line(
    open: &mut (dyn FnMut(usize) -> Result<(BookFormat, Box<dyn BookSource>), BookError> + Send),
    file: usize,
) -> Option<String> {
    let (_, mut source) = open(file).ok()?;
    if let Some(title) = source.title().filter(|t| chapter_number(t).is_some()) {
        return Some(title);
    }
    if source.sections().is_empty() {
        return None;
    }
    let section = source.load(0).ok()?;
    let line = section
        .paragraphs
        .iter()
        .filter(|p| image::key_of(p).is_none())
        .flat_map(|p| p.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    Some(line.chars().take(120).collect())
}

impl BookSource for FolderBook {
    fn title(&self) -> Option<String> {
        Some(self.title.clone())
    }

    fn sections(&self) -> Vec<SectionMeta> {
        self.chapters
            .iter()
            .map(|c| SectionMeta {
                label: c.title.clone(),
                pages: None,
                number: c.number,
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
        self.chapters[index].title = chapter_title(&paragraphs);
        Ok(Section { index, paragraphs })
    }

    fn section_title(&self, index: usize) -> Option<String> {
        self.chapters.get(index)?.title.clone()
    }

    fn image(&mut self, key: &str) -> Result<Image, BookError> {
        let (index, key) = key
            .split_once('/')
            .and_then(|(i, k)| Some((i.parse::<usize>().ok()?, k)))
            .ok_or_else(|| BookError::NoImage(key.to_string()))?;
        self.source(index)?.image(key)
    }
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

/// The chapter title from the first lines of its text: the first line without its
/// "Chương 12:" part, or the line itself when it is short and reads like a heading.
/// None when the text starts straight with the story.
pub fn chapter_title(paragraphs: &[String]) -> Option<String> {
    let mut lines = paragraphs
        .iter()
        .take(5)
        .filter(|p| image::key_of(p).is_none())
        .flat_map(|p| p.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty());
    let line = lines.next()?;
    let heading = |l: &str| {
        l.chars().count() <= 80
            && !l.ends_with(['.', '!', '?', '。', '！', '？', '…', '"', '”', ',', ';'])
    };
    if let Some(rest) = strip_chapter_prefix(line) {
        let rest = trim_separators(rest);
        if !rest.is_empty() {
            return Some(rest.chars().take(120).collect());
        }
        // "Chương 12" alone on its line, the title on the next one.
        return lines
            .next()
            .filter(|l| heading(l) && strip_chapter_prefix(l).is_none())
            .map(String::from);
    }
    heading(line).then(|| line.to_string())
}

/// `rest` without the separators between a chapter number and its title.
pub fn trim_separators(rest: &str) -> &str {
    rest.trim_start_matches(|c: char| c.is_whitespace() || ":：-–—.、,)]".contains(c))
        .trim()
}

/// The rest of `line` after a leading "Chương 12" / "Chapter 12" / "第十二章";
/// None when the line doesn't start with one.
pub fn strip_chapter_prefix(line: &str) -> Option<&str> {
    let lower = line.to_lowercase();
    if lower.len() != line.len() {
        // Lowercasing changed byte offsets; only the 第…章 form is matched then.
        return strip_han_prefix(line);
    }
    if let Some(rest) = strip_han_prefix(line) {
        return Some(rest);
    }
    for word in CHAPTER_WORDS[0].iter().filter(|w| w.len() > 1) {
        if let Some(after) = lower.strip_prefix(word) {
            let offset = line.len() - after.len();
            let rest = &line[offset..];
            let rest = rest.trim_start_matches([' ', '.', ':', '_', '-', '#', '\u{a0}']);
            if let Some((_, len)) = leading_number(rest) {
                return Some(&rest[len..]);
            }
        }
    }
    None
}

fn strip_han_prefix(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('第')?;
    let end = rest.find(['章', '回', '话', '話', '节', '節', '集'])?;
    let num = rest[..end].trim();
    let c = rest[end..].chars().next()?;
    (leading_number(num).is_some() || chinese_number(num).is_some())
        .then(|| &rest[end + c.len_utf8()..])
}

/// A chapter number written as a heading ("Chương 12: …", "第十二章"), not just any
/// number: for labels of books imported from one file.
pub fn heading_number(label: &str) -> Option<f64> {
    strip_chapter_prefix(label)?;
    chapter_number(label)
}

/// Splits a "Chương 12: Gặp lại" label into its title ("Gặp lại", or None when
/// nothing is left) and number; other labels stay as they are, without a number.
pub fn split_heading(label: Option<String>) -> (Option<String>, Option<f64>) {
    match label.as_deref().and_then(heading_number) {
        Some(n) => {
            let rest = label
                .as_deref()
                .and_then(strip_chapter_prefix)
                .map(|r| trim_separators(r).to_string())
                .filter(|r| !r.is_empty());
            (rest, Some(n))
        }
        None => (label, None),
    }
}

/// Words that come right before a chapter number in file names, strongest first:
/// "Tập 2 Chương 15" is chapter 15.
const CHAPTER_WORDS: &[&[&str]] = &[
    &[
        "chương", "chuong", "chapter", "chap", "ch", "hồi", "hoi", "c",
    ],
    &["tập", "tap", "episode", "ep", "phần", "phan", "part"],
];

/// The chapter number in a file name or heading: after a chapter word ("Chương 12",
/// "chap_012", "C12"), inside 第…章/回/话/節 (also in Chinese numerals), or else the
/// first number in the text. "12.5" counts as 12.5.
pub fn chapter_number(text: &str) -> Option<f64> {
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();

    // 第12章 / 第十二章
    for (i, _) in chars.iter().enumerate().filter(|(_, &c)| c == '第') {
        let rest: String = chars[i + 1..]
            .iter()
            .take_while(|c| !matches!(c, '章' | '回' | '话' | '話' | '节' | '節' | '集'))
            .collect();
        if rest.chars().count() < chars.len() - i - 1 {
            let rest = rest.trim();
            if let Some(n) = leading_number(rest)
                .map(|(n, _)| n)
                .or_else(|| chinese_number(rest))
            {
                return Some(n);
            }
        }
    }

    // A chapter word at a word start, then separators, then digits.
    for words in CHAPTER_WORDS {
        for i in (0..chars.len()).filter(|&i| i == 0 || !chars[i - 1].is_alphanumeric()) {
            for word in *words {
                let w: Vec<char> = word.chars().collect();
                if !chars[i..].starts_with(&w) {
                    continue;
                }
                let mut j = i + w.len();
                // "chap.", "Chương: ", "ch_"; not letters ("chapters" is no match).
                while j < chars.len()
                    && matches!(chars[j], ' ' | '.' | ':' | '_' | '-' | '#' | '\u{a0}')
                {
                    j += 1;
                }
                let rest: String = chars[j..].iter().collect();
                if let Some((n, _)) = leading_number(&rest) {
                    return Some(n);
                }
            }
        }
    }

    // The first number anywhere.
    let start = chars.iter().position(|c| c.is_ascii_digit())?;
    let rest: String = chars[start..].iter().collect();
    leading_number(&rest).map(|(n, _)| n)
}

/// Digits at the start of `s` (with an optional ".5" part) and their length in bytes.
fn leading_number(s: &str) -> Option<(f64, usize)> {
    let int = s.bytes().take_while(u8::is_ascii_digit).count();
    if int == 0 {
        return None;
    }
    let mut end = int;
    let bytes = s.as_bytes();
    if bytes.get(int) == Some(&b'.') {
        let frac = bytes[int + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if frac > 0 {
            end = int + 1 + frac;
        }
    }
    s[..end].parse().ok().map(|n| (n, end))
}

/// 十二 → 12, 一百零五 → 105, 两千 → 2000. None when not all Chinese numerals.
fn chinese_number(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let (mut total, mut section, mut digit) = (0u64, 0u64, None::<u64>);
    for c in s.chars() {
        let d = match c {
            '零' | '〇' => Some(0),
            '一' => Some(1),
            '二' | '两' | '兩' => Some(2),
            '三' => Some(3),
            '四' => Some(4),
            '五' => Some(5),
            '六' => Some(6),
            '七' => Some(7),
            '八' => Some(8),
            '九' => Some(9),
            _ => None,
        };
        if let Some(d) = d {
            digit = Some(d);
            continue;
        }
        let unit = match c {
            '十' => 10,
            '百' => 100,
            '千' => 1000,
            '万' | '萬' => 10_000,
            _ => return None,
        };
        if unit == 10_000 {
            section += digit.take().unwrap_or(0);
            total += section.max(1) * unit;
            section = 0;
        } else {
            section += digit.take().unwrap_or(1) * unit;
        }
    }
    Some((total + section + digit.unwrap_or(0)) as f64)
}

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
    fn finds_chapter_numbers() {
        let cases: &[(&str, Option<f64>)] = &[
            ("Chương 12 - Gặp lại", Some(12.0)),
            ("chuong_005", Some(5.0)),
            ("Chapter 7: The End", Some(7.0)),
            ("chap.3", Some(3.0)),
            ("Tập 2 Chương 15", Some(15.0)),
            ("Tập 3", Some(3.0)),
            ("第12章 重逢", Some(12.0)),
            ("第一百零五章", Some(105.0)),
            ("第十二回", Some(12.0)),
            ("Quyển 2 - 第三章", Some(3.0)),
            ("Ngoại truyện 3", Some(3.0)),
            ("001", Some(1.0)),
            ("Chương 12.5", Some(12.5)),
            ("C120 Hồi kết", Some(120.0)),
            ("Lời mở đầu", None),
            ("chapters", None),
        ];
        for (text, want) in cases {
            assert_eq!(chapter_number(text), *want, "{text}");
        }
    }

    #[test]
    fn chinese_numerals() {
        assert_eq!(chinese_number("十"), Some(10.0));
        assert_eq!(chinese_number("二十三"), Some(23.0));
        assert_eq!(chinese_number("两千零一"), Some(2001.0));
        assert_eq!(chinese_number("一万二千"), Some(12000.0));
        assert_eq!(chinese_number("abc"), None);
    }

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
    fn orders_files_by_chapter() {
        let (_, mut book) = fake_folder(
            &[
                "content://x/document/primary%3AT%2FCh%C6%B0%C6%A1ng%2010.txt",
                "content://x/document/primary%3AT%2FL%E1%BB%9Di%20b%E1%BA%A1t.txt",
                "content://x/document/primary%3AT%2FCh%C6%B0%C6%A1ng%202.txt",
                "content://x/msf%3A55",
                "content://x/document/primary%3AT%2FCh%C6%B0%C6%A1ng%201.txt",
            ],
            vec![
                ["Mười", "hết"],
                ["Bạt", "hết"],
                ["Hai", "\u{FFFC}a.png"],
                ["Chương 3: Ba", "hết"],
                ["Một", "hết"],
            ],
        );
        let numbers: Vec<_> = book.sections().into_iter().map(|s| s.number).collect();
        assert_eq!(numbers, [Some(1.0), Some(2.0), Some(3.0), Some(10.0), None]);
        assert_eq!(book.sections()[2].label, None); // titles come once read
        assert_eq!(book.load(0).unwrap().paragraphs, ["Một", "hết"]);
        assert_eq!(book.section_title(0).as_deref(), Some("Một"));
        book.load(2).unwrap();
        assert_eq!(book.sections()[2].label.as_deref(), Some("Ba"));
        assert_eq!(book.load(1).unwrap().paragraphs, ["Hai", "\u{FFFC}1/a.png"]);
        assert_eq!(book.image("1/a.png").unwrap().mime, "image/png");
        assert!(book.image("a.png").is_err());
        assert!(book.load(5).is_err());
    }

    #[test]
    fn chapter_titles_from_first_lines() {
        let t = |v: &[&str]| chapter_title(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(t(&["Chương 12: Gặp lại", "..."]), Some("Gặp lại".into()));
        assert_eq!(t(&["Chapter 3 - The Road", "..."]), Some("The Road".into()));
        assert_eq!(t(&["第十二章 重逢", "..."]), Some("重逢".into()));
        assert_eq!(
            t(&["Chương 5", "", "Mưa đêm", "Trời mưa."]),
            Some("Mưa đêm".into())
        );
        assert_eq!(t(&["Chương 5", "Trời mưa rất to."]), None);
        assert_eq!(t(&["Mưa đêm", "..."]), Some("Mưa đêm".into()));
        assert_eq!(t(&["Ngày xưa có một cô bé."]), None);
        assert_eq!(heading_number("Chương 7: Bão"), Some(7.0));
        assert_eq!(heading_number("Năm 1990"), None);
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
        // Titles from the first line: a story sentence is not one, "Hai" is.
        assert_eq!(sections[0].label, None);
        assert_eq!(sections[1].label.as_deref(), Some("Hai"));
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
