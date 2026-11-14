//! A chapter's text as stored in `chapter_translations.content`: a JSON array of blocks,
//! one per paragraph, each `type` + `content`:
//!
//! ```json
//! [
//!   {"type":"text","content":["Lâm Phong mở mắt.","\"Đây là đâu?\" hắn lẩm bẩm."]},
//!   {"type":"img","content":"img_0003"}
//! ]
//! ```
//!
//! A text block holds the paragraph's sentences (plain text, no HTML); an img block the
//! key of the picture in `chapter_images`. The rest of the app works with paragraphs
//! (`encode` / `paragraphs` convert), a picture paragraph being `IMAGE_MARK + key`.
//!
//! Sentences are cut offline by punctuation: 。！？ end a sentence straight away
//! (Chinese, Japanese), `. ! ? …` when a space and something that is not a lowercase
//! letter follow (Vietnamese, English…). Nothing inside quotes is cut, and a quote
//! ending a sentence stays with it ("…?" hắn nói. is one sentence).

use crate::book::image;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "content", rename_all = "lowercase")]
pub enum Block {
    /// The sentences of a paragraph.
    Text(Vec<String>),
    /// The key of a picture in `chapter_images`.
    Img(String),
}

/// The blocks of paragraphs (a picture paragraph becomes an img block).
pub fn of_paragraphs<S: AsRef<str>>(paragraphs: &[S]) -> Vec<Block> {
    paragraphs
        .iter()
        .filter_map(|p| {
            let p = p.as_ref();
            if let Some(key) = image::key_of(p) {
                return Some(Block::Img(key.to_string()));
            }
            let sentences = sentences(p);
            (!sentences.is_empty()).then_some(Block::Text(sentences))
        })
        .collect()
}

/// What `content` stores for these paragraphs.
pub fn encode<S: AsRef<str>>(paragraphs: &[S]) -> String {
    serde_json::to_string(&of_paragraphs(paragraphs)).unwrap_or_else(|_| "[]".into())
}

/// The paragraphs of a stored `content`. Text from before blocks (paragraphs separated
/// by a blank line) is read too.
pub fn paragraphs(content: &str) -> Vec<String> {
    match serde_json::from_str::<Vec<Block>>(content.trim_start()) {
        Ok(blocks) => blocks
            .into_iter()
            .map(|b| match b {
                Block::Text(s) => join(&s),
                Block::Img(key) => image::paragraph(&key),
            })
            .filter(|p| !p.is_empty())
            .collect(),
        Err(_) => content
            .replace("\r\n", "\n")
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect(),
    }
}

/// The stored `content` as plain text, paragraphs separated by a blank line.
pub fn plain(content: &str) -> String {
    paragraphs(content).join("\n\n")
}

/// A paragraph from its sentences: a space between them, none after Chinese/Japanese.
fn join(sentences: &[String]) -> String {
    let mut out = String::new();
    for s in sentences {
        let tight = out
            .chars()
            .last()
            .is_some_and(|c| is_cjk(c) || CJK_ENDS.contains(&c) || CLOSING_CJK.contains(&c))
            && s.chars()
                .next()
                .is_some_and(|c| is_cjk(c) || OPENING.contains(&c));
        if !out.is_empty() && !tight {
            out.push(' ');
        }
        out.push_str(s);
    }
    out
}

