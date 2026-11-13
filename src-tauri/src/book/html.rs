//! HTML and MHT/MHTML (web archive) files, extracted streaming into a [`Sink`].

use super::extract::{html_block, ExtractedBook, Sink};
use super::text::HtmlText;
use super::BookError;
use encoding_rs::{Encoding, UTF_8};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

const CHUNK: usize = 64 * 1024;

/// Charset from a BOM or a `<meta charset>` near the start, else UTF-8/GB18030 sniffing.
pub fn html_encoding(head: &[u8]) -> (&'static Encoding, usize) {
    if let Some((enc, bom)) = Encoding::for_bom(head) {
        return (enc, bom);
    }
    let ascii = String::from_utf8_lossy(&head[..head.len().min(4096)]).to_ascii_lowercase();
    if let Some(i) = ascii.find("charset=") {
        let label: String = ascii[i + 8..]
            .trim_start_matches(['"', '\''])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if let Some(enc) = Encoding::for_label(label.as_bytes()) {
            return (enc, 0);
        }
    }
    (super::txt::detect_encoding(head).0, 0)
}

/// Feeds decoded HTML text from `bytes` chunks to the parser.
struct Feeder {
    decoder: encoding_rs::Decoder,
    parser: HtmlText,
    text: String,
}

impl Feeder {
    fn new(enc: &'static Encoding) -> Self {
        Self {
            decoder: enc.new_decoder_without_bom_handling(),
            parser: HtmlText::default(),
            text: String::new(),
        }
    }

    fn bytes(&mut self, sink: &mut Sink, bytes: &[u8], last: bool) -> Result<(), BookError> {
        self.text.clear();
        self.text.reserve(
            self.decoder
                .max_utf8_buffer_length(bytes.len())
                .unwrap_or(bytes.len() * 3),
        );
        let _ = self.decoder.decode_to_string(bytes, &mut self.text, last);
        let mut err = Ok(());
        let mut emit = |b| {
            if err.is_ok() {
                err = html_block(sink, b);
            }
        };
        self.parser.feed(&self.text, &mut emit);
        if last {
            self.parser.finish(&mut emit);
        }
        err
    }
}

pub fn extract_html(
    mut r: impl Read,
    cache_dir: &Path,
    title: Option<String>,
    base: Option<&Path>,
) -> Result<ExtractedBook, BookError> {
    let mut sink = Sink::new(cache_dir)?;
    sink.title = title;
    let mut buf = vec![0u8; CHUNK];
    let n = read_full(&mut r, &mut buf)?;
    let (enc, bom) = html_encoding(&buf[..n]);
    let mut feeder = Feeder::new(enc);
    feeder.bytes(&mut sink, &buf[bom..n], false)?;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        feeder.bytes(&mut sink, &buf[..n], false)?;
    }
    feeder.bytes(&mut sink, &[], true)?;
    if let Some(base) = base {
        sink.load_linked_images(base)?;
    }
    sink.finish()
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

#[derive(PartialEq)]
enum TransferEncoding {
    Plain,
    QuotedPrintable,
    Base64,
}

/// MHT: a MIME message; the first `text/html` part is the page. Read line by line,
/// decoding quoted-printable or base64 as it goes.
pub fn extract_mht(
    r: impl Read,
    cache_dir: &Path,
    title: Option<String>,
) -> Result<ExtractedBook, BookError> {
    let mut sink = Sink::new(cache_dir)?;
    sink.title = title;
    let mut r = BufReader::new(r);
    let mut line = Vec::new();

    // Headers of the message, then of each part, until the HTML part's body starts.
    let mut boundary: Option<Vec<u8>> = None;
    let mut headers = String::new();
    let (te, enc) = loop {
        line.clear();
        if r.read_until(b'\n', &mut line)? == 0 {
            return Err(BookError::Parse("no HTML part in this web archive".into()));
        }
        let text = String::from_utf8_lossy(&line);
        let trimmed = text.trim_end();
        if let Some(b) = &boundary {
            if trimmed.as_bytes().starts_with(b"--") && trimmed.as_bytes()[2..].starts_with(b) {
                headers.clear();
                continue;
            }
        }
        if !trimmed.is_empty() {
            // Folded header lines start with whitespace.
            if trimmed.starts_with([' ', '\t']) {
                headers.push(' ');
            } else {
                headers.push('\n');
            }
            headers.push_str(trimmed.trim());
            continue;
        }
        if headers.is_empty() {
            continue;
        }
        let lower = headers.to_ascii_lowercase();
        let ctype = header(&lower, "content-type").unwrap_or_default();
        if ctype.starts_with("multipart/") {
            let raw = header(&headers, "content-type").unwrap_or_default();
            boundary = param(&raw, "boundary").map(String::into_bytes);
            headers.clear();
            continue;
        }
        if ctype.starts_with("text/html") {
            let te = match header(&lower, "content-transfer-encoding").as_deref() {
                Some("quoted-printable") => TransferEncoding::QuotedPrintable,
                Some("base64") => TransferEncoding::Base64,
                _ => TransferEncoding::Plain,
            };
            let enc = param(&ctype, "charset")
                .and_then(|c| Encoding::for_label(c.as_bytes()))
                .unwrap_or(UTF_8);
            break (te, enc);
        }
        headers.clear();
    };

    let mut feeder = Feeder::new(enc);
    let mut b64 = Vec::new();
    let mut decoded = Vec::new();
    loop {
        line.clear();
        if r.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if let Some(b) = &boundary {
            if line.starts_with(b"--") && line[2..].starts_with(b) {
                break;
            }
        }
        decoded.clear();
        match te {
            TransferEncoding::Plain => decoded.extend_from_slice(&line),
            TransferEncoding::QuotedPrintable => decode_qp_line(&line, &mut decoded),
            TransferEncoding::Base64 => {
                b64.extend(line.iter().copied().filter(|c| !c.is_ascii_whitespace()));
                let whole = b64.len() / 4 * 4;
                decode_base64(&b64[..whole], &mut decoded);
                b64.drain(..whole);
            }
        }
        feeder.bytes(&mut sink, &decoded, false)?;
    }
    feeder.bytes(&mut sink, &[], true)?;
    if let Some(b) = &boundary {
        read_mht_images(&mut r, b, &mut sink)?;
    }
    sink.finish()
}

