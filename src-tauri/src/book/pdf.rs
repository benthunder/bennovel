use super::image::{self, Image};
use super::text::pdf_pages_to_paragraphs;
use super::{BookError, BookSource, Section, SectionMeta};
use pdfium_render::prelude::*;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Pages grouped into one reader section when the PDF has no usable bookmarks.
pub const PAGES_PER_SECTION: u32 = 10;
/// A chapter from the bookmarks longer than this is cut into parts of
/// [`PAGES_PER_SECTION`] pages; only the first part carries its title.
pub const MAX_CHAPTER_PAGES: u32 = 30;

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
    /// First and last page (1-based) of each section, with its bookmark title.
    sections: Vec<(u32, u32, Option<String>)>,
}

impl PdfBook {
    pub fn open(file: std::fs::File, pdfium_dirs: &[PathBuf]) -> Result<Self, BookError> {
        let doc = pdfium(pdfium_dirs)?
            .load_pdf_from_reader(file, None)
            .map_err(|e| BookError::Parse(e.to_string()))?;
        let pages = doc.pages().len() as u32;
        let sections = page_sections(pages, &bookmarks(&doc));
        Ok(Self {
            doc,
            pages,
            sections,
        })
    }
}

/// Bookmark levels used as chapters: the top level and the one below (volumes and
/// their chapters); deeper ones are sections inside a chapter.
const BOOKMARK_LEVELS: usize = 2;

/// Bookmarks of the first [`BOOKMARK_LEVELS`] levels, in reading order, as
/// (0-based page, title), when they point to a page of this document.
fn bookmarks(doc: &PdfDocument) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    // Bounds the walk, also when a broken file links bookmarks in a loop.
    let mut budget = 10_000usize;
    walk_bookmarks(doc.bookmarks().root(), 1, &mut budget, &mut out);
    out
}

fn walk_bookmarks(
    first: Option<PdfBookmark>,
    level: usize,
    budget: &mut usize,
    out: &mut Vec<(u32, String)>,
) {
    let mut node = first;
    while let Some(b) = node {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        if let Some(mark) = bookmark_page(&b) {
            out.push(mark);
        }
        if level < BOOKMARK_LEVELS {
            walk_bookmarks(b.first_child(), level + 1, budget, out);
        }
        node = b.next_sibling();
    }
}

/// A bookmark's (0-based page, title), when it has a title and goes to a page.
fn bookmark_page(b: &PdfBookmark) -> Option<(u32, String)> {
    let title = b.title()?.split_whitespace().collect::<Vec<_>>().join(" ");
    let page = match b.destination() {
        Some(dest) => dest.page_index().ok()?,
        None => {
            let action = b.action()?;
            let local = action.as_local_destination_action()?;
            let dest = local.destination().ok()?;
            dest.page_index().ok()?
        }
    } as u32;
    (!title.is_empty()).then_some((page, title))
}

