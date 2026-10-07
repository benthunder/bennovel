//! RTF, tokenized streaming: control words, groups, `\'hh` bytes in the document
//! code page (`\ansicpg`) and `\uN` Unicode characters.

use super::extract::{ExtractedBook, Sink};
use super::BookError;
use encoding_rs::{Encoding, WINDOWS_1252};
use std::io::{BufReader, Bytes, Read};
use std::iter::Peekable;
use std::path::Path;

/// Groups whose content is not body text.
const SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "footnote",
    "fldinst",
    "themedata",
    "colorschememapping",
    "datastore",
    "latentstyles",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "generator",
    "xmlnstbl",
    "mmathPr",
    "filetbl",
    "revtbl",
    "pgdsctbl",
    "bkmkstart",
    "bkmkend",
    "comment",
    "nonshppict",
];

#[derive(Clone, Copy)]
struct Group {
    skip: bool,
    /// Characters to drop after `\uN` (the ANSI fallback).
    uc: usize,
}

struct Rtf<'a> {
    sink: &'a mut Sink,
    enc: &'static Encoding,
    para: String,
    /// Raw code-page bytes waiting to be decoded together (double-byte code pages).
    bytes: Vec<u8>,
    skip_chars: usize,
}

impl Rtf<'_> {
    fn flush_bytes(&mut self) {
        if !self.bytes.is_empty() {
            let (text, _) = self.enc.decode_without_bom_handling(&self.bytes);
            self.para.push_str(&text);
            self.bytes.clear();
        }
    }

    fn text(&mut self, c: char) {
        self.flush_bytes();
        self.para.push(c);
    }

    fn byte(&mut self, b: u8) {
        if self.skip_chars > 0 {
            self.skip_chars -= 1;
        } else {
            self.bytes.push(b);
        }
    }

    fn par(&mut self) -> Result<(), BookError> {
        self.flush_bytes();
        let p = std::mem::take(&mut self.para);
        self.sink.paragraph(&p)
    }
}

pub fn extract_rtf(
    r: impl Read,
    cache_dir: &Path,
    title: Option<String>,
) -> Result<ExtractedBook, BookError> {
    let mut sink = Sink::new(cache_dir)?;
    sink.title = title;
    let mut input = BufReader::new(r).bytes().peekable();
    let mut doc = Rtf {
        sink: &mut sink,
        enc: WINDOWS_1252,
        para: String::new(),
        bytes: Vec::new(),
        skip_chars: 0,
    };
    let mut stack: Vec<Group> = Vec::new();
    let mut g = Group { skip: false, uc: 1 };
    // `\*` marks the group being opened as skippable if its word is unknown.
    let mut star = false;

    while let Some(b) = input.next() {
        let b = b?;
        match b {
            b'{' => {
                doc.flush_bytes();
                stack.push(g);
                star = false;
            }
            b'}' => {
                doc.flush_bytes();
                g = stack.pop().unwrap_or(g);
                star = false;
            }
            b'\\' => {
                let Some(next) = input.next() else { break };
                let next = next?;
                if next.is_ascii_alphabetic() {
                    let (word, param) = control_word(next, &mut input)?;
                    if g.skip {
                        continue;
                    }
                    if star || SKIP_DESTINATIONS.contains(&word.as_str()) {
                        g.skip = true;
                        star = false;
                        continue;
                    }
                    match word.as_str() {
                        "par" | "sect" | "page" | "row" => doc.par()?,
                        "line" | "tab" | "cell" => doc.text(' '),
                        "ansicpg" => {
                            if let Some(enc) = param.and_then(code_page) {
                                doc.enc = enc;
                            }
                        }
                        "uc" => g.uc = param.unwrap_or(1).max(0) as usize,
                        "u" => {
                            let v = param.unwrap_or(0);
                            let v = if v < 0 { v + 65536 } else { v } as u32;
                            doc.text(char::from_u32(v).unwrap_or('\u{fffd}'));
                            doc.skip_chars = g.uc;
                        }
                        "emdash" => doc.text('—'),
                        "endash" => doc.text('–'),
                        "lquote" => doc.text('‘'),
                        "rquote" => doc.text('’'),
                        "ldblquote" => doc.text('“'),
                        "rdblquote" => doc.text('”'),
                        "bullet" => doc.text('•'),
                        _ => {}
                    }
                } else {
                    match next {
                        b'*' => star = true,
                        b'\'' => {
                            let hex: Vec<u8> = (0..2)
                                .filter_map(|_| input.next().and_then(Result::ok))
                                .collect();
                            if !g.skip {
                                if let Some(v) = std::str::from_utf8(&hex)
                                    .ok()
                                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                                {
                                    doc.byte(v);
                                }
                            }
                        }
                        b'\n' | b'\r' if !g.skip => doc.par()?,
                        b'~' if !g.skip => doc.text(' '),
                        b'_' if !g.skip => doc.text('-'),
                        b'{' | b'}' | b'\\' if !g.skip => {
                            if doc.skip_chars > 0 {
                                doc.skip_chars -= 1;
                            } else {
                                doc.text(next as char);
                            }
                        }
                        _ => {}
                    }
                }
            }
            b'\r' | b'\n' => {}
            _ if g.skip => {}
            _ => {
                star = false;
                if doc.skip_chars > 0 {
                    doc.skip_chars -= 1;
                } else if b < 0x80 {
                    doc.text(b as char);
                } else {
                    doc.byte(b);
                }
            }
        }
    }
    doc.par()?;
    sink.finish()
}

