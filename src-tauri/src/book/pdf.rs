use super::text::pdf_text_to_paragraphs;
use super::{BookError, BookSource, Section, SectionMeta};
use pdfium_render::prelude::*;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Pages grouped into one reader section.
pub const PAGES_PER_SECTION: u32 = 10;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

/// Loads the Pdfium library once: from the given folders (next to the app, its
/// resources), then from the system.
fn pdfium(dirs: &[PathBuf]) -> Result<&'static Pdfium, BookError> {
    PDFIUM
        .get_or_init(|| {
            let bindings = dirs
                .iter()
                .find_map(|d| {
                    Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(d)).ok()
                })
                .map(Ok)
                .unwrap_or_else(Pdfium::bind_to_system_library)
                .map_err(|e| e.to_string())?;
            Ok(Pdfium::new(bindings))
        })
        .as_ref()
        .map_err(|e| BookError::PdfUnavailable(e.clone()))
}

/// PDF read through Pdfium's file access callbacks, so pages are parsed from disk
/// only when their section is requested.
pub struct PdfBook {
    doc: PdfDocument<'static>,
    pages: u32,
}

impl PdfBook {
    pub fn open(file: std::fs::File, pdfium_dirs: &[PathBuf]) -> Result<Self, BookError> {
        let doc = pdfium(pdfium_dirs)?
            .load_pdf_from_reader(file, None)
            .map_err(|e| BookError::Parse(e.to_string()))?;
        let pages = doc.pages().len() as u32;
        Ok(Self { doc, pages })
    }
}

impl BookSource for PdfBook {
    fn title(&self) -> Option<String> {
        self.doc
            .metadata()
            .get(PdfDocumentMetadataTagType::Title)
            .map(|t| t.value().trim().to_string())
            .filter(|t| !t.is_empty())
    }

    fn sections(&self) -> Vec<SectionMeta> {
        (0..self.pages.div_ceil(PAGES_PER_SECTION))
            .map(|s| {
                let first = s * PAGES_PER_SECTION + 1;
                let last = (first + PAGES_PER_SECTION - 1).min(self.pages);
                SectionMeta {
                    label: None,
                    pages: Some([first, last]),
                }
            })
            .collect()
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let first = index as u32 * PAGES_PER_SECTION;
        if first >= self.pages {
            return Err(BookError::NoSection(index));
        }
        let last = (first + PAGES_PER_SECTION).min(self.pages);
        let mut paragraphs = Vec::new();
        for i in first..last {
            let page = self
                .doc
                .pages()
                .get(i as PdfPageIndex)
                .map_err(|e| BookError::Parse(e.to_string()))?;
            let text = page.text().map_err(|e| BookError::Parse(e.to_string()))?;
            paragraphs.extend(pdf_text_to_paragraphs(&text.all()));
        }
        Ok(Section { index, paragraphs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A PDF whose page `n` shows "Page n." in Helvetica.
    fn sample_pdf(pages: u32) -> Vec<u8> {
        let mut objs: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            String::new(), // page tree, filled in below
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ];
        let mut kids = Vec::new();
        for n in 1..=pages {
            let stream = format!("BT /F1 18 Tf 72 720 Td (Page {n}.) Tj ET");
            objs.push(format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            ));
            let content = objs.len();
            objs.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {content} 0 R /Resources << /Font << /F1 3 0 R >> >> >>"
            ));
            kids.push(format!("{} 0 R", objs.len()));
        }
        objs[1] = format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        );

        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            write!(out, "{} 0 obj\n{o}\nendobj\n", i + 1).unwrap();
        }
        let xref = out.len();
        write!(out, "xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).unwrap();
        for off in offsets {
            writeln!(out, "{off:010} 00000 n ").unwrap();
        }
        write!(
            out,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .unwrap();
        out
    }

    /// Needs the Pdfium library: set PDFIUM_DIR to the folder holding it.
    #[test]
    fn reads_pages_in_sections() {
        let Some(dir) = std::env::var_os("PDFIUM_DIR") else {
            eprintln!("skipped: PDFIUM_DIR not set");
            return;
        };
        let path = std::env::temp_dir().join(format!("bennovel-test-{}.pdf", std::process::id()));
        std::fs::write(&path, sample_pdf(25)).unwrap();
        let mut book = PdfBook::open(std::fs::File::open(&path).unwrap(), &[dir.into()]).unwrap();

        let pages: Vec<_> = book
            .sections()
            .into_iter()
            .map(|s| s.pages.unwrap())
            .collect();
        assert_eq!(pages, vec![[1, 10], [11, 20], [21, 25]]);
        let last = book.load(2).unwrap().paragraphs;
        assert_eq!(
            last,
            (21..=25).map(|n| format!("Page {n}.")).collect::<Vec<_>>()
        );
        assert!(matches!(book.load(3), Err(BookError::NoSection(3))));
        std::fs::remove_file(path).unwrap();
    }
}