/// Rewrites every `content` still kept as plain text into blocks (the migration).
pub fn convert_all(conn: &rusqlite::Connection) -> rusqlite::Result<usize> {
    let mut done = 0;
    let mut after = i64::MIN;
    loop {
        let rows: Vec<(i64, String)> = conn
            .prepare(
                "select id, content from chapter_translations where id > ?1 order by id limit 200",
            )?
            .query_map([after], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let Some(last) = rows.last() else {
            return Ok(done);
        };
        after = last.0;
        let mut update =
            conn.prepare("update chapter_translations set content = ?2 where id = ?1")?;
        for (id, content) in &rows {
            if serde_json::from_str::<Vec<Block>>(content.trim_start()).is_ok() {
                continue;
            }
            update.execute(rusqlite::params![id, encode(&paragraphs(content))])?;
            done += 1;
        }
    }
}

const OPENING: &[char] = &['“', '‘', '「', '『', '《', '〈', '（', '(', '«', '【'];
const CLOSING: &[char] = &['”', '’', '」', '』', '》', '〉', '）', ')', '»', '】'];
const CLOSING_CJK: &[char] = &['”', '」', '』', '》', '〉', '）', '】'];
/// End a sentence wherever they are.
const CJK_ENDS: &[char] = &['。', '！', '？', '｡'];
/// End a sentence when a space follows.
const LATIN_ENDS: &[char] = &['.', '!', '?', '…'];
/// Words that end with a dot without ending the sentence.
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "st", "jr", "sr", "prof", "vs", "etc", "e.g", "i.e", "no", "tp",
    "ths", "ts", "gs", "pgs",
];

/// The sentences of one paragraph, trimmed; a line break always ends one.
pub fn sentences(paragraph: &str) -> Vec<String> {
    let (out, balanced) = cut(paragraph, true);
    if balanced {
        out
    } else {
        // A quote that never closes would hold the rest of the paragraph together.
        cut(paragraph, false).0
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF | 0xAC00..=0xD7AF)
}

/// Cuts `p`; with `quotes`, never inside quotes. Returns whether every quote closed.
fn cut(p: &str, quotes: bool) -> (Vec<String>, bool) {
    let chars: Vec<char> = p.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut straight = false; // inside "…"
    let mut i = 0;
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        let s = cur.trim();
        if !s.is_empty() {
            out.push(s.to_string());
        }
        cur.clear();
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            flush(&mut cur, &mut out);
            depth = 0;
            straight = false;
            i += 1;
            continue;
        }
        cur.push(c);
        let inside = |depth: usize, straight: bool| quotes && (depth > 0 || straight);
        let mut closed_quote = false;
        if quotes {
            if OPENING.contains(&c) {
                depth += 1;
            } else if CLOSING.contains(&c) {
                if depth > 0 {
                    depth -= 1;
                    closed_quote = depth == 0 && !straight;
                }
            } else if c == '"' {
                straight = !straight;
                closed_quote = !straight && depth == 0;
            }
        }
        if closed_quote {
            // "…?" / “…！”: the sentence ends with the quote unless it carries on.
            let before = cur.chars().rev().nth(1);
            if before.is_some_and(|b| CJK_ENDS.contains(&b) || LATIN_ENDS.contains(&b)) {
                let mut j = i + 1;
                while j < chars.len() && CLOSING.contains(&chars[j]) {
                    cur.push(chars[j]);
                    j += 1;
                }
                i = j;
                if starts_new(&chars, i, true) {
                    flush(&mut cur, &mut out);
                }
                continue;
            }
            i += 1;
            continue;
        }
        if !inside(depth, straight) && (CJK_ENDS.contains(&c) || LATIN_ENDS.contains(&c)) {
            // Take the whole run: "?!", "...", "。」".
            let mut j = i + 1;
            while j < chars.len()
                && (CJK_ENDS.contains(&chars[j])
                    || LATIN_ENDS.contains(&chars[j])
                    || CLOSING.contains(&chars[j]))
            {
                cur.push(chars[j]);
                j += 1;
            }
            let run: String = chars[i..j].iter().collect();
            i = j;
            let ends = if run.chars().any(|r| CJK_ENDS.contains(&r)) {
                true
            } else if (j < chars.len() && !chars[j].is_whitespace())
                || (run == "." && abbreviation(&cur))
            {
                false // 3.14, "…không", "Mr."
            } else {
                starts_new(&chars, j, false)
            };
            if ends {
                flush(&mut cur, &mut out);
            }
            continue;
        }
        i += 1;
    }
    flush(&mut cur, &mut out);
    (out, depth == 0 && !straight)
}

