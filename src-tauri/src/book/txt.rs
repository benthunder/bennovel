use super::headings::{self, Candidate};
use super::{BookError, BookSource, Section, SectionMeta};
use encoding_rs::{Encoding, GB18030, GBK, UTF_16BE, UTF_16LE, UTF_8};
use std::io::{BufReader, Read, Seek, SeekFrom};

/// Target size of one section; it ends at the first line break after this.
pub const SECTION_BYTES: u64 = 64 * 1024;
/// A section with no line break is cut here anyway, at a character boundary.
const MAX_SECTION_BYTES: u64 = 4 * SECTION_BYTES;
/// A chapter longer than this is split into several sections at line breaks.
const MAX_CHAPTER_BYTES: u64 = 4 * SECTION_BYTES;
/// Text before the first chapter heading shorter than this joins the first chapter
/// (a book title and author line); longer, it is a section of its own.
const PREAMBLE_BYTES: u64 = 256;
/// Lines longer than this (bytes) are not checked for a chapter heading.
const MAX_HEADING_BYTES: usize = 4 * headings::MAX_HEADING_CHARS;

/// Plain text read in sections: one per chapter when the text has chapter headings
/// ("Chương 12", "第十二章"…), else ~64 KB each. Opening scans the file once, a
/// buffer at a time, to find where sections start; their text is decoded only when
/// requested.
pub struct TxtBook<R: Read + Seek> {
    reader: R,
    title: Option<String>,
    encoding: &'static Encoding,
    /// Where the text starts (after a byte order mark).
    data_start: u64,
    /// Start offset (relative to `data_start`) and byte length of each section.
    sections: Vec<(u64, u64)>,
    /// Chapter heading that starts each section, if any.
    labels: Vec<Option<String>>,
}

impl<R: Read + Seek + Send> TxtBook<R> {
    pub fn open(mut reader: R, title: Option<String>) -> Result<Self, BookError> {
        let mut head = vec![0u8; SECTION_BYTES as usize];
        let n = read_up_to(&mut reader, &mut head)?;
        head.truncate(n);
        let (encoding, data_start) = detect_encoding(&head);

        reader.seek(SeekFrom::Start(data_start))?;
        let scan = scan(BufReader::new(&mut reader), encoding)?;
        let (sections, labels) = chapter_sections(scan);
        Ok(Self {
            reader,
            title,
            encoding,
            data_start,
            sections,
            labels,
        })
    }
}

/// Byte order mark first, then UTF-16 without one (every other byte zero), then
/// UTF-8 when the sample decodes as UTF-8 (a few broken bytes allowed), else the
/// guess of `chardetng` (Firefox's detector: GBK, Big5, Shift_JIS, EUC-KR,
/// Windows-1251/1258…). GBK is read as GB18030, its superset.
pub(super) fn detect_encoding(head: &[u8]) -> (&'static Encoding, u64) {
    if let Some((enc, bom)) = Encoding::for_bom(head) {
        return (enc, bom as u64);
    }
    if let Some(enc) = utf16_without_bom(head) {
        return (enc, 0);
    }
    if mostly_utf8(head) {
        return (UTF_8, 0);
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(head, true);
    let enc = detector.guess(None, false);
    (if enc == GBK { GB18030 } else { enc }, 0)
}

/// UTF-16 text in a Latin or CJK script has a zero in most high bytes and almost
/// never in the low ones.
fn utf16_without_bom(head: &[u8]) -> Option<&'static Encoding> {
    let units = head.len() / 2;
    if units < 32 {
        return None;
    }
    let zeros = |parity: usize| {
        head.iter()
            .skip(parity)
            .step_by(2)
            .filter(|&&b| b == 0)
            .count()
    };
    let (even, odd) = (zeros(0), zeros(1));
    // Latin text: half the bytes are zero; CJK text has fewer, so the bar is lower.
    if odd * 5 > units && even * 50 < units {
        Some(UTF_16LE)
    } else if even * 5 > units && odd * 50 < units {
        Some(UTF_16BE)
    } else {
        None
    }
}

