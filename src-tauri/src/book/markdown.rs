//! Markdown files as reading text: `#`/`##` headings start chapters, markup is removed.

use super::extract::{ExtractedBook, Sink};
use super::BookError;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// Ends the current paragraph, then adds the pictures it contained.
fn flush(sink: &mut Sink, para: &mut String, imgs: &mut Vec<String>) -> Result<(), BookError> {
    sink.paragraph(para)?;
    para.clear();
    for src in imgs.drain(..) {
        match super::image::data_uri(&src) {
            Some(bytes) => sink.picture(&format!("data:{}", bytes.len()), &bytes)?,
            None => sink.image(&src)?,
        }
    }
    Ok(())
}

pub fn extract_markdown(
    r: impl Read,
    cache_dir: &Path,
    title: Option<String>,
    base: Option<&Path>,
) -> Result<ExtractedBook, BookError> {
    let mut sink = Sink::new(cache_dir)?;
    sink.title = title;
    let mut r = BufReader::new(r);
    let mut raw = Vec::new();
    let mut para = String::new();
    let mut in_code = false;
    let mut first = true;
    let mut imgs = Vec::new();

    loop {
        raw.clear();
        if r.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        let mut line = String::from_utf8_lossy(&raw).into_owned();
        if first {
            line = line.trim_start_matches('\u{feff}').to_string();
            first = false;
        }
        let line = line.trim_end();
        let t = line.trim_start();

        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut sink, &mut para, &mut imgs)?;
            in_code = !in_code;
            continue;
        }
        if in_code {
            sink.paragraph(line)?;
            continue;
        }
        if t.is_empty() || is_rule(t) {
            flush(&mut sink, &mut para, &mut imgs)?;
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut sink, &mut para, &mut imgs)?;
            let text = inline(t[hashes..].trim().trim_end_matches('#').trim(), &mut imgs);
            if hashes <= 2 {
                sink.chapter(&text)?;
            } else {
                sink.paragraph(&text)?;
            }
            continue;
        }
        // A list item or quote starts its own paragraph.
        let (item, body) = list_item(t);
        if item || t.starts_with('>') {
            flush(&mut sink, &mut para, &mut imgs)?;
        }
        let body = body.trim_start_matches('>').trim_start();
        if item {
            para.push_str("• ");
        } else if !para.is_empty() {
            para.push(' ');
        }
        para.push_str(&inline(body, &mut imgs));
    }
    flush(&mut sink, &mut para, &mut imgs)?;
    if let Some(base) = base {
        sink.load_linked_images(base)?;
    }
    sink.finish()
}

fn is_rule(t: &str) -> bool {
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ["-", "*", "_", "="]
            .iter()
            .any(|m| compact.chars().all(|c| c.to_string() == *m))
}

/// `- x`, `* x`, `+ x`, `1. x` → (true, "x").
fn list_item(t: &str) -> (bool, &str) {
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(m) {
            return (true, rest);
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        if let Some(rest) = t[digits..]
            .strip_prefix(". ")
            .or_else(|| t[digits..].strip_prefix(") "))
        {
            return (true, rest);
        }
    }
    (false, t)
}

/// Removes inline markup: links (keeping their text), emphasis and code marks.
/// Pictures (`![alt](src)`, `<img src>`) are taken out and their links added to `imgs`.
fn inline(s: &str, imgs: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(['[', '!', '*', '_', '`', '<']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let image = rest.starts_with("![");
        if image || rest.starts_with('[') {
            let open = if image { 2 } else { 1 };
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close..].find(')') {
                    if image {
                        let target = rest[close + 2..close + end].trim();
                        let src = target.split_whitespace().next().unwrap_or_default();
                        let src = src.trim_start_matches('<').trim_end_matches('>');
                        if !src.is_empty() {
                            imgs.push(src.to_string());
                        }
                    } else {
                        out.push_str(&rest[open..close]);
                    }
                    rest = &rest[close + end + 1..];
                    continue;
                }
            }
            out.push_str(&rest[..open]);
            rest = &rest[open..];
        } else if rest.starts_with("**") || rest.starts_with("__") {
            rest = &rest[2..];
        } else if rest.starts_with('*') || rest.starts_with('`') {
            rest = &rest[1..];
        } else if rest.starts_with('<') {
            // Inline HTML tags such as <br> or <span>.
            if let Some(end) = rest.find('>') {
                if rest[1..].to_ascii_lowercase().starts_with("img") {
                    if let Some(src) = super::image::attr(&rest[1..end], "src") {
                        imgs.push(src);
                    }
                    rest = &rest[end + 1..];
                    continue;
                }
            }
            match rest.find('>') {
                Some(end)
                    if rest[1..end]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || " /=\"'-:".contains(c)) =>
                {
                    rest = &rest[end + 1..];
                }
                _ => {
                    out.push('<');
                    rest = &rest[1..];
                }
            }
        } else {
            // A lone `_` or `!` inside words stays.
            out.push_str(&rest[..1]);
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::super::extract::test_dir;
    use super::super::BookSource;
    use super::*;
    use std::io::Cursor;

    #[test]
    fn markdown_becomes_chapters_and_paragraphs() {
        let md = "Mở đầu **đậm** và `mã`.\n\n# Chương 1\n\nDòng một\nnối tiếp [liên kết](http://x) ![ảnh](a.png)\n\n- mục a\n- mục b\n\n> trích dẫn\n\n---\n\n```\nlet x = 1;\n```\n\n## Chương 2 ##\n### Mục nhỏ\nsnake_case giữ nguyên\n";
        let base = test_dir().join(format!("md-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.png"), b"\x89PNG data").unwrap();
        let mut book = extract_markdown(
            Cursor::new(md.as_bytes().to_vec()),
            &test_dir(),
            None,
            Some(&base),
        )
        .unwrap();
        let labels: Vec<_> = book.sections().into_iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            vec![None, Some("Chương 1".into()), Some("Chương 2".into())]
        );
        assert_eq!(book.load(0).unwrap().paragraphs, vec!["Mở đầu đậm và mã."]);
        assert_eq!(
            book.load(1).unwrap().paragraphs,
            vec![
                "Chương 1",
                "Dòng một nối tiếp liên kết",
                "\u{FFFC}a.png",
                "• mục a",
                "• mục b",
                "trích dẫn",
                "let x = 1;"
            ]
        );
        assert_eq!(
            book.load(2).unwrap().paragraphs,
            vec!["Chương 2", "Mục nhỏ", "snake_case giữ nguyên"]
        );
        assert_eq!(book.image("a.png").unwrap().mime, "image/png");
        assert!(book.image("b.png").is_err());
    }
}