/// Whether what comes from `at` (after spaces) starts a new sentence.
fn starts_new(chars: &[char], at: usize, after_quote: bool) -> bool {
    let next = chars[at.min(chars.len())..]
        .iter()
        .find(|c| !c.is_whitespace() || **c == '\n');
    match next {
        None | Some('\n') => true,
        Some(&c) if c.is_lowercase() => false,
        // “走吧！”他说。 is one sentence.
        Some(&c) if after_quote && is_cjk(c) => false,
        Some(_) => true,
    }
}

/// Whether the text ends with "Mr." or an initial ("J.").
fn abbreviation(text: &str) -> bool {
    let word = text
        .trim_end_matches('.')
        .rsplit(|c: char| c.is_whitespace() || OPENING.contains(&c) || c == '"')
        .next()
        .unwrap_or("");
    let mut letters = word.chars();
    if let (Some(first), None) = (letters.next(), letters.next()) {
        return first.is_uppercase();
    }
    ABBREVIATIONS.contains(&word.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_paragraphs_and_a_picture() {
        let paragraphs = [
            "Lâm Phong mở mắt, ánh nắng chiếu qua khe cửa. \"Đây là đâu?\" hắn lẩm bẩm.",
            "\u{FFFC}img_0003",
            "Ngoài sân, một lão giả râu bạc đang chậm rãi quét lá. Như thể ông đã đợi hắn từ lâu.",
        ];
        let stored = encode(&paragraphs);
        assert_eq!(
            stored,
            r#"[{"type":"text","content":["Lâm Phong mở mắt, ánh nắng chiếu qua khe cửa.","\"Đây là đâu?\" hắn lẩm bẩm."]},{"type":"img","content":"img_0003"},{"type":"text","content":["Ngoài sân, một lão giả râu bạc đang chậm rãi quét lá.","Như thể ông đã đợi hắn từ lâu."]}]"#
        );
        assert_eq!(super::paragraphs(&stored), paragraphs);
        // Plain text from before blocks reads the same.
        assert_eq!(super::paragraphs(&paragraphs.join("\n\n")), paragraphs);
        let zh = ["林动睁开眼睛。“这是哪里？”他喃喃道。"];
        assert_eq!(super::paragraphs(&encode(&zh)), zh);
    }

    #[test]
    fn chinese() {
        assert_eq!(
            sentences("林动睁开眼睛。“这是哪里？”他喃喃道。“走吧！”“好！”"),
            vec![
                "林动睁开眼睛。",
                "“这是哪里？”他喃喃道。",
                "“走吧！”",
                "“好！”"
            ]
        );
        assert_eq!(
            sentences("窗外下着雨……他沉默了。真的吗？！是的。"),
            vec!["窗外下着雨……他沉默了。", "真的吗？！", "是的。"]
        );
    }

    #[test]
    fn english_and_vietnamese() {
        assert_eq!(
            sentences("Mr. Smith paid 3.5 dollars... then left! Did he? “Yes,” she said. Hắn… không biết. Cuối cùng."),
            vec!["Mr. Smith paid 3.5 dollars... then left!", "Did he?", "“Yes,” she said.", "Hắn… không biết.", "Cuối cùng."]
        );
        assert_eq!(
            sentences("“Ngươi là ai? Từ đâu tới?” Lão giả hỏi. Hắn im lặng"),
            vec!["“Ngươi là ai? Từ đâu tới?”", "Lão giả hỏi.", "Hắn im lặng"]
        );
    }

    #[test]
    fn an_open_quote_does_not_hold_everything() {
        assert_eq!(
            sentences("\"Đi thôi. Trời tối rồi. Mau lên."),
            vec!["\"Đi thôi.", "Trời tối rồi.", "Mau lên."]
        );
        assert_eq!(
            sentences("Dòng một\nDòng hai"),
            vec!["Dòng một", "Dòng hai"]
        );
        assert!(of_paragraphs(&["  ", ""]).is_empty());
    }
}