/// Sections of a `pages`-page PDF: one per bookmarked chapter when at least two
/// bookmarks point to different pages in order, else [`PAGES_PER_SECTION`] pages
/// each. Bookmarks pointing back before an earlier one are skipped; on a page with
/// several, the last (deepest) names the chapter. Pages before the first bookmark
/// and long chapters are cut like a PDF without bookmarks.
fn page_sections(pages: u32, marks: &[(u32, String)]) -> Vec<(u32, u32, Option<String>)> {
    let mut starts: Vec<(u32, &str)> = Vec::new();
    for (page, title) in marks {
        let page = page + 1;
        if page > pages {
            continue;
        }
        match starts.last_mut() {
            Some(last) if last.0 == page => last.1 = title,
            Some(last) if last.0 > page => {}
            _ => starts.push((page, title)),
        }
    }
    let mut out = Vec::new();
    let mut chunks = |from: u32, to: u32, label: Option<&str>, max: u32| {
        let mut first = from;
        let mut label = label.map(String::from);
        while first <= to {
            let last = if to - first < max {
                to
            } else {
                first + PAGES_PER_SECTION - 1
            };
            out.push((first, last, label.take()));
            first = last + 1;
        }
    };
    if starts.len() < 2 {
        chunks(1, pages, None, PAGES_PER_SECTION);
        return out;
    }
    if starts[0].0 > 1 {
        chunks(1, starts[0].0 - 1, None, PAGES_PER_SECTION);
    }
    for (n, &(first, title)) in starts.iter().enumerate() {
        let last = starts.get(n + 1).map_or(pages, |next| next.0 - 1);
        chunks(first, last, Some(title), MAX_CHAPTER_PAGES);
    }
    out
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
        self.sections
            .iter()
            .map(|(first, last, label)| SectionMeta {
                label: label.clone(),
                pages: Some([*first, *last]),
                number: None,
            })
            .collect()
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let &(first, last, _) = self
            .sections
            .get(index)
            .ok_or(BookError::NoSection(index))?;
        let mut pages = Vec::new();
        for i in first - 1..last {
            let page = self
                .doc
                .pages()
                .get(i as PdfPageIndex)
                .map_err(|e| BookError::Parse(e.to_string()))?;
            let text = page.text().map_err(|e| BookError::Parse(e.to_string()))?;
            pages.push(text.all());
        }
        let paragraphs = pdf_pages_to_paragraphs(&pages);
        Ok(Section { index, paragraphs })
    }

    fn page_images(&self) -> bool {
        true
    }

    fn render_page(&mut self, page: u32, width: u32) -> Result<Image, BookError> {
        if page == 0 || page > self.pages {
            return Err(BookError::NoImage(format!("page {page}")));
        }
        let page = self
            .doc
            .pages()
            .get((page - 1) as PdfPageIndex)
            .map_err(|e| BookError::Parse(e.to_string()))?;
        let config = PdfRenderConfig::new()
            .set_target_width(width.clamp(200, 3000) as Pixels)
            .render_form_data(true)
            .render_annotations(true);
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| BookError::Parse(e.to_string()))?;
        image::encode_jpeg(
            bitmap.width() as u32,
            bitmap.height() as u32,
            &bitmap.as_rgba_bytes(),
        )
    }

    fn page_text(&mut self, page: u32) -> Result<Option<String>, BookError> {
        if page == 0 || page > self.pages {
            return Err(BookError::NoImage(format!("page {page}")));
        }
        let page = self
            .doc
            .pages()
            .get((page - 1) as PdfPageIndex)
            .map_err(|e| BookError::Parse(e.to_string()))?;
        let text = page.text().map_err(|e| BookError::Parse(e.to_string()))?;
        Ok(Some(text.all()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A PDF whose page `n` shows "Page n." in Helvetica.
    fn sample_pdf(pages: u32) -> Vec<u8> {
        sample_pdf_with_outline(pages, &[])
    }

    /// Same, with bookmarks (title, 0-based page, index of the parent bookmark).
    fn sample_pdf_with_outline(pages: u32, outline: &[(&str, u32, Option<usize>)]) -> Vec<u8> {
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
        if !outline.is_empty() {
            // Object numbers: the outline root, then one per bookmark.
            let root = objs.len() + 1;
            let num = |i: usize| root + 1 + i;
            let children = |parent: Option<usize>| -> Vec<usize> {
                (0..outline.len())
                    .filter(|&i| outline[i].2 == parent)
                    .collect()
            };
            let links = |list: &[usize]| {
                format!(
                    "/First {} 0 R /Last {} 0 R /Count {}",
                    num(list[0]),
                    num(*list.last().unwrap()),
                    list.len()
                )
            };
            objs.push(format!("<< /Type /Outlines {} >>", links(&children(None))));
            for (i, &(title, page, parent)) in outline.iter().enumerate() {
                let siblings = children(parent);
                let at = siblings.iter().position(|&s| s == i).unwrap();
                let mut o = format!(
                    "<< /Title ({title}) /Parent {} 0 R /Dest [{} /Fit]",
                    parent.map_or(root, num),
                    kids[page as usize]
                );
                if at > 0 {
                    o += &format!(" /Prev {} 0 R", num(siblings[at - 1]));
                }
                if let Some(&next) = siblings.get(at + 1) {
                    o += &format!(" /Next {} 0 R", num(next));
                }
                let own = children(Some(i));
                if !own.is_empty() {
                    o += &format!(" {}", links(&own));
                }
                objs.push(o + " >>");
            }
            objs[0] = format!("<< /Type /Catalog /Pages 2 0 R /Outlines {root} 0 R >>");
        }

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

    fn marks(list: &[(u32, &str)]) -> Vec<(u32, String)> {
        list.iter().map(|&(p, t)| (p, t.to_string())).collect()
    }

    #[test]
    fn bookmarks_make_chapters() {
        let got = page_sections(
            60,
            &marks(&[
                (2, "Quyển 1"),
                (2, "Chương 1"),
                (5, "Chương 2"),
                (3, "Sai thứ tự"),
                (9, "Chương 3"),
                (99, "Ngoài sách"),
            ]),
        );
        assert_eq!(
            got,
            vec![
                (1, 2, None),
                (3, 5, Some("Chương 1".to_string())),
                (6, 9, Some("Chương 2".to_string())),
                // 51 pages: cut into parts, only the first named.
                (10, 19, Some("Chương 3".to_string())),
                (20, 29, None),
                (30, 39, None),
                (40, 60, None),
            ]
        );
    }

    #[test]
    fn without_bookmarks_pages_are_grouped_by_ten() {
        let ten = vec![(1, 10, None), (11, 20, None), (21, 25, None)];
        assert_eq!(page_sections(25, &[]), ten);
        // One bookmark is not a table of contents.
        assert_eq!(page_sections(25, &marks(&[(0, "Bìa")])), ten);
        assert_eq!(page_sections(0, &[]), vec![]);
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
        assert_eq!(book.page_text(3).unwrap().unwrap().trim(), "Page 3.");
        let page = book.render_page(25, 600).unwrap();
        assert_eq!(image::mime_of(&page.bytes), "image/jpeg");
        assert!(book.render_page(26, 600).is_err());
        std::fs::remove_file(path).unwrap();
    }

    /// Needs the Pdfium library: set PDFIUM_DIR to the folder holding it.
    #[test]
    fn reads_bookmarks_as_chapters() {
        let Some(dir) = std::env::var_os("PDFIUM_DIR") else {
            eprintln!("skipped: PDFIUM_DIR not set");
            return;
        };
        let path =
            std::env::temp_dir().join(format!("bennovel-test-marks-{}.pdf", std::process::id()));
        let outline = [
            ("Part One", 2, None),
            ("Chapter 1", 2, Some(0)),
            ("Inside", 3, Some(1)),
            ("Chapter 2", 5, Some(0)),
        ];
        std::fs::write(&path, sample_pdf_with_outline(8, &outline)).unwrap();
        let mut book = PdfBook::open(std::fs::File::open(&path).unwrap(), &[dir.into()]).unwrap();
        let got: Vec<_> = book
            .sections()
            .into_iter()
            .map(|s| (s.pages.unwrap(), s.label))
            .collect();
        assert_eq!(
            got,
            vec![
                ([1, 2], None),
                ([3, 5], Some("Chapter 1".to_string())),
                ([6, 8], Some("Chapter 2".to_string())),
            ]
        );
        assert_eq!(
            book.load(1).unwrap().paragraphs,
            vec!["Page 3.", "Page 4.", "Page 5."]
        );
        std::fs::remove_file(path).unwrap();
    }
}
