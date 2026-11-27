//! MOBI / AZW3 / PRC (Palm database) books. Text records are read and PalmDOC-
//! decompressed one at a time (about 4 KB each) and streamed into a [`Sink`].

use super::extract::{html_block, ExtractedBook, Sink};
use super::text::{Block, HtmlText};
use super::BookError;
use encoding_rs::{UTF_8, WINDOWS_1252};
use std::collections::HashMap;
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
    // Picture records follow the text; links count from this one.
    let first_image = if has_mobi {
        be32(&r0, 0x6C)
            .filter(|&n| n != 0 && n != 0xFFFF_FFFF)
            .map(|n| n as usize)
    } else {
        None
    };
    let (utf8, extra_flags, title, ncx_index) = if has_mobi {
        let header_len = be32(&r0, 20).unwrap_or(0);
        // The table of contents index, when the header is long enough to name it.
        let ncx_index = (header_len >= 0xE8)
            .then(|| be32(&r0, 0xF4))
            .flatten()
            .filter(|&n| n != 0 && n != 0xFFFF_FFFF && n as usize > text_records)
            .map(|n| n as usize);
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
        (utf8, flags, title, ncx_index)
    } else {
        (false, 0, None, None)
    };
    let pdb_name = String::from_utf8_lossy(&pdb[..32])
        .trim_end_matches('\0')
        .to_string();

    let mut sink = Sink::new(cache_dir)?;
    sink.title = title
        .filter(|t| !t.trim().is_empty())
        .or(fallback_title)
        .or(Some(pdb_name));
    // Chapters from the table of contents, when it lists at least two: a marker
    // element is put in the text where each starts, and the HTML reader reports it.
    let ncx = ncx_index
        .map(|first| read_ncx(&mut |i| record(&mut r, i).ok(), first, utf8))
        .unwrap_or_default();
    let ncx = if ncx.len() >= 2 { ncx } else { Vec::new() };
    let mut decoder = if utf8 { UTF_8 } else { WINDOWS_1252 }.new_decoder_without_bom_handling();
    let mut html = HtmlText::default();
    html.anchors = (0..ncx.len()).map(ncx_marker).collect();
    let mut text = String::new();
    let mut plain = String::new();
    let mut next_entry = 0usize;
    let mut consumed = 0u64;
    let by_toc = !ncx.is_empty();
    let emit = |sink: &mut Sink, b: Block| -> Result<(), BookError> {
        match b {
            Block::Anchor(id) => {
                let label = id
                    .strip_prefix(NCX_MARKER)
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| ncx.get(n))
                    .map(|e| e.label.as_str());
                sink.start_section(label);
                Ok(())
            }
            // With a table of contents, headings are text inside its chapters.
            Block::Heading(t) if by_toc => sink.paragraph(&t),
            b => html_block(sink, b),
        }
    };

    for i in 1..=text_records {
        let mut data = record(&mut r, i)?;
        let trailing = trailing_size(&data, extra_flags);
        data.truncate(data.len().saturating_sub(trailing));
        let bytes = match compression {
            1 => data,
            2 => palmdoc_decompress(&data),
            _ => return Err(bad("unknown MOBI compression")),
        };
        let last = i == text_records;
        if has_mobi {
            let mut err = Ok(());
            let mut feed = |html: &mut HtmlText, sink: &mut Sink, piece: &str| {
                html.feed(piece, &mut |b| {
                    if err.is_ok() {
                        err = emit(sink, b);
                    }
                });
            };
            // Cut the record where table of contents entries start.
            let mut start = 0usize;
            while let Some(e) = ncx.get(next_entry) {
                if e.pos >= consumed + bytes.len() as u64 {
                    break;
                }
                let cut = (e.pos.saturating_sub(consumed) as usize).max(start);
                text.clear();
                text.reserve((cut - start) * 3 + 16);
                let _ = decoder.decode_to_string(&bytes[start..cut], &mut text, false);
                feed(&mut html, &mut sink, &text);
                feed(
                    &mut html,
                    &mut sink,
                    &format!("<a id=\"{}\"></a>", ncx_marker(next_entry)),
                );
                start = cut;
                next_entry += 1;
            }
            text.clear();
            text.reserve((bytes.len() - start) * 3 + 16);
            let _ = decoder.decode_to_string(&bytes[start..], &mut text, last);
            feed(&mut html, &mut sink, &text);
            err?;
            consumed += bytes.len() as u64;
        } else {
            text.clear();
            text.reserve(bytes.len() * 3);
            let _ = decoder.decode_to_string(&bytes, &mut text, last);
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
            err = emit(&mut sink, b);
        }
    });
    err?;
    sink.paragraph(&plain)?;
    if let Some(first) = first_image {
        for key in sink.missing_images() {
            let Some(n) = image_number(&key) else {
                continue;
            };
            if n == 0 {
                continue;
            }
            if let Ok(data) = record(&mut r, first + n - 1) {
                sink.save_image(&key, &data)?;
            }
        }
    }
    sink.finish()
}

