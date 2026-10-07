//! MOBI / AZW3 / PRC (Palm database) books. Text records are read and PalmDOC-
//! decompressed one at a time (about 4 KB each) and streamed into a [`Sink`].

use super::extract::{html_block, ExtractedBook, Sink};
use super::text::HtmlText;
use super::BookError;
use encoding_rs::{UTF_8, WINDOWS_1252};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn bad(msg: &str) -> BookError {
    BookError::Parse(msg.to_string())
}

pub fn extract_mobi<R: Read + Seek>(
    mut r: R,
    cache_dir: &Path,
    fallback_title: Option<String>,
) -> Result<ExtractedBook, BookError> {
    let mut pdb = [0u8; 78];
    r.read_exact(&mut pdb)?;
    let kind = &pdb[60..68];
    if kind != b"BOOKMOBI" && kind != b"TEXtREAd" {
        return Err(bad("not a MOBI/PRC book"));
    }
    let count = be16(&pdb, 76).unwrap_or(0) as usize;
    let mut table = vec![0u8; count * 8];
    r.read_exact(&mut table)?;
    let offsets: Vec<u64> = (0..count)
        .map(|i| be32(&table, i * 8).unwrap_or(0) as u64)
        .collect();
    let file_len = r.seek(SeekFrom::End(0))?;
    let record = |r: &mut R, i: usize| -> Result<Vec<u8>, BookError> {
        let start = *offsets.get(i).ok_or_else(|| bad("missing record"))?;
        let end = offsets.get(i + 1).copied().unwrap_or(file_len);
        if end < start || end > file_len {
            return Err(bad("broken record table"));
        }
        r.seek(SeekFrom::Start(start))?;
        let mut data = vec![0u8; (end - start) as usize];
        r.read_exact(&mut data)?;
        Ok(data)
    };

    let r0 = record(&mut r, 0)?;
    let compression = be16(&r0, 0).unwrap_or(1);
    let text_records = be16(&r0, 8).unwrap_or(0) as usize;
    if be16(&r0, 12).unwrap_or(0) != 0 {
        return Err(bad("this book is DRM-protected"));
    }
    if compression == 17480 {
        return Err(bad("HUFF/CDIC compressed MOBI is not supported yet"));
    }
    let has_mobi = r0.get(16..20) == Some(b"MOBI");
    let (utf8, extra_flags, title) = if has_mobi {
        let header_len = be32(&r0, 20).unwrap_or(0);
        let utf8 = be32(&r0, 28) == Some(65001);
        let flags = if header_len >= 0xE4 {
            be16(&r0, 0xF2).unwrap_or(0)
        } else {
            0
        };
        let name_at = be32(&r0, 0x54).unwrap_or(0) as usize;
        let name_len = be32(&r0, 0x58).unwrap_or(0) as usize;
        let title = r0.get(name_at..name_at + name_len).map(|n| {
            if utf8 {
                String::from_utf8_lossy(n).into_owned()
            } else {
                WINDOWS_1252.decode_without_bom_handling(n).0.into_owned()
            }
        });
        (utf8, flags, title)
    } else {
        (false, 0, None)
    };
    let pdb_name = String::from_utf8_lossy(&pdb[..32])
        .trim_end_matches('\0')
        .to_string();

    let mut sink = Sink::new(cache_dir)?;
    sink.title = title
        .filter(|t| !t.trim().is_empty())
        .or(fallback_title)
        .or(Some(pdb_name));
    let mut decoder = if utf8 { UTF_8 } else { WINDOWS_1252 }.new_decoder_without_bom_handling();
    let mut html = HtmlText::default();
    let mut text = String::new();
    let mut plain = String::new();

    for i in 1..=text_records {
        let mut data = record(&mut r, i)?;
        let trailing = trailing_size(&data, extra_flags);
        data.truncate(data.len().saturating_sub(trailing));
        let bytes = match compression {
            1 => data,
            2 => palmdoc_decompress(&data),
            _ => return Err(bad("unknown MOBI compression")),
        };
        text.clear();
        text.reserve(bytes.len() * 3);
        let _ = decoder.decode_to_string(&bytes, &mut text, i == text_records);
        if has_mobi {
            let mut err = Ok(());
            html.feed(&text, &mut |b| {
                if err.is_ok() {
                    err = html_block(&mut sink, b);
                }
            });
            err?;
        } else {
            // Plain PalmDOC text: one paragraph per line.
            plain.push_str(&text);
            while let Some(nl) = plain.find('\n') {
                sink.paragraph(&plain[..nl])?;
                plain.drain(..=nl);
            }
        }
    }
    let mut err = Ok(());
    html.finish(&mut |b| {
        if err.is_ok() {
            err = html_block(&mut sink, b);
        }
    });
    err?;
    sink.paragraph(&plain)?;
    sink.finish()
}

/// Bytes at the end of a text record that are index data, not text.
fn trailing_size(data: &[u8], flags: u16) -> usize {
    let mut size = 0usize;
    let mut f = flags >> 1;
    while f != 0 {
        if f & 1 != 0 {
            let end = data.len().saturating_sub(size);
            let mut n = 0usize;
            for &v in &data[end.saturating_sub(4)..end] {
                if v & 0x80 != 0 {
                    n = 0;
                }
                n = (n << 7) | (v & 0x7F) as usize;
            }
            size += n;
        }
        f >>= 1;
    }
    if flags & 1 != 0 {
        if let Some(&b) = data.len().checked_sub(size + 1).and_then(|i| data.get(i)) {
            size += (b & 0x3) as usize + 1;
        }
    }
    size.min(data.len())
}