/// Saves the picture parts that follow the page, keyed by Content-Location and
/// `cid:` Content-ID, then matches the page's links to them (by full URL, else by
/// file name, as pages often link relatively).
fn read_mht_images(
    r: &mut impl BufRead,
    boundary: &[u8],
    sink: &mut Sink,
) -> Result<(), BookError> {
    let mut line = Vec::new();
    let mut headers = String::new();
    let mut in_body = false;
    let mut body = Vec::new();
    let mut saved: Vec<String> = Vec::new();
    let mut finish_part = |headers: &str, body: &[u8], sink: &mut Sink| -> Result<(), BookError> {
        let lower = headers.to_ascii_lowercase();
        if !header(&lower, "content-type")
            .unwrap_or_default()
            .starts_with("image/")
        {
            return Ok(());
        }
        let mut bytes = Vec::new();
        match header(&lower, "content-transfer-encoding").as_deref() {
            Some("base64") => decode_base64(
                &body
                    .iter()
                    .copied()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect::<Vec<_>>(),
                &mut bytes,
            ),
            Some("quoted-printable") => {
                for l in body.split_inclusive(|&c| c == b'\n') {
                    decode_qp_line(l, &mut bytes);
                }
            }
            _ => bytes.extend_from_slice(body),
        }
        let mut keys = Vec::new();
        if let Some(loc) = header(headers, "content-location") {
            keys.push(loc);
        }
        if let Some(id) = header(headers, "content-id") {
            keys.push(format!("cid:{}", id.trim_matches(['<', '>'])));
        }
        for k in keys {
            sink.save_image(&k, &bytes)?;
            saved.push(k);
        }
        Ok(())
    };
    loop {
        line.clear();
        let eof = r.read_until(b'\n', &mut line)? == 0;
        let is_boundary = line.starts_with(b"--") && line[2..].starts_with(boundary);
        if eof || is_boundary {
            if in_body {
                finish_part(&headers, &body, sink)?;
            }
            if eof {
                break;
            }
            headers.clear();
            body.clear();
            in_body = false;
            continue;
        }
        if in_body {
            body.extend_from_slice(&line);
            continue;
        }
        let text = String::from_utf8_lossy(&line);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            in_body = !headers.is_empty();
        } else {
            headers.push(if trimmed.starts_with([' ', '\t']) {
                ' '
            } else {
                '\n'
            });
            headers.push_str(trimmed.trim());
        }
    }
    for key in sink.missing_images() {
        let name = super::image::resolve("", &key);
        let name = name.rsplit('/').next().unwrap_or_default().to_string();
        let found = saved.iter().find(|s| {
            let s = super::image::percent_decode(s.split(['#', '?']).next().unwrap_or_default());
            !name.is_empty() && (s == name || s.ends_with(&format!("/{name}")))
        });
        if let Some(existing) = found.cloned() {
            sink.alias(&key, &existing);
        }
    }
    Ok(())
}

fn header(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().to_string())
    })
}

fn param(value: &str, name: &str) -> Option<String> {
    value.split(';').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().trim_matches('"').to_string())
    })
}