/// Id of the marker element put in the text where table of contents entry `n` starts.
const NCX_MARKER: &str = "bennovel-toc-";
fn ncx_marker(n: usize) -> String {
    format!("{NCX_MARKER}{n}")
}

/// A table of contents entry: where it starts in the decompressed text, and its title.
#[derive(Debug, PartialEq)]
struct NcxEntry {
    pos: u64,
    label: String,
}

/// A forward-encoded number: 7 bits per byte, the last byte has its high bit set.
fn varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for _ in 0..9 {
        let c = *b.get(*at)?;
        *at += 1;
        v = (v << 7) | (c & 0x7F) as u64;
        if c & 0x80 != 0 {
            return Some(v);
        }
    }
    None
}

/// The tag values of one index entry (`b` ends with the entry), read after its
/// control bytes at `at`, as described by the TAGX table (tag, values per entry,
/// mask, end flag).
fn entry_tags(
    b: &[u8],
    mut at: usize,
    control: &[u8],
    tagx: &[[u8; 4]],
) -> Option<HashMap<u8, Vec<u64>>> {
    let mut ci = 0usize;
    // (tag, number of values or None, byte length or None, values per entry)
    let mut wanted = Vec::new();
    for &[tag, per_entry, mask, end] in tagx {
        if end & 1 != 0 {
            ci += 1;
            continue;
        }
        let Some(&byte) = control.get(ci) else { break };
        let mut value = byte & mask;
        if value == 0 || mask == 0 {
            continue;
        }
        if value == mask {
            if mask.count_ones() > 1 {
                wanted.push((tag, None, Some(varint(b, &mut at)?), per_entry));
            } else {
                wanted.push((tag, Some(1u64), None, per_entry));
            }
        } else {
            let mut m = mask;
            while m & 1 == 0 {
                m >>= 1;
                value >>= 1;
            }
            wanted.push((tag, Some(value as u64), None, per_entry));
        }
    }
    let mut tags = HashMap::new();
    for (tag, count, bytes, per_entry) in wanted {
        let mut values = Vec::new();
        match (count, bytes) {
            (Some(n), _) => {
                for _ in 0..n * per_entry as u64 {
                    values.push(varint(b, &mut at)?);
                }
            }
            (None, Some(len)) => {
                let start = at;
                while ((at - start) as u64) < len {
                    values.push(varint(b, &mut at)?);
                }
            }
            _ => {}
        }
        tags.insert(tag, values);
    }
    Some(tags)
}

/// The table of contents (NCX index starting at record `first`): entries with a
/// position (tag 1) and a title (tag 3, in the CNCX records), in reading order.
/// Entries pointing back before an earlier one are dropped; of several at one
/// place, the last (deepest) names it. Empty when the index cannot be read.
fn read_ncx(
    record: &mut dyn FnMut(usize) -> Option<Vec<u8>>,
    first: usize,
    utf8: bool,
) -> Vec<NcxEntry> {
    read_ncx_entries(record, first, utf8).unwrap_or_default()
}