/// Valid UTF-8, or UTF-8 with a few broken bytes among plenty of valid multi-byte
/// characters (a file damaged in one place). Legacy encodings such as GBK produce
/// many invalid sequences, so they never pass.
fn mostly_utf8(head: &[u8]) -> bool {
    let (mut errors, mut multibyte, mut rest) = (0usize, 0usize, head);
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                multibyte += s.chars().filter(|c| !c.is_ascii()).count();
                break;
            }
            Err(e) => {
                let valid = &rest[..e.valid_up_to()];
                multibyte += std::str::from_utf8(valid)
                    .map_or(0, |s| s.chars().filter(|c| !c.is_ascii()).count());
                match e.error_len() {
                    // Only an incomplete character cut off at the end of the sample.
                    None => break,
                    Some(len) => {
                        errors += 1;
                        if errors > 8 {
                            return false;
                        }
                        rest = &rest[e.valid_up_to() + len..];
                    }
                }
            }
        }
    }
    errors == 0 || multibyte >= 50 * errors
}

/// What one pass over the file finds: fixed-size blocks and heading-like lines.
struct Scan {
    /// Start offset and length of ~64 KB blocks ending at line breaks.
    blocks: Vec<(u64, u64)>,
    /// Lines that look like chapter headings, in order.
    headings: Vec<Candidate>,
    len: u64,
}

fn scan(mut r: impl Read, enc: &'static Encoding) -> Result<Scan, BookError> {
    let utf16_le = enc == UTF_16LE;
    let utf16 = utf16_le || enc == UTF_16BE;
    let mut blocks = Vec::new();
    let mut found = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut start, mut pos, mut prev) = (0u64, 0u64, 0u8);
    // The current line, while it is short enough to be a heading.
    let (mut line_start, mut line) = (0u64, Vec::with_capacity(MAX_HEADING_BYTES));
    let mut check_line = |line: &mut Vec<u8>, line_start: u64, end: u64| {
        // UTF-16 has zero bytes in its characters; other encodings keep ASCII as is.
        let worth = !line.is_empty()
            && line.len() <= MAX_HEADING_BYTES
            && (utf16 || headings::may_be_heading(line));
        // The first few characters rule out most lines without decoding the rest.
        let start = &line[..line.len().min(if utf16 { 96 } else { 48 })];
        if worth && headings::may_start(&enc.decode_without_bom_handling(start).0) {
            let (text, _) = enc.decode_without_bom_handling(line);
            if let Some(c) = headings::candidate(&text, line_start, end) {
                found.push(c);
            }
        }
        line.clear();
    };

    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            // Over-long line: cut before this byte if a character starts here.
            if pos - start >= MAX_SECTION_BYTES {
                let boundary = if utf16 {
                    pos % 2 == 0
                } else if enc == UTF_8 {
                    b & 0xC0 != 0x80
                } else {
                    // GB18030 trail bytes are >= 0x30, so a lower byte stands alone.
                    b < 0x30
                };
                if boundary {
                    blocks.push((start, pos - start));
                    start = pos;
                }
            }
            let line_end = if utf16 {
                pos % 2 == 1
                    && if utf16_le {
                        prev == b'\n' && b == 0
                    } else {
                        prev == 0 && b == b'\n'
                    }
            } else {
                b == b'\n'
            };
            if line.len() <= MAX_HEADING_BYTES {
                line.push(b);
            }
            pos += 1;
            prev = b;
            if line_end {
                check_line(&mut line, line_start, pos);
                line_start = pos;
                if pos - start >= SECTION_BYTES {
                    blocks.push((start, pos - start));
                    start = pos;
                }
            }
        }
    }
    check_line(&mut line, line_start, pos);
    if pos > start || blocks.is_empty() {
        blocks.push((start, pos - start));
    }
    Ok(Scan {
        blocks,
        headings: found,
        len: pos,
    })
}

