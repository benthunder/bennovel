//! Pictures inside books. A section's paragraph list holds a picture as a single
//! paragraph `IMAGE_MARK + key`; the bytes are fetched separately, only when the
//! reader shows that picture (`BookSource::image`). Formats laid out as pages (PDF,
//! DjVu) can also render whole pages (`BookSource::render_page`).

use super::BookError;
use serde::Serialize;

/// Starts a paragraph that stands for a picture (U+FFFC OBJECT REPLACEMENT CHARACTER).
pub const IMAGE_MARK: char = '\u{FFFC}';

/// Picture bytes with their media type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Image {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

impl Image {
    /// Wraps picture bytes, taking the type from their first bytes.
    pub fn sniff(bytes: Vec<u8>) -> Self {
        let mime = mime_of(&bytes);
        Self { bytes, mime }
    }
}

/// The paragraph standing for picture `key` (line breaks are not allowed in keys).
pub fn paragraph(key: &str) -> String {
    let mut p = String::with_capacity(key.len() + 3);
    p.push(IMAGE_MARK);
    p.extend(
        key.chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c }),
    );
    p
}

/// The picture key if `paragraph` stands for one.
pub fn key_of(paragraph: &str) -> Option<&str> {
    paragraph.strip_prefix(IMAGE_MARK)
}

/// Media type from the first bytes; `application/octet-stream` when unknown.
pub fn mime_of(b: &[u8]) -> &'static str {
    if b.starts_with(b"\x89PNG") {
        "image/png"
    } else if b.starts_with(b"\xFF\xD8") {
        "image/jpeg"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if b.starts_with(b"BM") {
        "image/bmp"
    } else if b.starts_with(b"\0\0\x01\0") {
        "image/x-icon"
    } else if looks_like_svg(b) {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

fn looks_like_svg(b: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&b[..b.len().min(512)]).to_ascii_lowercase();
    head.contains("<svg")
}

/// Encodes an RGBA bitmap as JPEG (pages have no transparency worth keeping).
pub fn encode_jpeg(width: u32, height: u32, rgba: &[u8]) -> Result<Image, BookError> {
    use ::image::codecs::jpeg::JpegEncoder;
    use ::image::ExtendedColorType;
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| {
            // Composite over white so transparent areas don't turn black.
            let a = p[3] as u16;
            [0, 1, 2].map(|i| ((p[i] as u16 * a + 255 * (255 - a)) / 255) as u8)
        })
        .collect();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85)
        .encode(&rgb, width, height, ExtendedColorType::Rgb8)
        .map_err(|e| BookError::Parse(e.to_string()))?;
    Ok(Image {
        bytes: out,
        mime: "image/jpeg",
    })
}

/// Decodes `%XX` escapes (URLs in documents, file names in Android URIs).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(b) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves a link found in document `base` (a path inside an archive, `/`-separated)
/// to an archive path: drops `#fragment`/`?query`, handles `..` and `./`.
pub fn resolve(base: &str, link: &str) -> String {
    let link = link.split(['#', '?']).next().unwrap_or_default();
    let link = percent_decode(link);
    let mut parts: Vec<&str> = if link.starts_with('/') {
        Vec::new()
    } else {
        let mut p: Vec<&str> = base.split('/').collect();
        p.pop(); // the document's own file name
        p
    };
    for seg in link.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join("/")
}

/// Bytes of a `data:` URI with base64 or percent-encoded content.
pub fn data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    if meta.ends_with(";base64") {
        base64(data)
    } else {
        Some(percent_decode(data).into_bytes())
    }
}

/// Lenient base64 decoder: whitespace is skipped, padding is optional.
pub fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Value of attribute `name` in a start tag's text (`img src="a.png" alt=x`).
pub fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let start = from + pos;
        from = start + name.len();
        // Must be a whole attribute name: preceded by whitespace (or ':' for xlink:href).
        let before = lower[..start].chars().next_back();
        if !matches!(before, Some(c) if c.is_whitespace() || c == ':') {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default(),
            _ => rest
                .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .next()
                .unwrap_or_default(),
        };
        return Some(super::text::decode_entities(value));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_round_trip() {
        let p = paragraph("OEBPS/images/a b.png");
        assert_eq!(key_of(&p), Some("OEBPS/images/a b.png"));
        assert_eq!(key_of("plain text"), None);
    }

    #[test]
    fn resolves_links() {
        assert_eq!(
            resolve("OEBPS/text/c1.xhtml", "../images/a.png"),
            "OEBPS/images/a.png"
        );
        assert_eq!(
            resolve("OEBPS/c1.xhtml", "img/a%20b.png#x"),
            "OEBPS/img/a b.png"
        );
        assert_eq!(resolve("a/b.html", "/root.png"), "root.png");
        assert_eq!(resolve("c1.xhtml", "./a.jpg"), "a.jpg");
    }

    #[test]
    fn reads_attributes_and_data() {
        assert_eq!(
            attr(r#"img class="x" src="a&amp;b.png" alt='y'"#, "src").as_deref(),
            Some("a&b.png")
        );
        assert_eq!(
            attr("image xlink:href=\"c.jpg\"/", "href").as_deref(),
            Some("c.jpg")
        );
        assert_eq!(attr("img data-src=\"no\"", "src"), None);
        assert_eq!(base64("aGVs\nbG8="), Some(b"hello".to_vec()));
        assert_eq!(data_uri("data:image/png;base64,aGk="), Some(b"hi".to_vec()));
        assert_eq!(mime_of(b"\x89PNG\r\n"), "image/png");
    }

    #[test]
    fn encodes_jpeg() {
        let img = encode_jpeg(2, 1, &[255, 0, 0, 255, 0, 0, 0, 0]).unwrap();
        assert_eq!(mime_of(&img.bytes), "image/jpeg");
    }
}