fn read_ncx_entries(
    record: &mut dyn FnMut(usize) -> Option<Vec<u8>>,
    first: usize,
    utf8: bool,
) -> Option<Vec<NcxEntry>> {
    let head = record(first)?;
    if head.get(0..4)? != b"INDX" {
        return None;
    }
    let header_len = be32(&head, 4)? as usize;
    let index_records = (be32(&head, 24)? as usize).min(1000);
    let cncx_records = (be32(&head, 52)? as usize).min(1000);
    let tagx = head.get(header_len..)?;
    if tagx.get(0..4)? != b"TAGX" {
        return None;
    }
    let tagx_len = (be32(tagx, 4)? as usize).min(tagx.len());
    let control_bytes = be32(tagx, 8)? as usize;
    let table: Vec<[u8; 4]> = tagx
        .get(12..tagx_len)?
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    let cncx: Vec<Vec<u8>> = (0..cncx_records)
        .map(|j| record(first + 1 + index_records + j).unwrap_or_default())
        .collect();
    let name = |offset: u64| -> Option<String> {
        let rec = cncx.get((offset / 0x10000) as usize)?;
        let mut at = (offset % 0x10000) as usize;
        let len = varint(rec, &mut at)? as usize;
        let bytes = rec.get(at..at + len)?;
        let text = if utf8 {
            String::from_utf8_lossy(bytes).into_owned()
        } else {
            WINDOWS_1252
                .decode_without_bom_handling(bytes)
                .0
                .into_owned()
        };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    };

    let mut out: Vec<NcxEntry> = Vec::new();
    for i in 0..index_records {
        let Some(rec) = record(first + 1 + i) else {
            continue;
        };
        if rec.get(0..4) != Some(b"INDX") {
            continue;
        }
        let (Some(idxt), Some(count)) = (be32(&rec, 20), be32(&rec, 24)) else {
            continue;
        };
        let idxt = idxt as usize;
        if rec.get(idxt..idxt + 4) != Some(b"IDXT") {
            continue;
        }
        let offsets: Vec<usize> = (0..count as usize)
            .map_while(|k| be16(&rec, idxt + 4 + 2 * k).map(usize::from))
            .collect();
        for (k, &start) in offsets.iter().enumerate() {
            let end = offsets.get(k + 1).copied().unwrap_or(idxt).min(rec.len());
            let Some(entry) = rec.get(..end) else {
                continue;
            };
            let Some(&ident_len) = entry.get(start) else {
                continue;
            };
            let at = start + 1 + ident_len as usize;
            let Some(control) = entry.get(at..at + control_bytes) else {
                continue;
            };
            let Some(tags) = entry_tags(entry, at + control_bytes, control, &table) else {
                continue;
            };
            let pos = tags.get(&1).and_then(|v| v.first().copied());
            let label = tags.get(&3).and_then(|v| v.first().copied()).and_then(name);
            let (Some(pos), Some(label)) = (pos, label) else {
                continue;
            };
            match out.last_mut() {
                Some(last) if last.pos == pos => last.label = label,
                Some(last) if last.pos > pos => {}
                _ => out.push(NcxEntry { pos, label }),
            }
        }
    }
    Some(out)
}

