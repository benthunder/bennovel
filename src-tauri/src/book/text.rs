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

/// Paragraph texts of an (X)HTML document, ignoring markup, scripts and styles.
pub fn html_to_paragraphs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut skip_until: Option<String> = None;
    let mut rest = html;

    while let Some(lt) = rest.find('<') {
        if skip_until.is_none() {
            push_text(&mut cur, &rest[..lt]);
        }
        rest = &rest[lt..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |e| &after[e + 3..]);
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];

        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        // Namespaced tags such as <svg:svg> count by their local name.
        let name = name.rsplit(':').next().unwrap_or("").to_string();

        if let Some(skipped) = &skip_until {
            if closing && *skipped == name {
                skip_until = None;
            }
            continue;
        }
        if !closing && !tag.ends_with('/') && SKIPPED_TAGS.contains(&name.as_str()) {
            skip_until = Some(name);
            continue;
        }
        if BLOCK_TAGS.contains(&name.as_str()) {
            flush(&mut out, &mut cur);
        }
    }
    if skip_until.is_none() {
        push_text(&mut cur, rest);
    }
    flush(&mut out, &mut cur);
    out
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

fn push_text(cur: &mut String, raw: &str) {
    let decoded = decode_entities(raw);
    for word in decoded.split_whitespace() {
        if !cur.is_empty() && !cur.ends_with(' ') {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    // Keep a separating space when the text ends in whitespace before an inline tag.
    if decoded.ends_with(char::is_whitespace) && !cur.is_empty() && !cur.ends_with(' ') {
        cur.push(' ');
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