fn control_word<R: Read>(
    first: u8,
    input: &mut Peekable<Bytes<R>>,
) -> Result<(String, Option<i32>), BookError> {
    let mut word = String::from(first as char);
    while let Some(Ok(c)) = input.peek() {
        if c.is_ascii_alphabetic() {
            word.push(*c as char);
            input.next();
        } else {
            break;
        }
    }
    let mut num = String::new();
    if let Some(Ok(b'-')) = input.peek() {
        num.push('-');
        input.next();
    }
    while let Some(Ok(c)) = input.peek() {
        if c.is_ascii_digit() {
            num.push(*c as char);
            input.next();
        } else {
            break;
        }
    }
    // One space after a control word is its delimiter.
    if let Some(Ok(b' ')) = input.peek() {
        input.next();
    }
    Ok((word, num.parse().ok()))
}

fn code_page(cp: i32) -> Option<&'static Encoding> {
    let label = match cp {
        936 => "gbk".to_string(),
        950 => "big5".to_string(),
        932 => "shift_jis".to_string(),
        949 => "euc-kr".to_string(),
        65001 => "utf-8".to_string(),
        n => format!("windows-{n}"),
    };
    Encoding::for_label(label.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::super::extract::test_dir;
    use super::super::BookSource;
    use super::*;
    use std::io::Cursor;

    #[test]
    fn rtf_text_unicode_and_code_page() {
        let rtf = r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Times;}}{\*\generator Foo;}{\info{\title T}}
\pard Caf\'e9 \b bold\b0\par
Vi\u7879?t Nam\par
{\*\unknowndest hidden}\ldblquote x\rdblquote\par}";
        let mut book =
            extract_rtf(Cursor::new(rtf.as_bytes().to_vec()), &test_dir(), None).unwrap();
        assert_eq!(
            book.load(0).unwrap().paragraphs,
            vec!["Café bold", "Việt Nam", "“x”"]
        );
    }

    #[test]
    fn rtf_double_byte_code_page() {
        let rtf = r"{\rtf1\ansi\ansicpg936 \'b5\'da\'d2\'bb\'d5\'c2\par}";
        let mut book =
            extract_rtf(Cursor::new(rtf.as_bytes().to_vec()), &test_dir(), None).unwrap();
        assert_eq!(book.load(0).unwrap().paragraphs, vec!["第一章"]);
    }
}
