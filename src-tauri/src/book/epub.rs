use super::image::{self, Image};
use super::text::html_to_paragraphs;
use super::{BookError, BookSource, Section, SectionMeta};
use ::epub::doc::{EpubDoc, NavPoint};
use std::collections::HashMap;
use std::io::{Read, Seek};

/// EPUB read chapter by chapter from the ZIP; opening only reads the manifest and TOC.
pub struct EpubBook<R: Read + Seek> {
    doc: EpubDoc<R>,
    labels: Vec<Option<String>>,
}

impl<R: Read + Seek + Send> EpubBook<R> {
    pub fn open(reader: R) -> Result<Self, BookError> {
        let doc = EpubDoc::from_reader(reader).map_err(|e| BookError::Parse(e.to_string()))?;
        let mut by_chapter = HashMap::new();
        collect_labels(&doc, &doc.toc, &mut by_chapter);
        let labels = (0..doc.spine.len())
            .map(|i| by_chapter.remove(&i))
            .collect();
        Ok(Self { doc, labels })
    }
}

/// Maps TOC entries to spine positions; the first entry pointing into a chapter names it.
fn collect_labels<R: Read + Seek>(
    doc: &EpubDoc<R>,
    points: &[NavPoint],
    out: &mut HashMap<usize, String>,
) {
    for p in points {
        // TOC links may carry a #fragment; the chapter is the file before it.
        let path = p.content.to_string_lossy();
        let file = path.split('#').next().unwrap_or_default();
        if let Some(ch) = doc.resource_uri_to_chapter(&file.into()) {
            let label = p.label.trim();
            if !label.is_empty() {
                out.entry(ch).or_insert_with(|| label.to_string());
            }
        }
        collect_labels(doc, &p.children, out);
    }
}

impl<R: Read + Seek + Send> BookSource for EpubBook<R> {
    fn title(&self) -> Option<String> {
        self.doc.get_title()
    }

    fn sections(&self) -> Vec<SectionMeta> {
        self.labels
            .iter()
            .map(|label| SectionMeta {
                label: label.clone(),
                pages: None,
            })
            .collect()
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
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
        // Picture links are relative to the chapter file; keys are paths in the ZIP.
        let chapter = self
            .doc
            .resources
            .get(&idref)
            .map(|r| r.path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        Ok(Section {
            index,
            paragraphs: html_to_paragraphs(&html, |src| {
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
