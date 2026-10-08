//! DjVu: text comes from each page's hidden text layer (scans without OCR have none).
//! The file is memory-mapped, so the OS only loads the pages that are read.

use super::extract::TempCopy;
use super::image::{self, Image};
use super::text::pdf_text_to_paragraphs;
use super::{BookError, BookSource, Section, SectionMeta};
use djvu_rs::djvu_document::MmapDocument;
use std::path::Path;

/// Pages grouped into one reader section, as for PDF.
const PAGES_PER_SECTION: usize = 10;

pub struct DjvuBook {
    doc: MmapDocument,
    pages: usize,
    title: Option<String>,
    _copy: Option<TempCopy>,
}

fn err(e: impl std::fmt::Display) -> BookError {
    BookError::Parse(e.to_string())
}

impl DjvuBook {
    pub fn open(
        path: &Path,
        copy: Option<TempCopy>,
        title: Option<String>,
    ) -> Result<Self, BookError> {
        let doc = MmapDocument::open(path).map_err(err)?;
        let pages = doc.page_count();
        Ok(Self {
            doc,
            pages,
            title,
            _copy: copy,
        })
    }
}

impl BookSource for DjvuBook {
    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn sections(&self) -> Vec<SectionMeta> {
        (0..self.pages.div_ceil(PAGES_PER_SECTION))
            .map(|s| {
                let first = s * PAGES_PER_SECTION + 1;
                let last = (first + PAGES_PER_SECTION - 1).min(self.pages);
                SectionMeta {
                    label: None,
                    pages: Some([first as u32, last as u32]),
                }
            })
            .collect()
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let first = index * PAGES_PER_SECTION;
        if first >= self.pages {
            return Err(BookError::NoSection(index));
        }
        let mut paragraphs = Vec::new();
        for p in first..(first + PAGES_PER_SECTION).min(self.pages) {
            let page = self.doc.page(p).map_err(err)?;
            if let Some(layer) = page.text_layer().map_err(err)? {
                paragraphs.extend(pdf_text_to_paragraphs(&layer.text));
            }
        }
        Ok(Section { index, paragraphs })
    }

    fn page_images(&self) -> bool {
        true
    }

    fn render_page(&mut self, page: u32, width: u32) -> Result<Image, BookError> {
        if page == 0 || page as usize > self.pages {
            return Err(BookError::NoImage(format!("page {page}")));
        }
        let page = self.doc.page(page as usize - 1).map_err(err)?;
        let opts = djvu_rs::djvu_render::RenderOptions::fit_to_width(page, width.clamp(200, 3000));
        let pix = djvu_rs::djvu_render::render_pixmap(page, &opts).map_err(err)?;
        image::encode_jpeg(pix.width, pix.height, &pix.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12 blank pages whose text layer says "Trang n dòng một." / "Kết thúc trang n.".
    #[test]
    fn reads_text_layer_in_page_groups() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/book/fixtures/sample.djvu");
        let mut book = DjvuBook::open(&path, None, None).unwrap();
        let pages: Vec<_> = book
            .sections()
            .into_iter()
            .map(|s| s.pages.unwrap())
            .collect();
        assert_eq!(pages, vec![[1, 10], [11, 12]]);
        assert_eq!(
            book.load(1).unwrap().paragraphs,
            vec![
                "Trang 11 dòng một.",
                "Kết thúc trang 11.",
                "Trang 12 dòng một.",
                "Kết thúc trang 12."
            ]
        );
        let page = book.render_page(12, 400).unwrap();
        assert_eq!(image::mime_of(&page.bytes), "image/jpeg");
        assert!(book.render_page(13, 400).is_err());
    }
}