fn decode_qp_line(line: &[u8], out: &mut Vec<u8>) {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let (body, soft) = match line.strip_suffix(b"=") {
        Some(b) => (b, true),
        None => (line, false),
    };
    let mut i = 0;
    while i < body.len() {
        if body[i] == b'=' {
            if let Some(v) = body
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(body[i]);
        i += 1;
    }
    if !soft {
        out.push(b'\n');
    }
}

fn decode_base64(input: &[u8], out: &mut Vec<u8>) {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    for quad in input.chunks_exact(4) {
        let pad = quad.iter().rev().take_while(|&&c| c == b'=').count();
        let mut n = 0u32;
        for &c in &quad[..4 - pad] {
            n = (n << 6) | val(c).unwrap_or(0);
        }
        n <<= 6 * pad as u32;
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..4 - pad]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::extract::test_dir;
    use super::super::BookSource;
    use super::*;
    use std::io::Cursor;

    fn texts(book: &mut ExtractedBook) -> Vec<String> {
        (0..book.sections().len())
            .flat_map(|i| book.load(i).unwrap().paragraphs)
            .collect()
    }

    #[test]
    fn html_headings_start_chapters() {
        let html = "<html><head><meta charset=\"windows-1252\"><title>x</title></head><body><h1>One</h1><p>caf\u{e9}</p><h2>Two</h2><p>b</p></body></html>";
        let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode(html);
        let mut book =
            extract_html(Cursor::new(bytes.into_owned()), &test_dir(), None, None).unwrap();
        let labels: Vec<_> = book.sections().into_iter().map(|s| s.label).collect();
        assert_eq!(labels, vec![Some("One".into()), Some("Two".into())]);
        assert_eq!(texts(&mut book), vec!["One", "café", "Two", "b"]);
    }

    #[test]
    fn mht_quoted_printable_and_base64() {
        let qp = "MIME-Version: 1.0\r\nContent-Type: multipart/related;\r\n\tboundary=\"XX\"\r\n\r\n--XX\r\nContent-Type: text/html; charset=\"utf-8\"\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<p>Xin ch=C3=A0o th=\r\n=E1=BA=BF gi=E1=BB=9Bi</p>\r\n--XX\r\nContent-Type: image/png\r\n\r\nzzz\r\n--XX--\r\n";
        let mut book = extract_mht(Cursor::new(qp.as_bytes().to_vec()), &test_dir(), None).unwrap();
        assert_eq!(texts(&mut book), vec!["Xin chào thế giới"]);

        let html = "<h1>Tựa</h1><p>Nội dung</p>";
        let mut b64 = Vec::new();
        for chunk in html.as_bytes().chunks(3) {
            let mut n = 0u32;
            for (i, &b) in chunk.iter().enumerate() {
                n |= (b as u32) << (16 - 8 * i);
            }
            let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            for i in 0..4 {
                b64.push(if i <= chunk.len() {
                    alpha[(n >> (18 - 6 * i) & 63) as usize]
                } else {
                    b'='
                });
            }
        }
        let mut msg =
            b"Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"
                .to_vec();
        for line in b64.chunks(10) {
            msg.extend_from_slice(line);
            msg.extend_from_slice(b"\r\n");
        }
        let mut book = extract_mht(Cursor::new(msg), &test_dir(), None).unwrap();
        assert_eq!(texts(&mut book), vec!["Tựa", "Nội dung"]);
    }

    #[test]
    fn html_and_mht_pictures() {
        let base = test_dir().join(format!("html-{}", std::process::id()));
        std::fs::create_dir_all(base.join("img")).unwrap();
        std::fs::write(base.join("img/a b.png"), b"\x89PNG").unwrap();
        let html = r#"<p>Một</p><img src="img/a%20b.png"><img src="data:image/gif;base64,R0lGOA=="><img src="http://x/y.png">"#;
        let mut book =
            extract_html(Cursor::new(html.as_bytes().to_vec()), &test_dir(), None, Some(&base))
                .unwrap();
        let paras = texts(&mut book);
        assert_eq!(paras.len(), 4);
        assert_eq!(book.image("img/a%20b.png").unwrap().mime, "image/png");
        let data_key = paras[2].trim_start_matches('\u{FFFC}');
        assert_eq!(book.image(data_key).unwrap().mime, "image/gif");
        assert!(book.image("http://x/y.png").is_err());

        let msg = "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/html; charset=utf-8\r\nContent-Location: http://site/page/index.html\r\n\r\n<p>Trang</p><img src=\"pics/p.png\"><img src=\"cid:c1\">\r\n--B\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nContent-Location: http://site/page/pics/p.png\r\n\r\niVBORw==\r\n--B\r\nContent-Type: image/jpeg\r\nContent-Transfer-Encoding: base64\r\nContent-ID: <c1>\r\n\r\n/9g=\r\n--B--\r\n";
        let mut book = extract_mht(Cursor::new(msg.as_bytes().to_vec()), &test_dir(), None).unwrap();
        assert_eq!(texts(&mut book).len(), 3);
        assert_eq!(book.image("pics/p.png").unwrap().mime, "image/png");
        assert_eq!(book.image("cid:c1").unwrap().mime, "image/jpeg");
    }
}