pub fn palmdoc_decompress(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 2);
    let mut i = 0;
    while i < input.len() {
        let c = input[i];
        i += 1;
        match c {
            0x01..=0x08 => {
                let end = (i + c as usize).min(input.len());
                out.extend_from_slice(&input[i..end]);
                i = end;
            }
            0x80..=0xBF => {
                let Some(&next) = input.get(i) else { break };
                i += 1;
                let pair = ((c as usize) << 8) | next as usize;
                let dist = (pair >> 3) & 0x7FF;
                let len = (pair & 7) + 3;
                if dist == 0 || dist > out.len() {
                    continue;
                }
                let from = out.len() - dist;
                for k in 0..len {
                    out.push(out[from + k]);
                }
            }
            0xC0..=0xFF => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::extract::test_dir;
    use super::super::BookSource;
    use super::*;
    use std::io::Cursor;

    /// Simplest valid PalmDOC stream: literal bytes, with 0x01-0x08 and >= 0x80 escaped.
    fn palmdoc_compress(text: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in text {
            if (0x01..=0x08).contains(&b) || b >= 0x80 {
                out.push(1);
            }
            out.push(b);
        }
        out
    }

    /// A MOBI file with UTF-8 HTML split over 4 KB text records, each with one
    /// multibyte trailing entry (extra flags = 0b10).
    pub fn sample_mobi(html: &str) -> Vec<u8> {
        let records: Vec<Vec<u8>> = html
            .as_bytes()
            .chunks(4096)
            .map(|c| {
                let mut r = palmdoc_compress(c);
                r.extend_from_slice(&[0xAA, 0xBB, 0x83]); // 3-byte trailing entry
                r
            })
            .collect();
        let title = "Sách MOBI";
        let mut r0 = vec![0u8; 0x100];
        r0[0..2].copy_from_slice(&2u16.to_be_bytes());
        r0[4..8].copy_from_slice(&(html.len() as u32).to_be_bytes());
        r0[8..10].copy_from_slice(&(records.len() as u16).to_be_bytes());
        r0[10..12].copy_from_slice(&4096u16.to_be_bytes());
        r0[16..20].copy_from_slice(b"MOBI");
        r0[20..24].copy_from_slice(&0xE8u32.to_be_bytes());
        r0[28..32].copy_from_slice(&65001u32.to_be_bytes());
        r0[0x54..0x58].copy_from_slice(&(0x100u32).to_be_bytes());
        r0[0x58..0x5C].copy_from_slice(&(title.len() as u32).to_be_bytes());
        r0[0xF2..0xF4].copy_from_slice(&2u16.to_be_bytes());
        r0.extend_from_slice(title.as_bytes());

        let all: Vec<&Vec<u8>> = std::iter::once(&r0).chain(records.iter()).collect();
        let mut out = vec![0u8; 78];
        out[..8].copy_from_slice(b"pdbname\0");
        out[60..68].copy_from_slice(b"BOOKMOBI");
        out[76..78].copy_from_slice(&(all.len() as u16).to_be_bytes());
        let mut offset = 78 + all.len() * 8 + 2;
        for rec in &all {
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&[0; 4]);
            offset += rec.len();
        }
        out.extend_from_slice(&[0, 0]);
        for rec in all {
            out.extend_from_slice(rec);
        }
        out
    }

    #[test]
    fn decompresses_back_references() {
        // "abcabcabc": 3 literals, then copy distance 3 length 6.
        let pair: u16 = 0x8000 | (3 << 3) | (6 - 3);
        let mut input = b"abc".to_vec();
        input.extend_from_slice(&pair.to_be_bytes());
        input.push(0xC1); // " A"
        assert_eq!(palmdoc_decompress(&input), b"abcabcabc A");
    }

    #[test]
    fn reads_mobi_records_into_chapters() {
        let mut html = String::from("<html><body><h1>Chương 1</h1>");
        for i in 0..400 {
            html.push_str(&format!("<p>Đoạn văn số {i} với chữ tiếng Việt.</p>"));
        }
        html.push_str("<mbp:pagebreak/><h2>Chương 2</h2><p>Hết.</p></body></html>");
        let mut book = extract_mobi(Cursor::new(sample_mobi(&html)), &test_dir(), None).unwrap();
        assert_eq!(book.title().as_deref(), Some("Sách MOBI"));
        let labels: Vec<_> = book
            .sections()
            .into_iter()
            .filter_map(|s| s.label)
            .collect();
        assert_eq!(labels, vec!["Chương 1", "Chương 2"]);
        let all: Vec<String> = (0..book.sections().len())
            .flat_map(|i| book.load(i).unwrap().paragraphs)
            .collect();
        assert_eq!(all.len(), 403);
        assert_eq!(all[1], "Đoạn văn số 0 với chữ tiếng Việt.");
        assert_eq!(all[400], "Đoạn văn số 399 với chữ tiếng Việt.");
        assert_eq!(all[402], "Hết.");
    }
}
