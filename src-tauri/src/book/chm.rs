//! CHM (compiled HTML help): pages listed in the `.hhc` table of contents become
//! sections; each page is decompressed only when it is read.

use super::extract::TempCopy;
use super::html::html_encoding;
use super::text::html_to_paragraphs;
use super::{BookError, BookSource, Section, SectionMeta};
use libchm::{ChmFile, EntrySel};
use std::path::Path;

pub struct ChmBook {
    chm: ChmFile,
    pages: Vec<(String, Option<String>)>,
    title: Option<String>,
    _copy: Option<TempCopy>,
}

fn err(e: impl std::fmt::Display) -> BookError {
    BookError::Parse(e.to_string())
}

fn decode(bytes: &[u8]) -> String {
    let (enc, bom) = html_encoding(bytes);
    enc.decode_without_bom_handling(&bytes[bom..])
        .0
        .into_owned()
}

impl ChmBook {
    pub fn open(
        path: &Path,
        copy: Option<TempCopy>,
        title: Option<String>,
    ) -> Result<Self, BookError> {
        let mut chm = ChmFile::open(path).map_err(err)?;
        let files = chm
            .entries(EntrySel::NORMAL | EntrySel::FILES)
            .map_err(err)?;
        let mut pages = Vec::new();
        if let Some(hhc) = files
            .iter()
            .find(|e| e.path.to_ascii_lowercase().ends_with(".hhc"))
        {
            let toc = decode(&chm.read(hhc).map_err(err)?);
            pages = toc_pages(&toc);
        }
        if pages.is_empty() {
            let mut html: Vec<String> = files
                .iter()
                .map(|e| e.path.clone())
                .filter(|p| {
                    let l = p.to_ascii_lowercase();
                    l.ends_with(".htm") || l.ends_with(".html")
                })
                .collect();
            html.sort();
            pages = html.into_iter().map(|p| (p, None)).collect();
        }
        Ok(Self {
            chm,
            pages,
            title,
            _copy: copy,
        })
    }
}

/// Archive path of a picture linked from page `page` (`/dir/page.htm`); links of the
/// form `ms-its:book.chm::/img/a.gif` name the path after `::`.
fn chm_link(page: &str, src: &str) -> String {
    let src = src.rsplit_once("::").map_or(src, |(_, p)| p);
    format!(
        "/{}",
        super::image::resolve(page.trim_start_matches('/'), src)
    )
}

/// (path, title) of each `<param name="Local">` in the TOC, in order, without repeats.
fn toc_pages(toc: &str) -> Vec<(String, Option<String>)> {
    let mut pages: Vec<(String, Option<String>)> = Vec::new();
    let mut name: Option<String> = None;
    let lower = toc.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<param") {
        let start = at + i;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &toc[start..end];
        let tag_l = &lower[start..end];
        at = end;
        let value = attr_value(tag, tag_l, "value");
        if tag_l.contains("\"name\"") || tag_l.contains("=name") {
            name = value;
        } else if tag_l.contains("\"local\"") || tag_l.contains("=local") {
            if let Some(local) = value {
                let file = local
                    .split('#')
                    .next()
                    .unwrap_or_default()
                    .replace('\\', "/");
                let file = if file.starts_with('/') {
                    file
                } else {
                    format!("/{file}")
                };
                if !pages.iter().any(|(p, _)| p.eq_ignore_ascii_case(&file)) {
                    pages.push((file, name.take()));
                }
            }
        }
    }
    pages
}

fn attr_value(tag: &str, tag_l: &str, key: &str) -> Option<String> {
    let i = tag_l.find(&format!("{key}="))? + key.len() + 1;
    let rest = &tag[i..];
    let (q, rest) = match rest.chars().next()? {
        c @ ('"' | '\'') => (Some(c), &rest[1..]),
        _ => (None, rest),
    };
    let end = match q {
        Some(q) => rest.find(q)?,
        None => rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len()),
    };
    Some(rest[..end].to_string())
}

impl BookSource for ChmBook {
    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn sections(&self) -> Vec<SectionMeta> {
        self.pages
            .iter()
            .map(|(_, label)| SectionMeta {
                label: label.clone(),
                pages: None,
            })
            .collect()
    }

    fn image(&mut self, key: &str) -> Result<super::image::Image, BookError> {
        let bytes = self
            .chm
            .read_path(key)
            .map_err(|_| BookError::NoImage(key.to_string()))?;
        Ok(super::image::Image::sniff(bytes))
    }

    fn load(&mut self, index: usize) -> Result<Section, BookError> {
        let (path, _) = self.pages.get(index).ok_or(BookError::NoSection(index))?;
        let bytes = self.chm.read_path(path).map_err(err)?;
        Ok(Section {
            index,
            paragraphs: html_to_paragraphs(&decode(&bytes), |src| {
                (!src.contains("://") || src.starts_with("ms-its:") || src.starts_with("mk:"))
                    .then(|| chm_link(path, src))
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_toc_entries_in_order() {
        let hhc = r#"<UL><LI><OBJECT type="text/sitemap"><param name="Name" value="Mở đầu"><param name="Local" value="intro.htm"></OBJECT>
<UL><LI><OBJECT type="text/sitemap"><param name="Name" value="Chương 1"><param name="Local" value="ch\1.htm#top"></OBJECT>
<LI><OBJECT type="text/sitemap"><param name="Name" value="Lặp"><param name="Local" value="intro.htm#x"></OBJECT></UL></UL>"#;
        assert_eq!(
            toc_pages(hhc),
            vec![
                ("/intro.htm".into(), Some("Mở đầu".into())),
                ("/ch/1.htm".into(), Some("Chương 1".into()))
            ]
        );
    }
}
