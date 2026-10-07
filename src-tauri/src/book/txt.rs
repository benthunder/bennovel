use super::{BookError, BookSource, Section, SectionMeta};
use encoding_rs::{Encoding, GB18030, UTF_16BE, UTF_16LE, UTF_8};
use std::io::{BufReader, Read, Seek, SeekFrom};

/// Target size of one section; it ends at the first line break after this.
pub const SECTION_BYTES: u64 = 64 * 1024;
/// A section with no line break is cut here anyway, at a character boundary.
const MAX_SECTION_BYTES: u64 = 4 * SECTION_BYTES;

/// Plain text read in ~64 KB sections. Opening scans the file once, a buffer at a
/// time, to find where sections start; their text is decoded only when requested.
pub struct TxtBook<R: Read + Seek> {
    reader: R,
    title: Option<String>,
    encoding: &'static Encoding,
    /// Where the text starts (after a byte order mark).
    data_start: u64,
    /// Start offset (relative to `data_start`) and byte length of each section.
    sections: Vec<(u64, u64)>,
}

impl<R: Read + Seek + Send> TxtBook<R> {
    pub fn open(mut reader: R, title: Option<String>) -> Result<Self, BookError> {
        let mut head = vec![0u8; SECTION_BYTES as usize];
        let n = read_up_to(&mut reader, &mut head)?;
        head.truncate(n);
        let (encoding, data_start) = detect_encoding(&head);

        reader.seek(SeekFrom::Start(data_start))?;
        let sections = index_sections(BufReader::new(&mut reader), encoding)?;
        Ok(Self {
            reader,
            title,
            encoding,
            data_start,
            sections,
        })
    }
}

/// Byte order mark first; otherwise UTF-8 when the start of the file is valid
/// UTF-8, else GB18030 (a superset of GBK, common for Chinese novels).
pub(super) fn detect_encoding(head: &[u8]) -> (&'static Encoding, u64) {
    if let Some((enc, bom)) = Encoding::for_bom(head) {
        return (enc, bom as u64);
    }
    match std::str::from_utf8(head) {
        Ok(_) => (UTF_8, 0),
        // Only an incomplete character cut off at the end of the sample.
        Err(e) if e.error_len().is_none() => (UTF_8, 0),
        Err(_) => (GB18030, 0),
    }
}

fn index_sections(mut r: impl Read, enc: &'static Encoding) -> Result<Vec<(u64, u64)>, BookError> {
    let utf16_le = enc == UTF_16LE;
    let utf16 = utf16_le || enc == UTF_16BE;
    let mut sections = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut start, mut pos, mut prev) = (0u64, 0u64, 0u8);

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
                    sections.push((start, pos - start));
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
            pos += 1;
            prev = b;
            if line_end && pos - start >= SECTION_BYTES {
                sections.push((start, pos - start));
                start = pos;
            }
        }
    }
    if pos > start || sections.is_empty() {
        sections.push((start, pos - start));
    }
    Ok(sections)
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
        self.sections
            .iter()
            .map(|_| SectionMeta {
                label: None,
                pages: None,
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

    #[test]
    fn empty_file_has_one_empty_section() {
        let mut book = TxtBook::open(Cursor::new(Vec::new()), None).unwrap();
        assert_eq!(book.sections().len(), 1);
        assert!(book.load(0).unwrap().paragraphs.is_empty());
    }
}
