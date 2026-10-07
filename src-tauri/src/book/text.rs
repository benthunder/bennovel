//! Turns chapter XHTML into plain paragraphs for the reader.

const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "br",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "blockquote",
    "tr",
    "section",
    "article",
    "hr",
    "pre",
    "dd",
    "dt",
    "figcaption",
    "body",
];
const SKIPPED_TAGS: &[&str] = &["head", "script", "style", "svg"];

/// A paragraph of text found in HTML; headings are flagged so they can start chapters.
#[derive(Debug, PartialEq)]
pub enum Block {
    Text(String),
    /// Text of an `<h1>`–`<h3>`.
    Heading(String),
}

/// Paragraph texts of an (X)HTML document, ignoring markup, scripts and styles.
pub fn html_to_paragraphs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut parser = HtmlText::default();
    let mut push = |b: Block| match b {
        Block::Text(t) | Block::Heading(t) => out.push(t),
    };
    parser.feed(html, &mut push);
    parser.finish(&mut push);
    out
}

/// Streaming HTML-to-text: feed it the document piece by piece (any split, even
/// inside a tag) and it emits paragraphs as soon as they end.
#[derive(Default)]
pub struct HtmlText {
    pending: String,
    cur: String,
    skip_until: Option<String>,
    in_heading: bool,
}

impl HtmlText {
    pub fn feed(&mut self, chunk: &str, out: &mut impl FnMut(Block)) {
        self.pending.push_str(chunk);
        let mut consumed = 0;
        loop {
            let rest = &self.pending[consumed..];
            let Some(lt) = rest.find('<') else {
                // Hold back a possibly unfinished entity such as "&am".
                let keep = rest
                    .rfind('&')
                    .filter(|&a| !rest[a..].contains(';') && rest.len() - a < 12);
                let text_end = keep.unwrap_or(rest.len());
                if self.skip_until.is_none() {
                    push_text(&mut self.cur, &rest[..text_end]);
                }
                consumed += text_end;
                break;
            };
            if self.skip_until.is_none() {
                push_text(&mut self.cur, &rest[..lt]);
            }
            let tag_start = &rest[lt..];
            let tag_end = if tag_start.starts_with("<!--") {
                tag_start.find("-->").map(|e| e + 3)
            } else {
                tag_start.find('>').map(|e| e + 1)
            };
            let Some(tag_end) = tag_end else {
                // The tag continues in the next chunk.
                consumed += lt;
                break;
            };
            let tag = tag_start[..tag_end].to_string();
            consumed += lt + tag_end;
            if !tag.starts_with("<!--") {
                self.tag(&tag[1..tag.len() - 1], out);
            }
        }
        self.pending.drain(..consumed);
    }

    pub fn finish(&mut self, out: &mut impl FnMut(Block)) {
        let rest = std::mem::take(&mut self.pending);
        if self.skip_until.is_none() && !rest.starts_with('<') {
            push_text(&mut self.cur, &rest);
        }
        self.flush(out);
    }

    fn tag(&mut self, tag: &str, out: &mut impl FnMut(Block)) {
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        // Namespaced tags such as <svg:svg> count by their local name.
        let name = name.rsplit(':').next().unwrap_or("").to_string();

        if let Some(skipped) = &self.skip_until {
            if closing && *skipped == name {
                self.skip_until = None;
            }
            return;
        }
        if !closing && !tag.ends_with('/') && SKIPPED_TAGS.contains(&name.as_str()) {
            self.skip_until = Some(name);
            return;
        }
        if BLOCK_TAGS.contains(&name.as_str()) || name == "pagebreak" {
            self.flush(out);
            self.in_heading = !closing && matches!(name.as_str(), "h1" | "h2" | "h3");
        }
    }

    fn flush(&mut self, out: &mut impl FnMut(Block)) {
        let p = self.cur.trim();
        if !p.is_empty() {
            let p = p.to_string();
            out(if self.in_heading {
                Block::Heading(p)
            } else {
                Block::Text(p)
            });
        }
        self.cur.clear();
        self.in_heading = false;
    }
}

/// Splits page text from a PDF into paragraphs: a line ending a sentence ends the
/// paragraph, other line breaks are just wrapping.
pub fn pdf_text_to_paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            flush(&mut out, &mut cur);
            continue;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(line);
        if line.ends_with(['.', '!', '?', '。', '！', '？', '”', '"', '」', '…', ':']) {
            flush(&mut out, &mut cur);
        }
    }
    flush(&mut out, &mut cur);
    out
}

/// Appends text with runs of whitespace collapsed to one space. Words split across
/// two calls stay joined.
fn push_text(cur: &mut String, raw: &str) {
    for c in decode_entities(raw).chars() {
        if c.is_whitespace() {
            if !cur.is_empty() && !cur.ends_with(' ') {
                cur.push(' ');
            }
        } else {
            cur.push(c);
        }
    }
}

fn flush(out: &mut Vec<String>, cur: &mut String) {
    let p = cur.trim();
    if !p.is_empty() {
        out.push(p.to_string());
    }
    cur.clear();
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let ent = &rest[1..semi];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "hellip" => Some('…'),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "lsquo" => Some('‘'),
                "rsquo" => Some('’'),
                "ldquo" => Some('“'),
                "rdquo" => Some('”'),
                _ => ent
                    .strip_prefix("#x")
                    .or_else(|| ent.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, semi))
        });
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_paragraphs_and_skips_head() {
        let html = r#"<?xml version="1.0"?><html><head><title>T</title><style>p{}</style></head>
            <body><h1>Chương 1</h1><p>Xin <b>chào</b>
            thế giới.</p><!-- note --><p>A&amp;B &#x4E2D; &#20013;&nbsp;x</p><br/>tail</body></html>"#;
        assert_eq!(
            html_to_paragraphs(html),
            vec!["Chương 1", "Xin chào thế giới.", "A&B 中 中 x", "tail"]
        );
    }

    #[test]
    fn streaming_matches_whole_document_at_any_split() {
        let html = "<html><head><style>x{}</style></head><body><h2>Tiêu đề</h2><p>A &amp; B<!-- c --> c</p><p>dài</p></body></html>";
        let whole = html_to_paragraphs(html);
        for size in 1..html.len() {
            let mut parser = HtmlText::default();
            let mut got = Vec::new();
            let mut push = |b: Block| got.push(b);
            let mut rest = html;
            while !rest.is_empty() {
                let mut cut = size.min(rest.len());
                while !rest.is_char_boundary(cut) {
                    cut += 1;
                }
                parser.feed(&rest[..cut], &mut push);
                rest = &rest[cut..];
            }
            parser.finish(&mut push);
            assert_eq!(got[0], Block::Heading("Tiêu đề".into()), "split {size}");
            let texts: Vec<String> = got
                .into_iter()
                .map(|b| match b {
                    Block::Text(t) | Block::Heading(t) => t,
                })
                .collect();
            assert_eq!(texts, whole, "split {size}");
        }
    }

    #[test]
    fn unknown_entities_stay_as_text() {
        assert_eq!(
            html_to_paragraphs("<p>a &foo; & b</p>"),
            vec!["a &foo; & b"]
        );
    }

    #[test]
    fn joins_wrapped_pdf_lines() {
        let text = "He walked into\nthe room.\nShe said:\n“Hi”\n\nNext part";
        assert_eq!(
            pdf_text_to_paragraphs(text),
            vec!["He walked into the room.", "She said:", "“Hi”", "Next part"]
        );
    }
}
