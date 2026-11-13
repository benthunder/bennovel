//! XML-based books, parsed streaming: FB2, DOCX (Word) and ODT (OpenDocument).

use super::extract::{ExtractedBook, Sink};
use super::BookError;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::Reader;
use std::io::{BufRead, BufReader, Read, Seek};
use std::path::Path;

fn xml_err(e: impl std::fmt::Display) -> BookError {
    BookError::Parse(e.to_string())
}

/// Text of an entity reference such as `&amp;` or `&#233;`.
fn entity(r: &BytesRef) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match &**r {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        _ => "",
    }
    .to_string()
}

fn attr(e: &BytesStart, local: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let key = a.key.local_name();
        (key.as_ref() == local)
            .then(|| {
                quick_xml::escape::unescape(&a.value)
                    .map(|v| v.into_owned())
                    .ok()
            })
            .flatten()
    })
}

/// Walks the XML events, handing each start/end/text to `on`.
enum Ev<'a> {
    Start(&'a BytesStart<'a>, bool),
    End(&'a str),
    Text(&'a str),
}

fn walk(r: impl BufRead, mut on: impl FnMut(Ev) -> Result<(), BookError>) -> Result<(), BookError> {
    // Transcodes documents declared as e.g. windows-1251 (common for FB2) to UTF-8.
    let mut reader = Reader::from_reader(quick_xml::encoding::DecodingReader::new(r));
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).map_err(xml_err)? {
            Event::Start(e) => on(Ev::Start(&e, false))?,
            Event::Empty(e) => {
                on(Ev::Start(&e, true))?;
                on(Ev::End(e.local_name().as_ref()))?;
            }
            Event::End(e) => on(Ev::End(e.local_name().as_ref()))?,
            Event::Text(t) => on(Ev::Text(&t.xml10_content()))?,
            Event::CData(t) => on(Ev::Text(&t.xml10_content()))?,
            Event::GeneralRef(r) => on(Ev::Text(&entity(&r)))?,
            Event::Decl(d) => {
                if let Some(enc) = d.encoder() {
                    reader.get_mut().set_encoding(enc);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// FictionBook 2: `<section><title>` names chapters; `<binary>` holds the pictures.
pub fn extract_fb2(
    r: impl Read,
    cache_dir: &Path,
    fallback_title: Option<String>,
) -> Result<ExtractedBook, BookError> {
    let mut sink = Sink::new(cache_dir)?;
    let mut stack: Vec<String> = Vec::new();
    let mut para = String::new();
    let mut title_parts: Vec<String> = Vec::new();
    let mut book_title = String::new();
    // Pictures are <binary id="…"> elements (base64) after the body, linked by
    // <image l:href="#id"/>; the cover is linked from <coverpage>.
    let mut binary: Option<(String, String)> = None;

    walk(BufReader::new(r), |ev| {
        match ev {
            Ev::Start(e, _) => {
                let name = e.local_name().as_ref().to_string();
                if matches!(name.as_str(), "p" | "v" | "subtitle" | "text-author") {
                    para.clear();
                }
                if name == "image" {
                    let in_cover = stack.iter().any(|n| n == "coverpage");
                    let in_body = !stack.iter().any(|n| n == "description");
                    if let Some(href) = attr(e, "href").filter(|_| in_cover || in_body) {
                        sink.paragraph(&para)?;
                        para.clear();
                        sink.image(href.trim_start_matches('#'))?;
                    }
                }
                if name == "binary" {
                    binary = attr(e, "id").map(|id| (id, String::new()));
                }
                stack.push(name);
            }
            Ev::Text(t) if stack.last().is_some_and(|n| n == "binary") => {
                if let Some((_, data)) = &mut binary {
                    data.push_str(t);
                }
            }
            Ev::Text(t) => {
                let skip = stack.iter().any(|n| n == "binary");
                let in_title_info = stack.iter().any(|n| n == "title-info");
                if in_title_info && stack.last().is_some_and(|n| n == "book-title") {
                    book_title.push_str(t);
                } else if !skip && !stack.iter().any(|n| n == "description") {
                    para.push_str(t);
                }
            }
            Ev::End(name) => {
                stack.pop();
                let in_title = stack.iter().any(|n| n == "title");
                if name == "binary" {
                    if let Some((id, data)) = binary.take() {
                        if let Some(bytes) = super::image::base64(&data) {
                            sink.save_image(&id, &bytes)?;
                        }
                    }
                }
                match name {
                    "p" | "v" | "subtitle" | "text-author" if in_title => {
                        title_parts.push(std::mem::take(&mut para));
                    }
                    "p" | "v" | "subtitle" | "text-author" => {
                        sink.paragraph(&para)?;
                        para.clear();
                    }
                    "title" if !in_title => {
                        let label = title_parts.join(" — ");
                        title_parts.clear();
                        sink.chapter(&label)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })?;
    let book_title = book_title.trim().to_string();
    sink.title = if book_title.is_empty() {
        fallback_title
    } else {
        Some(book_title)
    };
    sink.finish()
}

fn zip_entry_title<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let entry = zip.by_name(name).ok()?;
    let mut title = String::new();
    let mut in_title = false;
    walk(BufReader::new(entry), |ev| {
        match ev {
            Ev::Start(e, _) => in_title = e.local_name().as_ref() == "title",
            Ev::Text(t) if in_title => title.push_str(t),
            Ev::End(_) => in_title = false,
            _ => {}
        }
        Ok(())
    })
    .ok()?;
    let title = title.trim().to_string();
    (!title.is_empty()).then_some(title)
}

fn open_zip<R: Read + Seek>(r: R) -> Result<zip::ZipArchive<R>, BookError> {
    zip::ZipArchive::new(r).map_err(xml_err)
}

/// Word: paragraphs from `word/document.xml`; Title / Heading 1-2 styles start chapters.
pub fn extract_docx<R: Read + Seek>(
    r: R,
    cache_dir: &Path,
    fallback_title: Option<String>,
) -> Result<ExtractedBook, BookError> {
    let mut zip = open_zip(r)?;
    let mut sink = Sink::new(cache_dir)?;
    sink.title = zip_entry_title(&mut zip, "docProps/core.xml").or(fallback_title);
    let rels = docx_relationships(&mut zip);
    let doc = zip.by_name("word/document.xml").map_err(xml_err)?;
    let mut para = String::new();
    let mut heading = false;
    let mut in_text = false;
    // Pictures in the paragraph, as paths in the ZIP; added after its text.
    let mut pics: Vec<String> = Vec::new();
    // Text inside deleted revisions, field codes, footnote references is not shown.
    let mut hidden = 0usize;

    walk(BufReader::new(doc), |ev| {
        match ev {
            Ev::Start(e, empty) => match e.local_name().as_ref() {
                "p" => {
                    para.clear();
                    heading = false;
                }
                "pStyle" => {
                    let style = attr(e, "val")
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .replace(' ', "");
                    heading = matches!(
                        style.as_str(),
                        "title" | "heading1" | "heading2" | "berschrift1" | "1" | "2"
                    );
                }
                "t" => in_text = true,
                // DrawingML <a:blip r:embed="rId5"/> or VML <v:imagedata r:id="rId5"/>.
                "blip" | "imagedata" if hidden == 0 => {
                    let id = attr(e, "embed").or_else(|| attr(e, "id"));
                    if let Some(target) = id.and_then(|id| rels.get(&id)) {
                        pics.push(target.clone());
                    }
                }
                "tab" if hidden == 0 => para.push(' '),
                "br" | "cr" if hidden == 0 => para.push(' '),
                "del" | "instrText" | "delText" if !empty => hidden += 1,
                _ => {}
            },
            Ev::Text(t) if in_text && hidden == 0 => para.push_str(t),
            Ev::End(name) => match name {
                "t" => in_text = false,
                "del" | "instrText" | "delText" => hidden = hidden.saturating_sub(1),
                "p" => {
                    if heading {
                        sink.chapter(&para)?;
                    } else {
                        sink.paragraph(&para)?;
                    }
                    para.clear();
                    for pic in pics.drain(..) {
                        sink.image(&pic)?;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    save_zip_images(&mut zip, &mut sink)?;
    sink.finish()
}

/// Picture relationships of `word/document.xml`: id → path in the ZIP.
fn docx_relationships<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> std::collections::HashMap<String, String> {
    let mut rels = std::collections::HashMap::new();
    let Ok(entry) = zip.by_name("word/_rels/document.xml.rels") else {
        return rels;
    };
    let _ = walk(BufReader::new(entry), |ev| {
        if let Ev::Start(e, _) = ev {
            if e.local_name().as_ref() == "Relationship" {
                let external = attr(e, "TargetMode").is_some_and(|m| m == "External");
                if let (Some(id), Some(target), false) =
                    (attr(e, "Id"), attr(e, "Target"), external)
                {
                    rels.insert(id, super::image::resolve("word/document.xml", &target));
                }
            }
        }
        Ok(())
    });
    rels
}

/// Copies the pictures that image paragraphs link to (paths in the ZIP) out of it.
fn save_zip_images<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    sink: &mut Sink,
) -> Result<(), BookError> {
    for key in sink.missing_images() {
        let Ok(entry) = zip.by_name(&key) else {
            continue;
        };
        if entry.size() > super::extract::MAX_IMAGE {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .take(super::extract::MAX_IMAGE)
            .read_to_end(&mut bytes)?;
        sink.save_image(&key, &bytes)?;
    }
    Ok(())
}

/// OpenDocument text: `text:h` (outline level 1-2) starts chapters; notes are skipped.
pub fn extract_odt<R: Read + Seek>(
    r: R,
    cache_dir: &Path,
    fallback_title: Option<String>,
) -> Result<ExtractedBook, BookError> {
    let mut zip = open_zip(r)?;
    let mut sink = Sink::new(cache_dir)?;
    sink.title = zip_entry_title(&mut zip, "meta.xml").or(fallback_title);
    let content = zip.by_name("content.xml").map_err(xml_err)?;
    let mut para = String::new();
    let mut depth = 0usize; // nesting of text:p / text:h
    let mut heading_level: Option<u32> = None;
    let mut skip = 0usize;
    let mut in_body = false;
    let mut pics: Vec<String> = Vec::new();

    walk(BufReader::new(content), |ev| {
        match ev {
            Ev::Start(e, empty) => match e.local_name().as_ref() {
                "text" => in_body = true,
                "note" | "annotation" | "tracked-changes" if !empty => skip += 1,
                "p" | "h" if skip == 0 => {
                    if depth == 0 {
                        para.clear();
                        heading_level = (e.local_name().as_ref() == "h").then(|| {
                            attr(e, "outline-level")
                                .and_then(|l| l.parse().ok())
                                .unwrap_or(1)
                        });
                    }
                    depth += 1;
                }
                "s" if skip == 0 => {
                    let n: usize = attr(e, "c").and_then(|c| c.parse().ok()).unwrap_or(1);
                    para.push_str(&" ".repeat(n));
                }
                "tab" | "line-break" if skip == 0 => para.push(' '),
                // <draw:image xlink:href="Pictures/x.png"/> inside a frame.
                "image" if skip == 0 && in_body => {
                    if let Some(href) = attr(e, "href").filter(|h| !h.contains("://")) {
                        let path = super::image::resolve("content.xml", &href);
                        if depth == 0 {
                            sink.image(&path)?;
                        } else {
                            pics.push(path);
                        }
                    }
                }
                _ => {}
            },
            Ev::Text(t) if skip == 0 && depth > 0 && in_body => para.push_str(t),
            Ev::End(name) => match name {
                "note" | "annotation" | "tracked-changes" => skip = skip.saturating_sub(1),
                "p" | "h" if skip == 0 && depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        match heading_level {
                            Some(l) if l <= 2 => sink.chapter(&para)?,
                            _ => sink.paragraph(&para)?,
                        }
                        para.clear();
                        for pic in pics.drain(..) {
                            sink.image(&pic)?;
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    save_zip_images(&mut zip, &mut sink)?;
    sink.finish()
}

#[cfg(test)]
mod tests {
    use super::super::extract::test_dir;
    use super::super::BookSource;
    use super::*;
    use std::io::{Cursor, Write};

    fn texts(book: &mut ExtractedBook) -> Vec<String> {
        (0..book.sections().len())
            .flat_map(|i| book.load(i).unwrap().paragraphs)
            .collect()
    }
    fn labels(book: &ExtractedBook) -> Vec<Option<String>> {
        book.sections().into_iter().map(|s| s.label).collect()
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn fb2_sections_and_windows_1251() {
        let xml = r#"<?xml version="1.0" encoding="windows-1251"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info><book-title>Книга</book-title></title-info></description>
<body><section><title><p>Глава 1</p><p>Начало</p></title><p>Текст &amp; <emphasis>ещё</emphasis>.</p>
<section><title><p>Глава 2</p></title><poem><stanza><v>Строка</v></stanza></poem></section></section></body>
<binary id="x" content-type="image/png">AAAA</binary></FictionBook>"#;
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(xml);
        let mut book = extract_fb2(Cursor::new(bytes.into_owned()), &test_dir(), None).unwrap();
        assert_eq!(book.title().as_deref(), Some("Книга"));
        assert_eq!(
            labels(&book),
            vec![Some("Глава 1 — Начало".into()), Some("Глава 2".into())]
        );
        assert_eq!(
            texts(&mut book),
            vec!["Глава 1 — Начало", "Текст & ещё.", "Глава 2", "Строка"]
        );
    }

    #[test]
    fn docx_headings_and_runs() {
        let doc = r#"<w:document xmlns:w="w"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chương 1</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Xin </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>chào</w:t></w:r><w:r><w:tab/><w:t>bạn</w:t></w:r><w:del><w:r><w:delText>xoá</w:delText></w:r></w:del></w:p>
</w:body></w:document>"#;
        let core = r#"<cp:coreProperties xmlns:cp="c" xmlns:dc="d"><dc:title>Tài liệu</dc:title></cp:coreProperties>"#;
        let data = zip_of(&[("word/document.xml", doc), ("docProps/core.xml", core)]);
        let mut book = extract_docx(Cursor::new(data), &test_dir(), None).unwrap();
        assert_eq!(book.title().as_deref(), Some("Tài liệu"));
        assert_eq!(labels(&book), vec![Some("Chương 1".into())]);
        assert_eq!(texts(&mut book), vec!["Chương 1", "Xin chào bạn"]);
    }

    #[test]
    fn odt_headings_spaces_and_notes() {
        let content = r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><office:text>
<text:h text:outline-level="1">Phần A</text:h>
<text:p>Một<text:s text:c="2"/>hai<text:note><text:note-body><text:p>chú thích</text:p></text:note-body></text:note> ba</text:p>
<text:h text:outline-level="3">Mục nhỏ</text:h>
</office:text></office:body></office:document-content>"#;
        let data = zip_of(&[("content.xml", content)]);
        let mut book =
            extract_odt(Cursor::new(data), &test_dir(), Some("tên file".into())).unwrap();
        assert_eq!(book.title().as_deref(), Some("tên file"));
        assert_eq!(labels(&book), vec![Some("Phần A".into())]);
        assert_eq!(texts(&mut book), vec!["Phần A", "Một hai ba", "Mục nhỏ"]);
    }

    #[test]
    fn fb2_pictures_from_binaries() {
        let xml = r##"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink"><description><title-info><coverpage><image l:href="#cover.png"/></coverpage></title-info></description>
<body><section><p>Một</p><image l:href="#pic.jpg"/><p>Hai</p></section></body>
<binary id="cover.png" content-type="image/png">iVBORw==</binary><binary id="pic.jpg" content-type="image/jpeg">/9g=</binary></FictionBook>"##;
        let mut book = extract_fb2(Cursor::new(xml.as_bytes().to_vec()), &test_dir(), None).unwrap();
        assert_eq!(
            texts(&mut book),
            vec!["\u{FFFC}cover.png", "Một", "\u{FFFC}pic.jpg", "Hai"]
        );
        assert_eq!(book.image("cover.png").unwrap().mime, "image/png");
        assert_eq!(book.image("pic.jpg").unwrap().mime, "image/jpeg");
    }

    #[test]
    fn docx_and_odt_pictures_from_the_zip() {
        let doc = r#"<w:document xmlns:w="w" xmlns:a="a" xmlns:r="r"><w:body>
<w:p><w:r><w:t>Hình dưới</w:t></w:r><w:r><w:drawing><a:blip r:embed="rId7"/></w:drawing></w:r></w:p>
</w:body></w:document>"#;
        let rels = r#"<Relationships xmlns="x"><Relationship Id="rId7" Type="image" Target="media/image1.png"/><Relationship Id="rId8" Target="http://x" TargetMode="External"/></Relationships>"#;
        let data = zip_of(&[
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", rels),
            ("word/media/image1.png", "\u{89}PNG"),
        ]);
        let mut book = extract_docx(Cursor::new(data), &test_dir(), None).unwrap();
        assert_eq!(
            texts(&mut book),
            vec!["Hình dưới", "\u{FFFC}word/media/image1.png"]
        );
        assert!(book.image("word/media/image1.png").is_ok());

        let content = r#"<office:document-content xmlns:office="o" xmlns:text="t" xmlns:draw="d" xmlns:xlink="x"><office:body><office:text>
<text:p>Ảnh<draw:frame><draw:image xlink:href="Pictures/a.jpg"/></draw:frame></text:p>
</office:text></office:body></office:document-content>"#;
        let data = zip_of(&[("content.xml", content), ("Pictures/a.jpg", "jpg")]);
        let mut book = extract_odt(Cursor::new(data), &test_dir(), None).unwrap();
        assert_eq!(texts(&mut book), vec!["Ảnh", "\u{FFFC}Pictures/a.jpg"]);
        assert_eq!(book.image("Pictures/a.jpg").unwrap().bytes, b"jpg");
    }
}