/// 1-based picture number of a MOBI link: `recindex:00012` (MOBI 7) or
/// `kindle:embed:000C?mime=image/jpeg` (KF8/AZW3, base 32).
fn image_number(link: &str) -> Option<usize> {
    if let Some(n) = link.strip_prefix("recindex:") {
        return n.trim().parse().ok();
    }
    let n = link.strip_prefix("kindle:embed:")?;
    let n = n.split(['?', '#']).next()?;
    usize::from_str_radix(n, 32).ok()
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
        sample_mobi_with_images(html, &[])
    }

    /// Same, followed by picture records.
    pub fn sample_mobi_with_images(html: &str, images: &[&[u8]]) -> Vec<u8> {
        sample_mobi_with_extra(html, images, &[])
    }

    /// Same, then `ncx` records named as the table of contents index.
    fn sample_mobi_with_extra(html: &str, images: &[&[u8]], ncx: &[Vec<u8>]) -> Vec<u8> {
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
        if !images.is_empty() {
            r0[0x6C..0x70].copy_from_slice(&(records.len() as u32 + 1).to_be_bytes());
        }
        if !ncx.is_empty() {
            let at = 1 + records.len() + images.len();
            r0[0xF4..0xF8].copy_from_slice(&(at as u32).to_be_bytes());
        }
        r0.extend_from_slice(title.as_bytes());
        let images: Vec<Vec<u8>> = images.iter().map(|i| i.to_vec()).collect();

        let all: Vec<&Vec<u8>> = std::iter::once(&r0)
            .chain(records.iter())
            .chain(images.iter())
            .chain(ncx.iter())
            .collect();
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

    fn forward_varint(mut v: u64) -> Vec<u8> {
        let mut out = vec![(v & 0x7F) as u8 | 0x80];
        v >>= 7;
        while v > 0 {
            out.insert(0, (v & 0x7F) as u8);
            v >>= 7;
        }
        out
    }

    /// NCX index records (header, one index record, one CNCX record) for `entries`
    /// (position in the text, title).
    fn ncx_records(entries: &[(u64, &str)]) -> Vec<Vec<u8>> {
        let mut cncx = Vec::new();
        let mut names = Vec::new();
        for (_, label) in entries {
            names.push(cncx.len() as u64);
            cncx.extend(forward_varint(label.len() as u64));
            cncx.extend_from_slice(label.as_bytes());
        }
        let mut head = vec![0u8; 0xC0];
        head[0..4].copy_from_slice(b"INDX");
        head[4..8].copy_from_slice(&0xC0u32.to_be_bytes());
        head[24..28].copy_from_slice(&1u32.to_be_bytes());
        head[52..56].copy_from_slice(&1u32.to_be_bytes());
        head.extend_from_slice(b"TAGX");
        head.extend_from_slice(&24u32.to_be_bytes());
        head.extend_from_slice(&1u32.to_be_bytes());
        head.extend_from_slice(&[1, 1, 0x01, 0, 3, 1, 0x02, 0, 0, 0, 0, 1]);

        let mut index = vec![0u8; 0xC0];
        index[0..4].copy_from_slice(b"INDX");
        let mut offsets = Vec::new();
        for (k, (pos, _)) in entries.iter().enumerate() {
            offsets.push(index.len() as u16);
            let ident = format!("{k:03}");
            index.push(ident.len() as u8);
            index.extend_from_slice(ident.as_bytes());
            index.push(0x03);
            index.extend(forward_varint(*pos));
            index.extend(forward_varint(names[k]));
        }
        let idxt = index.len() as u32;
        index[20..24].copy_from_slice(&idxt.to_be_bytes());
        index[24..28].copy_from_slice(&(entries.len() as u32).to_be_bytes());
        index.extend_from_slice(b"IDXT");
        for o in offsets {
            index.extend_from_slice(&o.to_be_bytes());
        }
        vec![head, index, cncx]
    }

    #[test]
    fn reads_the_table_of_contents_index() {
        let mut html = String::from("<html><body><p>Mục lục</p>");
        let mut starts = Vec::new();
        for c in 1..=3 {
            starts.push(html.len() as u64);
            // Headings are text when the book has a table of contents.
            html.push_str(&format!("<p><b>Chương {c}</b></p><h3>Tiểu mục</h3>"));
            for i in 0..150 {
                html.push_str(&format!("<p>Đoạn {c}.{i} có chữ tiếng Việt.</p>"));
            }
        }
        html.push_str("</body></html>");
        let ncx = ncx_records(&[
            (starts[0], "Quyển 1"),
            (starts[0], "Chương 1"),
            (starts[1], "Chương 2"),
            (starts[2], "Chương 3"),
        ]);
        let mut mobi = sample_mobi_with_extra(&html, &[], &ncx);
        let mut book = extract_mobi(Cursor::new(&mut mobi), &test_dir(), None).unwrap();
        let sections = book.sections();
        let labels: Vec<_> = sections.iter().map(|s| s.label.clone()).collect();
        assert_eq!(labels[0], None);
        let named: Vec<_> = labels.iter().flatten().cloned().collect();
        assert_eq!(named, vec!["Chương 1", "Chương 2", "Chương 3"]);
        let ch2 = labels
            .iter()
            .position(|l| l.as_deref() == Some("Chương 2"))
            .unwrap();
        let paras = book.load(ch2).unwrap().paragraphs;
        assert_eq!(paras[0], "Chương 2");
        assert_eq!(paras[1], "Tiểu mục");
        assert_eq!(paras[2], "Đoạn 2.0 có chữ tiếng Việt.");
        let prev = book.load(ch2 - 1).unwrap().paragraphs;
        assert_eq!(prev.last().unwrap(), "Đoạn 1.149 có chữ tiếng Việt.");
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

    #[test]
    fn links_picture_records() {
        let html =
            r#"<p>Bìa</p><img recindex="00002"/><img src="kindle:embed:0001?mime=image/png"/>"#;
        let mobi = sample_mobi_with_images(html, &[b"\x89PNG one", b"\xFF\xD8 two"]);
        let mut book = extract_mobi(Cursor::new(mobi), &test_dir(), None).unwrap();
        let paras = book.load(0).unwrap().paragraphs;
        assert_eq!(paras[1], "\u{FFFC}recindex:00002");
        assert_eq!(book.image("recindex:00002").unwrap().mime, "image/jpeg");
        assert_eq!(
            book.image("kindle:embed:0001?mime=image/png").unwrap().mime,
            "image/png"
        );
    }
}