/// Sections from the chapter headings that pass [`headings::filter`], each long
/// chapter split again at block boundaries; the plain blocks when there are none.
fn chapter_sections(scan: Scan) -> (Vec<(u64, u64)>, Vec<Option<String>>) {
    let heads = headings::filter(scan.headings);
    if heads.is_empty() {
        let labels = vec![None; scan.blocks.len()];
        return (scan.blocks, labels);
    }
    let mut chapters: Vec<(u64, Option<String>)> = Vec::with_capacity(heads.len() + 1);
    let preamble = heads[0].start >= PREAMBLE_BYTES;
    if preamble {
        chapters.push((0, None));
    }
    for (i, h) in heads.into_iter().enumerate() {
        let start = if i == 0 && !preamble { 0 } else { h.start };
        chapters.push((start, Some(h.label)));
    }

    // Line breaks to split long chapters at: where blocks start.
    let cuts: Vec<u64> = scan.blocks.iter().skip(1).map(|&(s, _)| s).collect();
    let (mut sections, mut labels) = (Vec::new(), Vec::new());
    for (i, (start, label)) in chapters.iter().enumerate() {
        let end = chapters.get(i + 1).map_or(scan.len, |c| c.0);
        let mut cur = *start;
        let mut label = label.clone();
        let from = cuts.partition_point(|&c| c <= cur);
        for &cut in cuts[from..].iter().take_while(|&&c| c < end) {
            if cut - cur >= MAX_CHAPTER_BYTES {
                sections.push((cur, cut - cur));
                labels.push(label.take());
                cur = cut;
            }
        }
        sections.push((cur, end - cur));
        labels.push(label);
    }
    (sections, labels)
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

impl<R: Read + Seek + Send> BookSource for TxtBook<R> {
    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn sections(&self) -> Vec<SectionMeta> {
        self.labels
            .iter()
            .map(|label| SectionMeta {
                label: label.clone(),
                pages: None,
                number: None,
            })
            .collect()
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let &(offset, len) = self
            .sections
            .get(index)
            .ok_or(BookError::NoSection(index))?;
        self.reader
            .seek(SeekFrom::Start(self.data_start + offset))?;
        let mut bytes = vec![0u8; len as usize];
        self.reader.read_exact(&mut bytes)?;
        let (text, _) = self.encoding.decode_without_bom_handling(&bytes);
        let paragraphs = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        Ok(Section { index, paragraphs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn lines(n: usize) -> String {
        (1..=n)
            .map(|i| format!("Dòng số {i} của truyện.\n"))
            .collect()
    }

    fn all_paragraphs(book: &mut TxtBook<Cursor<Vec<u8>>>) -> Vec<String> {
        let count = book.sections().len();
        (0..count)
            .flat_map(|i| book.load(i).unwrap().paragraphs)
            .collect()
    }

    #[test]
    fn splits_large_utf8_file_at_line_breaks() {
        let text = lines(20_000);
        let mut book = TxtBook::open(Cursor::new(text.clone().into_bytes()), None).unwrap();
        let n = book.sections().len();
        assert!(n > 5, "expected several sections, got {n}");
        assert!(book
            .sections
            .iter()
            .all(|&(_, len)| len < SECTION_BYTES + 100));
        // Every line comes back exactly once, in order, none cut in half.
        let want: Vec<String> = text.lines().map(String::from).collect();
        assert_eq!(all_paragraphs(&mut book), want);
    }

    #[test]
    fn detects_gbk_and_utf16() {
        let text = "第一章 开始\r\n　　他走进了房间。\r\n";
        let (gbk, _, _) = GB18030.encode(text);
        let mut book = TxtBook::open(Cursor::new(gbk.into_owned()), None).unwrap();
        assert_eq!(book.encoding, GB18030);
        assert_eq!(
            book.load(0).unwrap().paragraphs,
            vec!["第一章 开始", "他走进了房间。"]
        );

        let mut le = vec![0xFF, 0xFE];
        le.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let mut book = TxtBook::open(Cursor::new(le), None).unwrap();
        assert_eq!(book.encoding, UTF_16LE);
        assert_eq!(
            book.load(0).unwrap().paragraphs,
            vec!["第一章 开始", "他走进了房间。"]
        );
    }

    #[test]
    fn splits_large_utf16_file_on_whole_lines() {
        let text = lines(10_000);
        let mut be = vec![0xFE, 0xFF];
        be.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        let mut book = TxtBook::open(Cursor::new(be), None).unwrap();
        assert!(book.sections().len() > 5);
        assert_eq!(
            all_paragraphs(&mut book),
            text.lines().map(String::from).collect::<Vec<_>>()
        );
    }

    #[test]
    fn cuts_endless_line_at_character_boundary() {
        let text = "ả".repeat(300_000); // 2 bytes each, no line breaks
        let mut book = TxtBook::open(Cursor::new(text.clone().into_bytes()), None).unwrap();
        assert!(book.sections().len() >= 2);
        assert_eq!(all_paragraphs(&mut book).concat(), text);
    }

    fn labels(book: &TxtBook<Cursor<Vec<u8>>>) -> Vec<Option<String>> {
        book.sections().into_iter().map(|s| s.label).collect()
    }

    fn body(n: usize) -> String {
        "Trời mưa rất to, hắn đứng lặng dưới mái hiên nhìn ra phố.\n".repeat(n)
    }

    #[test]
    fn finds_chapters_after_preamble_and_table_of_contents() {
        let mut text =
            String::from("Đấu Phá Thương Khung\nTác giả: Thiên Tằm Thổ Đậu\n\nMục lục\n");
        for i in 1..=4 {
            text.push_str(&format!("Chương {i}\n"));
        }
        text.push_str(&body(5));
        for i in 1..=4 {
            text.push_str(&format!("\nChương {i}: Phần {i}\n\n{}", body(20)));
        }
        let mut book = TxtBook::open(Cursor::new(text.into_bytes()), None).unwrap();
        let mut want = vec![None];
        want.extend((1..=4).map(|i| Some(format!("Chương {i}: Phần {i}"))));
        assert_eq!(labels(&book), want);
        // The preamble keeps the book title and the table of contents.
        assert_eq!(book.load(0).unwrap().paragraphs[0], "Đấu Phá Thương Khung");
        let ch2 = book.load(2).unwrap().paragraphs;
        assert_eq!(ch2[0], "Chương 2: Phần 2");
        assert_eq!(ch2.len(), 21);
    }

    #[test]
    fn short_title_joins_the_first_chapter() {
        let mut text = String::from("Truyện ngắn\n");
        for i in 1..=3 {
            text.push_str(&format!("Chương {i}\n{}", body(10)));
        }
        let mut book = TxtBook::open(Cursor::new(text.into_bytes()), None).unwrap();
        assert_eq!(
            labels(&book),
            vec![
                Some("Chương 1".to_string()),
                Some("Chương 2".to_string()),
                Some("Chương 3".to_string())
            ]
        );
        assert_eq!(
            book.load(0).unwrap().paragraphs[..2],
            ["Truyện ngắn", "Chương 1"]
        );
    }

    #[test]
    fn finds_headings_drawn_with_long_decoration() {
        let mut text = String::new();
        for i in 1..=3 {
            text.push_str(&format!(
                "==================== Chương {i} ====================\n{}",
                body(10)
            ));
        }
        let book = TxtBook::open(Cursor::new(text.into_bytes()), None).unwrap();
        assert_eq!(labels(&book)[2].as_deref(), Some("Chương 3"));
    }

    #[test]
    fn finds_chinese_chapters_in_gbk() {
        let mut text = String::new();
        let nums = ["一", "二", "三", "四", "五", "六"];
        for i in nums {
            text.push_str(&format!("第{i}章 风起\r\n"));
            for _ in 0..20 {
                text.push_str("　　他走进了房间，看见桌上放着一封信，信上写着他的名字。\r\n");
            }
        }
        // A sentence that starts like a heading, indented like the story text.
        text.push_str("　　第一章就这样结束了\r\n");
        let (gbk, _, _) = GBK.encode(&text);
        let mut book = TxtBook::open(Cursor::new(gbk.into_owned()), None).unwrap();
        assert_eq!(book.encoding, GB18030);
        assert_eq!(
            labels(&book),
            nums.map(|i| Some(format!("第{i}章 风起"))).to_vec()
        );
        assert_eq!(book.load(5).unwrap().paragraphs.len(), 22);
    }

    #[test]
    fn splits_a_long_chapter_and_names_only_its_first_part() {
        let mut text = String::new();
        for i in 1..=3 {
            text.push_str(&format!("Chương {i}\n"));
            text.push_str(&body(if i == 2 { 12_000 } else { 10 }));
        }
        let mut book = TxtBook::open(Cursor::new(text.clone().into_bytes()), None).unwrap();
        let labels = labels(&book);
        assert!(labels.len() > 4, "{labels:?}");
        assert_eq!(labels[1].as_deref(), Some("Chương 2"));
        assert_eq!(labels[2], None);
        assert_eq!(labels.last().unwrap().as_deref(), Some("Chương 3"));
        assert!(book
            .sections
            .iter()
            .all(|&(_, len)| len <= MAX_CHAPTER_BYTES + SECTION_BYTES + 100));
        let want: Vec<String> = text.lines().map(String::from).collect();
        assert_eq!(all_paragraphs(&mut book), want);
    }

    #[test]
    fn utf16_chapters_without_bom() {
        let mut text = String::new();
        for i in 1..=3 {
            text.push_str(&format!(
                "Chapter {i}\r\n{}",
                "It was raining.\r\n".repeat(30)
            ));
        }
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut book = TxtBook::open(Cursor::new(le), None).unwrap();
        assert_eq!(book.encoding, UTF_16LE);
        assert_eq!(labels(&book)[2].as_deref(), Some("Chapter 3"));
        assert_eq!(book.load(2).unwrap().paragraphs[0], "Chapter 3");
    }

    #[test]
    fn detects_legacy_encodings() {
        let cases: [(&str, &'static Encoding); 4] = [
            (
                "他走進了房間，看見桌上放著一封信。這是他第一次離開家鄉，心裡十分緊張。",
                encoding_rs::BIG5,
            ),
            (
                "彼は部屋に入って、机の上に置かれた手紙を見た。これは初めて故郷を離れる日だった。",
                encoding_rs::SHIFT_JIS,
            ),
            (
                "Он вошёл в комнату и увидел на столе письмо. Это был первый раз, когда он уехал из дома.",
                encoding_rs::WINDOWS_1251,
            ),
            (
                "他走进了房间，看见桌上放着一封信。这是他第一次离开家乡，心里十分紧张。",
                GB18030,
            ),
        ];
        for (text, enc) in cases {
            let text = format!("{text}\n").repeat(20);
            let (bytes, _, _) = enc.encode(&text);
            assert_eq!(detect_encoding(&bytes).0, enc, "{text}");
        }
    }

    #[test]
    fn keeps_utf8_with_one_broken_byte() {
        let mut bytes = "Trời mưa rất to, hắn đứng lặng dưới mái hiên.\n"
            .repeat(20)
            .into_bytes();
        bytes[300] = 0xFF;
        assert_eq!(detect_encoding(&bytes).0, UTF_8);
        assert_eq!(detect_encoding(b"plain ascii\n").0, UTF_8);
    }

    #[test]
    fn empty_file_has_one_empty_section() {
        let mut book = TxtBook::open(Cursor::new(Vec::new()), None).unwrap();
        assert_eq!(book.sections().len(), 1);
        assert!(book.load(0).unwrap().paragraphs.is_empty());
    }
}
