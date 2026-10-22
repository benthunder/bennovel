use super::image::{self, Image};
use super::text::{decode_entities, html_to_paragraphs};
use super::{BookError, BookSource, Section, SectionMeta};
use ::epub::doc::{EpubDoc, NavPoint};
use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

/// One section: a whole spine file, or the part of it between two TOC anchors when
/// the TOC points at several chapters inside one file.
struct Part {
    spine: usize,
    /// Byte range in the file's text, when only part of it belongs here.
    range: Option<(usize, usize)>,
    label: Option<String>,
}

/// A TOC entry: the file it points to (path in the ZIP), its #fragment and title.
struct TocEntry {
    path: String,
    fragment: Option<String>,
    label: String,
}

/// EPUB read chapter by chapter from the ZIP; opening reads the manifest and TOC
/// (and only the files that hold several chapters, to find where they start).
pub struct EpubBook<R: Read + Seek> {
    doc: EpubDoc<R>,
    parts: Vec<Part>,
}

impl<R: Read + Seek + Send> EpubBook<R> {
    pub fn open(mut reader: R) -> Result<Self, BookError> {
        if drm_protected(&mut reader)? {
            return Err(BookError::Drm);
        }
        reader.seek(SeekFrom::Start(0))?;
        let mut doc = EpubDoc::from_reader(reader).map_err(|e| BookError::Parse(e.to_string()))?;
        let mut toc = Vec::new();
        flatten_ncx(&doc.toc, &mut toc);
        // EPUB 3 books may have only the navigation document.
        if toc.is_empty() {
            toc = nav_toc(&mut doc);
        }
        let parts = parts(&mut doc, toc);
        Ok(Self { doc, parts })
    }

    /// The text of spine file `index`.
    fn spine_text(&mut self, index: usize) -> Result<(String, String), BookError> {
        let idref = self
            .doc
            .spine
            .get(index)
            .ok_or(BookError::NoSection(index))?
            .idref
            .clone();
        let (html, _mime) = self
            .doc
            .get_resource_str(&idref)
            .ok_or_else(|| BookError::Parse(format!("missing chapter {idref}")))?;
        let path = self
            .doc
            .resources
            .get(&idref)
            .map(|r| r.path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        Ok((html, path))
    }
}

/// Opens an EPUB whose package or TOC file is not well-formed XML (a bare `&`, an
/// HTML entity such as `&nbsp;`): the ZIP is copied into memory with those files
/// repaired. Only used after a normal open failed.
pub fn open_repaired(mut reader: impl Read + Seek) -> Result<EpubBook<Cursor<Vec<u8>>>, BookError> {
    let err = |e: zip::result::ZipError| BookError::Parse(e.to_string());
    reader.seek(SeekFrom::Start(0))?;
    let mut zip = zip::ZipArchive::new(reader).map_err(err)?;
    // The XML files the EPUB reader parses, by entry index, when repairing changed them.
    let mut fixed = HashMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(err)?;
        let lower = entry.name().to_ascii_lowercase();
        if !(lower.ends_with(".opf")
            || lower.ends_with(".ncx")
            || lower == "meta-inf/container.xml")
        {
            continue;
        }
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_ok() {
            let repaired = fix_xml_entities(&text);
            if repaired != text {
                fixed.insert(i, repaired);
            }
        }
    }
    if fixed.is_empty() {
        return Err(BookError::Parse("broken EPUB".into()));
    }
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        match fixed.remove(&i) {
            Some(text) => {
                let name = zip.by_index_raw(i).map_err(err)?.name().to_string();
                out.start_file(name, zip::write::SimpleFileOptions::default())
                    .map_err(err)?;
                out.write_all(text.as_bytes())?;
            }
            None => out
                .raw_copy_file(zip.by_index_raw(i).map_err(err)?)
                .map_err(err)?,
        }
    }
    let bytes = out.finish().map_err(err)?.into_inner();
    EpubBook::open(Cursor::new(bytes))
}

/// `&` that does not start an XML entity becomes `&amp;`; HTML-only entities
/// (`&nbsp;`, `&hellip;`…) become their characters.
fn fix_xml_entities(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let entity = rest[1..]
            .find(';')
            .filter(|&semi| semi <= 10)
            .map(|semi| &rest[..semi + 2]);
        match entity {
            Some(e)
                if matches!(e, "&amp;" | "&lt;" | "&gt;" | "&quot;" | "&apos;")
                    || e.starts_with("&#") =>
            {
                out.push_str(e);
                rest = &rest[e.len()..];
            }
            Some(e) if decode_entities(e) != e => {
                out.push_str(&decode_entities(e));
                rest = &rest[e.len()..];
            }
            _ => {
                out.push_str("&amp;");
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Encrypted content (Adobe ADEPT, Readium LCP, Apple FairPlay…), as opposed to the
/// font obfuscation many DRM-free books use. Moon+ Reader checks the same files.
fn drm_protected<R: Read + Seek>(reader: &mut R) -> Result<bool, BookError> {
    const FONT_OBFUSCATION: &[&str] = &[
        "http://www.idpf.org/2008/embedding",
        "http://ns.adobe.com/pdf/enc#RC",
    ];
    let Ok(mut zip) = zip::ZipArchive::new(&mut *reader) else {
        // Not readable as a ZIP here; the EPUB reader reports the real error.
        return Ok(false);
    };
    for name in [
        "META-INF/rights.xml",
        "META-INF/license.lcpl",
        "META-INF/sinf.xml",
    ] {
        if zip.by_name(name).is_ok() {
            return Ok(true);
        }
    }
    let mut xml = String::new();
    match zip.by_name("META-INF/encryption.xml") {
        Ok(mut f) => {
            let _ = f.read_to_string(&mut xml);
        }
        Err(_) => return Ok(false),
    }
    let mut rest = xml.as_str();
    while let Some(i) = rest.find("Algorithm=") {
        rest = &rest[i + "Algorithm=".len()..];
        let quote = rest.chars().next().unwrap_or('"');
        let value = rest[1..].split(quote).next().unwrap_or_default();
        if !FONT_OBFUSCATION.contains(&value) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// NCX entries in reading order, nested ones after their parent.
fn flatten_ncx(points: &[NavPoint], out: &mut Vec<TocEntry>) {
    for p in points {
        let src = p.content.to_string_lossy().replace('\\', "/");
        let (file, fragment) = split_fragment(&src);
        let label = p.label.trim();
        if !label.is_empty() {
            out.push(TocEntry {
                path: image::resolve("", file),
                fragment,
                label: label.to_string(),
            });
        }
        flatten_ncx(&p.children, out);
    }
}

fn split_fragment(link: &str) -> (&str, Option<String>) {
    match link.split_once('#') {
        Some((file, f)) if !f.is_empty() => (file, Some(image::percent_decode(f))),
        Some((file, _)) => (file, None),
        None => (link, None),
    }
}

/// Entries of the `<nav epub:type="toc">` of an EPUB 3 navigation document.
fn nav_toc<R: Read + Seek>(doc: &mut EpubDoc<R>) -> Vec<TocEntry> {
    let Some(id) = doc.get_nav_id() else {
        return Vec::new();
    };
    let Some(path) = doc
        .resources
        .get(&id)
        .map(|r| r.path.to_string_lossy().replace('\\', "/"))
    else {
        return Vec::new();
    };
    let Some((html, _)) = doc.get_resource_str(&id) else {
        return Vec::new();
    };
    nav_entries(&html, &path)
}

/// Links inside the TOC `<nav>` of navigation document `html` (at ZIP path `path`).
fn nav_entries(html: &str, path: &str) -> Vec<TocEntry> {
    let lower = html.to_ascii_lowercase();
    // The <nav> marked as the TOC, else the first <nav>.
    let navs: Vec<usize> = lower.match_indices("<nav").map(|(i, _)| i).collect();
    let start = navs
        .iter()
        .copied()
        .find(|&i| {
            let tag_end = lower[i..].find('>').map_or(lower.len(), |e| i + e);
            let tag = &lower[i..tag_end];
            tag.contains("type=\"toc\"") || tag.contains("type='toc'")
        })
        .or_else(|| navs.first().copied());
    let Some(start) = start else {
        return Vec::new();
    };
    let end = lower[start..]
        .find("</nav")
        .map_or(lower.len(), |e| start + e);
    let section = &html[start..end];
    let lower_section = &lower[start..end];

    let mut out = Vec::new();
    let mut at = 0;
    while let Some(a) = lower_section[at..].find("<a").map(|i| at + i) {
        let Some(tag_end) = section[a..].find('>').map(|e| a + e) else {
            break;
        };
        let close = lower_section[tag_end..]
            .find("</a")
            .map_or(section.len(), |e| tag_end + e);
        at = close;
        let tag = &section[a + 1..tag_end];
        // "<abbr>" and friends are not links.
        if !tag[1..].starts_with(char::is_whitespace) {
            continue;
        }
        let Some(href) = image::attr(tag, "href") else {
            continue;
        };
        let label = html_to_paragraphs(&section[tag_end + 1..close], |_| None).join(" ");
        if label.trim().is_empty() || href.contains("://") {
            continue;
        }
        let (file, fragment) = split_fragment(&href);
        let file_path = if file.is_empty() {
            path.to_string()
        } else {
            image::resolve(path, file)
        };
        out.push(TocEntry {
            path: file_path,
            fragment,
            label: label.trim().to_string(),
        });
    }
    out
}

/// Sections in spine order. A file the TOC points into at several anchors is cut
/// at those anchors, each piece named by its entry; any other file is one section,
/// named by the first entry pointing into it.
fn parts<R: Read + Seek>(doc: &mut EpubDoc<R>, toc: Vec<TocEntry>) -> Vec<Part> {
    let spine_of: HashMap<String, usize> = doc
        .spine
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let path = doc
                .resources
                .get(&item.idref)?
                .path
                .to_string_lossy()
                .replace('\\', "/");
            Some((image::resolve("", &path), i))
        })
        .collect();
    let mut by_spine: HashMap<usize, Vec<TocEntry>> = HashMap::new();
    for e in toc {
        if let Some(&i) = spine_of.get(&e.path) {
            let entries = by_spine.entry(i).or_default();
            if !entries.iter().any(|x| x.fragment == e.fragment) {
                entries.push(e);
            }
        }
    }

    let mut parts = Vec::with_capacity(doc.spine.len());
    for i in 0..doc.spine.len() {
        let entries = by_spine.remove(&i).unwrap_or_default();
        if entries.len() >= 2 {
            let idref = doc.spine[i].idref.clone();
            if let Some((html, _)) = doc.get_resource_str(&idref) {
                let split = split_at_anchors(&html, &entries);
                if split.len() >= 2 {
                    parts.extend(split.into_iter().map(|(range, label)| Part {
                        spine: i,
                        range: Some(range),
                        label: Some(label),
                    }));
                    continue;
                }
            }
        }
        parts.push(Part {
            spine: i,
            range: None,
            label: entries.into_iter().next().map(|e| e.label),
        });
    }
    parts
}

/// Byte ranges of `html` starting at each entry's anchor (the first piece starts at
/// the beginning of the file), keeping only anchors found in increasing order.
fn split_at_anchors(html: &str, entries: &[TocEntry]) -> Vec<((usize, usize), String)> {
    let mut starts: Vec<(usize, &str)> = Vec::new();
    for (n, e) in entries.iter().enumerate() {
        let pos = if n == 0 {
            Some(0)
        } else {
            e.fragment.as_deref().and_then(|f| anchor_pos(html, f))
        };
        if let Some(pos) = pos.filter(|&p| starts.last().is_none_or(|&(last, _)| p > last)) {
            starts.push((pos, &e.label));
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &(start, label))| {
            let end = starts.get(n + 1).map_or(html.len(), |&(s, _)| s);
            ((start, end), label.to_string())
        })
        .collect()
}

/// Where the tag carrying `id="fragment"` (or `name=`) starts.
fn anchor_pos(html: &str, fragment: &str) -> Option<usize> {
    for attr in ["id", "name"] {
        for q in ['"', '\''] {
            let needle = format!("{attr}={q}{fragment}{q}");
            if let Some(at) = html.match_indices(&needle).map(|(i, _)| i).find(|&i| {
                // A whole attribute name: "data-id=" is not "id=".
                html[..i].ends_with(char::is_whitespace)
            }) {
                return html[..at].rfind('<');
            }
        }
    }
    None
}

impl<R: Read + Seek + Send> BookSource for EpubBook<R> {
    fn title(&self) -> Option<String> {
        self.doc.get_title()
    }

    fn author(&self) -> Option<String> {
        let names: Vec<&str> = self
            .doc
            .metadata
            .iter()
            .filter(|m| m.property == "creator")
            .map(|m| m.value.trim())
            .filter(|v| !v.is_empty())
            .collect();
        (!names.is_empty()).then(|| names.join(", "))
    }

    fn description(&self) -> Option<String> {
        let raw = &self.doc.mdata("description")?.value;
        // Often HTML, sometimes escaped HTML.
        let html = if raw.contains("&lt;") {
            decode_entities(raw)
        } else {
            raw.clone()
        };
        let text = html_to_paragraphs(&html, |_| None).join("\n\n");
        (!text.is_empty()).then_some(text)
    }

    fn sections(&self) -> Vec<SectionMeta> {
        self.parts
            .iter()
            .map(|p| SectionMeta {
                label: p.label.clone(),
                pages: None,
                number: None,
            })
            .collect()
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let part = self.parts.get(index).ok_or(BookError::NoSection(index))?;
        let range = part.range;
        let (html, chapter) = self.spine_text(part.spine)?;
        let text = match range {
            Some((start, end)) => html.get(start..end.min(html.len())).unwrap_or_default(),
            None => &html,
        };
        // Picture links are relative to the chapter file; keys are paths in the ZIP.
        Ok(Section {
            index,
            paragraphs: html_to_paragraphs(text, |src| {
                (!src.contains("://") && !src.starts_with("data:"))
                    .then(|| image::resolve(&chapter, src))
            }),
        })
    }

    fn image(&mut self, key: &str) -> Result<Image, BookError> {
        self.doc
            .get_resource_by_path(key)
            .map(Image::sniff)
            .ok_or_else(|| BookError::NoImage(key.to_string()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    /// A minimal EPUB 2 with `chapters` chapters.
    pub fn sample_epub(chapters: usize) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        let mut put = |name: &str, body: &str| {
            zip.start_file(name, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        };
        put("mimetype", "application/epub+zip");
        put(
            "META-INF/container.xml",
            r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        );
        let items: String = (1..=chapters)
            .map(|i| {
                format!(r#"<item id="c{i}" href="c{i}.xhtml" media-type="application/xhtml+xml"/>"#)
            })
            .collect();
        let spine: String = (1..=chapters)
            .map(|i| format!(r#"<itemref idref="c{i}"/>"#))
            .collect();
        put(
            "OEBPS/content.opf",
            &format!(
                r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Sách thử</dc:title><dc:identifier id="id">x</dc:identifier></metadata><manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>{items}</manifest><spine toc="ncx">{spine}</spine></package>"#
            ),
        );
        let nav: String = (1..=chapters)
            .map(|i| format!(r#"<navPoint id="n{i}" playOrder="{i}"><navLabel><text>Chương {i}</text></navLabel><content src="c{i}.xhtml#top"/></navPoint>"#))
            .collect();
        put(
            "OEBPS/toc.ncx",
            &format!(
                r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>{nav}</navMap></ncx>"#
            ),
        );
        for i in 1..=chapters {
            put(
                &format!("OEBPS/c{i}.xhtml"),
                &format!("<html><body><h1>Chương {i}</h1><p>Nội dung {i}.</p></body></html>"),
            );
        }
        put(
            "OEBPS/text/pic.xhtml",
            r#"<html><body><p>Trước</p><svg><image xlink:href="../img/cover.png"/></svg><img src="../img/a.png"/><img src="http://x/y.png"/></body></html>"#,
        );
        put("OEBPS/img/a.png", "\u{89}PNG fake");
        zip.finish().unwrap().into_inner()
    }

    /// An EPUB from `files` (path, text) plus the mimetype and a container pointing
    /// at `OEBPS/content.opf`.
    fn build(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        let mut put = |name: &str, body: &str| {
            zip.start_file(name, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        };
        put("mimetype", "application/epub+zip");
        put(
            "META-INF/container.xml",
            r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        );
        for (name, body) in files {
            put(name, body);
        }
        zip.finish().unwrap().into_inner()
    }

    fn opf(version: &str, metadata: &str, manifest: &str, spine: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier>{metadata}</metadata><manifest>{manifest}</manifest><spine{spine}</spine></package>"#
        )
    }

    fn labels<R: Read + Seek + Send>(book: &EpubBook<R>) -> Vec<Option<String>> {
        book.sections().into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn reads_the_epub3_navigation_document() {
        let epub = build(&[
            (
                "OEBPS/content.opf",
                &opf(
                    "3.0",
                    "<dc:title>Ba</dc:title>",
                    r#"<item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/><item id="a" href="text/a.xhtml" media-type="application/xhtml+xml"/><item id="b" href="text/b.xhtml" media-type="application/xhtml+xml"/>"#,
                    r#"><itemref idref="a"/><itemref idref="b"/>"#,
                ),
            ),
            (
                "OEBPS/nav.xhtml",
                r#"<html><body><nav epub:type="landmarks"><a href="text/b.xhtml">Sai</a></nav><nav epub:type="toc"><ol><li><a href="text/a.xhtml">Chương <b>1</b></a></li><li><a href="text/b.xhtml#x">Chương 2</a></li></ol></nav></body></html>"#,
            ),
            ("OEBPS/text/a.xhtml", "<html><body><p>A</p></body></html>"),
            ("OEBPS/text/b.xhtml", "<html><body><p>B</p></body></html>"),
        ]);
        let book = EpubBook::open(Cursor::new(epub)).unwrap();
        assert_eq!(
            labels(&book),
            vec![Some("Chương 1".into()), Some("Chương 2".into())]
        );
    }

    #[test]
    fn splits_a_file_holding_several_chapters() {
        let ncx = r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap><navPoint id="n1" playOrder="1"><navLabel><text>Một</text></navLabel><content src="all.xhtml"/></navPoint><navPoint id="n2" playOrder="2"><navLabel><text>Hai</text></navLabel><content src="all.xhtml#c2"/></navPoint><navPoint id="n3" playOrder="3"><navLabel><text>Ba</text></navLabel><content src="all.xhtml#c3"/></navPoint><navPoint id="n4" playOrder="4"><navLabel><text>Bốn</text></navLabel><content src="end.xhtml"/></navPoint></navMap></ncx>"#;
        let epub = build(&[
            (
                "OEBPS/content.opf",
                &opf(
                    "2.0",
                    "<dc:title>T</dc:title>",
                    r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="all" href="all.xhtml" media-type="application/xhtml+xml"/><item id="end" href="end.xhtml" media-type="application/xhtml+xml"/>"#,
                    r#" toc="ncx"><itemref idref="all"/><itemref idref="end"/>"#,
                ),
            ),
            ("OEBPS/toc.ncx", ncx),
            (
                "OEBPS/all.xhtml",
                r#"<html><head><title>x</title></head><body><h1>Một</h1><p>một</p><h1 id="c2">Hai</h1><p>hai</p><div data-id="c3"></div><h1 id='c3'>Ba</h1><p>ba</p></body></html>"#,
            ),
            ("OEBPS/end.xhtml", "<html><body><p>hết</p></body></html>"),
        ]);
        let mut book = EpubBook::open(Cursor::new(epub)).unwrap();
        assert_eq!(
            labels(&book),
            ["Một", "Hai", "Ba", "Bốn"]
                .map(|l| Some(l.to_string()))
                .to_vec()
        );
        assert_eq!(book.load(0).unwrap().paragraphs, ["Một", "một"]);
        assert_eq!(book.load(1).unwrap().paragraphs, ["Hai", "hai"]);
        assert_eq!(book.load(2).unwrap().paragraphs, ["Ba", "ba"]);
        assert_eq!(book.load(3).unwrap().paragraphs, ["hết"]);
    }

    #[test]
    fn reads_authors_and_description() {
        let epub = build(&[
            (
                "OEBPS/content.opf",
                &opf(
                    "2.0",
                    "<dc:title>T</dc:title><dc:creator>Kim Dung</dc:creator><dc:creator>Người dịch</dc:creator><dc:description>&lt;p&gt;Đoạn một.&lt;/p&gt;&lt;p&gt;Đoạn hai.&lt;/p&gt;</dc:description>",
                    r#"<item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>"#,
                    r#"><itemref idref="a"/>"#,
                ),
            ),
            ("OEBPS/a.xhtml", "<html><body><p>A</p></body></html>"),
        ]);
        let book = EpubBook::open(Cursor::new(epub)).unwrap();
        assert_eq!(book.author().as_deref(), Some("Kim Dung, Người dịch"));
        assert_eq!(
            book.description().as_deref(),
            Some("Đoạn một.\n\nĐoạn hai.")
        );
    }

    #[test]
    fn rejects_drm_but_not_font_obfuscation() {
        let book = |algorithm: &str| {
            let enc = format!(
                r#"<encryption><EncryptedData><EncryptionMethod Algorithm="{algorithm}"/></EncryptedData></encryption>"#
            );
            let mut files: Vec<(String, String)> = vec![
                (
                    "OEBPS/content.opf".into(),
                    opf(
                        "2.0",
                        "<dc:title>T</dc:title>",
                        r#"<item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>"#,
                        r#"><itemref idref="a"/>"#,
                    ),
                ),
                (
                    "OEBPS/a.xhtml".into(),
                    "<html><body><p>A</p></body></html>".into(),
                ),
                ("META-INF/encryption.xml".into(), enc),
            ];
            files.sort();
            let refs: Vec<(&str, &str)> = files
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            EpubBook::open(Cursor::new(build(&refs)))
        };
        assert!(matches!(
            book("http://www.w3.org/2001/04/xmlenc#aes128-cbc"),
            Err(BookError::Drm)
        ));
        assert!(book("http://www.idpf.org/2008/embedding").is_ok());
    }

    #[test]
    fn repairs_a_bare_ampersand_in_the_package() {
        let epub = build(&[
            (
                "OEBPS/content.opf",
                &opf(
                    "2.0",
                    "<dc:title>Tom & Jerry&nbsp;</dc:title>",
                    r#"<item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>"#,
                    r#"><itemref idref="a"/>"#,
                ),
            ),
            ("OEBPS/a.xhtml", "<html><body><p>A</p></body></html>"),
        ]);
        assert!(EpubBook::open(Cursor::new(epub.clone())).is_err());
        let mut book = open_repaired(Cursor::new(epub)).unwrap();
        assert_eq!(book.title().as_deref().map(str::trim), Some("Tom & Jerry"));
        assert_eq!(book.load(0).unwrap().paragraphs, ["A"]);
        assert_eq!(
            fix_xml_entities("a &amp; b &#20013; &lt;"),
            "a &amp; b &#20013; &lt;"
        );
    }

    #[test]
    fn reads_chapters_on_demand() {
        let mut book = EpubBook::open(Cursor::new(sample_epub(3))).unwrap();
        assert_eq!(book.title().as_deref(), Some("Sách thử"));
        let labels: Vec<_> = book.sections().into_iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            vec![
                Some("Chương 1".into()),
                Some("Chương 2".into()),
                Some("Chương 3".into())
            ]
        );
        assert_eq!(
            book.load(1).unwrap().paragraphs,
            vec!["Chương 2", "Nội dung 2."]
        );
        assert!(matches!(book.load(3), Err(BookError::NoSection(3))));
    }

    #[test]
    fn pictures_resolve_against_the_chapter() {
        let mut book = EpubBook::open(Cursor::new(sample_epub(1))).unwrap();
        let html = book
            .doc
            .get_resource_str_by_path("OEBPS/text/pic.xhtml")
            .unwrap();
        let paras = html_to_paragraphs(&html, |src| {
            (!src.contains("://")).then(|| image::resolve("OEBPS/text/pic.xhtml", src))
        });
        assert_eq!(
            paras,
            vec![
                "Trước".to_string(),
                image::paragraph("OEBPS/img/cover.png"),
                image::paragraph("OEBPS/img/a.png")
            ]
        );
        assert!(book
            .image("OEBPS/img/a.png")
            .unwrap()
            .bytes
            .ends_with(b"PNG fake"));
        assert!(book.image("OEBPS/img/missing.png").is_err());
    }
}
