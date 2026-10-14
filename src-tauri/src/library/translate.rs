//! AI translation of an imported book's chapter with the user's own LLM (llm.rs).
//!
//! LangGraph has no Rust version, so the flow is the same idea written out: a small
//! graph of nodes over one shared state, each node returning the next one.
//!
//! ```text
//! Detect ──► Route ──► Translate ──┐
//!                 └──► Rewrite  ───┴──► Save ──► End
//! ```
//!
//! - Detect: the LLM names the language of the first 50 words of the book's first
//!   chapter (asked once per book, kept in `ai_books`).
//! - Route: another language than the one asked for → Translate; the same → Rewrite
//!   (polish it in the chosen style, correct in that language).
//! - Save: the result goes in `chapter_translations` as `source = 'ai'` with its
//!   style. That row is the mark that the chapter is done: the queue skips it, and the
//!   input is always the original text (`source <> 'ai'`), never an AI result, so a
//!   rewrite is never rewritten again. The graph also stops after `MAX_STEPS`.

use super::llm::{AiError, Chat};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// Target languages offered: English and Vietnamese.
pub const LANGS: &[&str] = &["en", "vi"];
pub const STYLES: &[&str] = &["literal", "natural", "literary", "casual"];

/// Paragraphs that stand for a picture (U+FFFC + key) pass through untouched.
const IMAGE_MARK: char = '\u{FFFC}';
/// Text sent to the LLM in one go, in characters.
const CHUNK_CHARS: usize = 3500;
/// Words of the first chapter the language is told from.
const DETECT_WORDS: usize = 50;
const MAX_STEPS: usize = 8;

/// What to do with one chapter.
#[derive(Debug, Clone)]
pub struct Request {
    pub novel_id: i64,
    pub chapter_id: i64,
    /// en or vi.
    pub lang: String,
    /// literal, natural, literary or casual.
    pub style: String,
    /// Keep the book's glossary names.
    pub glossary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Translate,
    Rewrite,
}

/// A chapter as the reader shows it.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Translated {
    pub title: String,
    pub paragraphs: Vec<String>,
    pub mode: Mode,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Node {
    Detect,
    Route,
    Translate,
    Rewrite,
    Save,
    End,
}

/// What the nodes share.
struct State {
    req: Request,
    source_lang: Option<String>,
    mode: Option<Mode>,
    title: String,
    paragraphs: Vec<String>,
    out_title: String,
    out: Vec<String>,
}

/// Runs the graph for one chapter and saves the result.
pub fn run(conn: &Connection, llm: &dyn Chat, req: Request) -> Result<Translated, AiError> {
    check_request(&req)?;
    let (title, content) = source_text(conn, req.chapter_id)?;
    let mut st = State {
        req,
        source_lang: None,
        mode: None,
        paragraphs: split_paragraphs(&content),
        title,
        out_title: String::new(),
        out: Vec::new(),
    };
    let mut node = Node::Detect;
    for _ in 0..MAX_STEPS {
        node = match node {
            Node::Detect => {
                st.source_lang = Some(detect(conn, llm, st.req.novel_id)?);
                Node::Route
            }
            Node::Route => {
                if st.source_lang.as_deref() == Some(st.req.lang.as_str()) {
                    st.mode = Some(Mode::Rewrite);
                    Node::Rewrite
                } else {
                    st.mode = Some(Mode::Translate);
                    Node::Translate
                }
            }
            Node::Translate => {
                let terms = if st.req.glossary {
                    glossary(conn, st.req.novel_id, &st.req.lang)?
                } else {
                    Vec::new()
                };
                let system = translate_prompt(
                    st.source_lang.as_deref().unwrap_or("auto"),
                    &st.req.lang,
                    &st.req.style,
                    &terms,
                );
                st.out = run_chunks(llm, &system, &st.paragraphs)?;
                st.out_title = if st.title.trim().is_empty() {
                    String::new()
                } else {
                    clean(&llm.chat(&title_prompt(&st.req.lang), &st.title)?)
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string()
                };
                Node::Save
            }
            Node::Rewrite => {
                let system = rewrite_prompt(&st.req.lang, &st.req.style);
                st.out = run_chunks(llm, &system, &st.paragraphs)?;
                // Same language: the chapter keeps its title.
                st.out_title = st.title.clone();
                Node::Save
            }
            Node::Save => {
                save(conn, &st.req, &st.out_title, &st.out, llm.model())?;
                Node::End
            }
            Node::End => {
                return Ok(Translated {
                    title: st.out_title,
                    paragraphs: st.out,
                    mode: st.mode.unwrap_or(Mode::Translate),
                    model: Some(llm.model().to_string()),
                })
            }
        };
    }
    Err(AiError::Other("translation flow did not finish".into()))
}

fn check_request(req: &Request) -> Result<(), AiError> {
    if !LANGS.contains(&req.lang.as_str()) {
        return Err(AiError::Other(format!("unsupported language {}", req.lang)));
    }
    if !STYLES.contains(&req.style.as_str()) {
        return Err(AiError::Other(format!("unsupported style {}", req.style)));
    }
    Ok(())
}

/// The chapter's own text: the original (or a human one), never an AI result.
fn source_text(conn: &Connection, chapter_id: i64) -> Result<(String, String), AiError> {
    conn.query_row(
        "select t.title, t.content from chapter_translations t
         join chapters c on c.id = t.chapter_id join novels n on n.id = c.novel_id
         where t.chapter_id = ?1 and t.source <> 'ai'
         order by t.lang = n.original_lang desc, t.source = 'original' desc limit 1",
        [chapter_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .ok_or_else(|| AiError::Other("chapter has no text".into()))
}

fn split_paragraphs(content: &str) -> Vec<String> {
    content
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// The book's language as the LLM tells it from the first 50 words of its first
/// chapter; asked once per book.
pub fn detect(conn: &Connection, llm: &dyn Chat, novel_id: i64) -> Result<String, AiError> {
    if let Some(lang) = conn
        .query_row(
            "select source_lang from ai_books where novel_id = ?1",
            [novel_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(lang);
    }
    let first: Option<i64> = conn
        .query_row(
            "select c.id from chapters c where c.novel_id = ?1 and exists (
               select 1 from chapter_translations t where t.chapter_id = c.id and t.source <> 'ai'
                 and length(trim(t.content)) > 0)
             order by c.number limit 1",
            [novel_id],
            |r| r.get(0),
        )
        .optional()?;
    let fallback: String = conn.query_row(
        "select original_lang from novels where id = ?1",
        [novel_id],
        |r| r.get(0),
    )?;
    let Some(first) = first else {
        return Ok(fallback);
    };
    let (_, content) = source_text(conn, first)?;
    let sample = first_words(&content, DETECT_WORDS);
    let reply = llm.chat(DETECT_PROMPT, &sample)?;
    let lang = parse_lang(&reply).unwrap_or(fallback);
    conn.execute(
        "insert into ai_books (novel_id, source_lang) values (?1, ?2)
         on conflict (novel_id) do update set source_lang = excluded.source_lang",
        params![novel_id, lang],
    )?;
    Ok(lang)
}

/// The first `n` words of the text (pictures left out); for scripts without spaces,
/// at most 4 characters a word.
fn first_words(content: &str, n: usize) -> String {
    let words: Vec<&str> = content
        .split_whitespace()
        .filter(|w| !w.starts_with(IMAGE_MARK))
        .take(n)
        .collect();
    words.join(" ").chars().take(n * 4).collect()
}

const DETECT_PROMPT: &str = "You identify the language of a text. Reply with only its ISO 639-1 code in lower case (for example: en, vi, zh, ko, ja), nothing else.";

/// `vi` from "vi", "Vi.", "Language: vi"…
fn parse_lang(reply: &str) -> Option<String> {
    reply
        .split(|c: char| !c.is_ascii_alphabetic())
        .map(str::to_ascii_lowercase)
        .find(|w| w.len() == 2 && w != "is")
}

fn lang_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "vi" => "Vietnamese",
        "zh" => "Chinese",
        "ko" => "Korean",
        "ja" => "Japanese",
        "auto" => "the source language",
        other => other,
    }
}

fn style_rule(style: &str) -> &'static str {
    match style {
        "literal" => "stay close to the original wording and sentence order",
        "literary" => "rich, vivid literary prose that keeps the mood of the story",
        "casual" => "simple, modern, easy everyday language",
        _ => "natural and fluent, as a native novelist would write it",
    }
}

const FORMAT_RULES: &str = "Keep every paragraph as its own paragraph, separated by a blank line, in the same order. Do not merge or drop paragraphs, do not add titles, notes, explanations or quotes around the text. Reply with only the resulting text.";

fn translate_prompt(source: &str, target: &str, style: &str, terms: &[(String, String)]) -> String {
    let mut s = format!(
        "You are a professional novel translator. Translate the user's text from {} into {}. Style: {}. Keep character names and terms consistent. {FORMAT_RULES}",
        lang_name(source),
        lang_name(target),
        style_rule(style),
    );
    if !terms.is_empty() {
        s.push_str("\nAlways translate these names and terms this way:\n");
        for (k, v) in terms {
            s.push_str(&format!("- {k} → {v}\n"));
        }
    }
    s
}

fn rewrite_prompt(lang: &str, style: &str) -> String {
    format!(
        "You are a professional {name} novel editor. The user's text is already in {name}; do not translate it into another language. Rewrite it in this style: {}. Make it grammatically correct and natural {name}: fix spelling, grammar, punctuation and awkward machine-translated phrasing, keep the meaning, events, dialogue and names. Do not summarize or shorten. {FORMAT_RULES}",
        style_rule(style),
        name = lang_name(lang),
    )
}

fn title_prompt(target: &str) -> String {
    format!(
        "Translate this novel chapter title into {}. Reply with only the translated title on one line.",
        lang_name(target)
    )
}

/// The book's glossary for `lang`: (term, name to use).
fn glossary(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "select term_key, value from glossary_entries where novel_id = ?1 and lang = ?2 order by term_key limit 200",
    )?;
    let rows = stmt.query_map(params![novel_id, lang], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Sends the text paragraphs in chunks of about `CHUNK_CHARS`; pictures stay where they are.
fn run_chunks(llm: &dyn Chat, system: &str, paragraphs: &[String]) -> Result<Vec<String>, AiError> {
    let mut out = Vec::with_capacity(paragraphs.len());
    let mut chunk: Vec<&str> = Vec::new();
    let mut size = 0;
    let flush = |chunk: &mut Vec<&str>, out: &mut Vec<String>| -> Result<(), AiError> {
        if chunk.is_empty() {
            return Ok(());
        }
        let reply = llm.chat(system, &chunk.join("\n\n"))?;
        out.extend(split_paragraphs(&clean(&reply)));
        chunk.clear();
        Ok(())
    };
    for p in paragraphs {
        if p.starts_with(IMAGE_MARK) {
            flush(&mut chunk, &mut out)?;
            size = 0;
            out.push(p.clone());
            continue;
        }
        if size > 0 && size + p.len() > CHUNK_CHARS {
            flush(&mut chunk, &mut out)?;
            size = 0;
        }
        size += p.len();
        chunk.push(p);
    }
    flush(&mut chunk, &mut out)?;
    Ok(out)
}

/// A reply without the ``` fence some models put around it.
fn clean(reply: &str) -> String {
    let t = reply.trim();
    let t = t
        .strip_prefix("```")
        .map(|rest| rest.split_once('\n').map(|(_, body)| body).unwrap_or(""))
        .unwrap_or(t);
    t.strip_suffix("```")
        .unwrap_or(t)
        .replace("\r\n", "\n")
        .trim()
        .to_string()
}

fn save(
    conn: &Connection,
    req: &Request,
    title: &str,
    paragraphs: &[String],
    model: &str,
) -> rusqlite::Result<()> {
    let content = paragraphs.join("\n\n");
    conn.execute(
        "insert into chapter_translations (chapter_id, lang, style, source, state, title, content, word_count, model)
         values (?1, ?2, ?3, 'ai', 'published', ?4, ?5, ?6, ?7)
         on conflict (chapter_id, lang, style) do update set title = excluded.title,
           content = excluded.content, word_count = excluded.word_count, model = excluded.model,
           source = 'ai', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![
            req.chapter_id,
            req.lang,
            req.style,
            title,
            content,
            super::word_count(&content),
            model
        ],
    )?;
    Ok(())
}

/// The AI text of a chapter in `lang` and `style`, if it was made.
pub fn get(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: &str,
) -> rusqlite::Result<Option<Translated>> {
    conn.query_row(
        "select t.title, t.content, t.model, coalesce(b.source_lang, n.original_lang)
         from chapter_translations t join chapters c on c.id = t.chapter_id
         join novels n on n.id = c.novel_id left join ai_books b on b.novel_id = n.id
         where t.chapter_id = ?1 and t.lang = ?2 and t.style = ?3 and t.source = 'ai'",
        params![chapter_id, lang, style],
        |r| {
            let content: String = r.get(1)?;
            let source: String = r.get(3)?;
            Ok(Translated {
                title: r.get(0)?,
                paragraphs: split_paragraphs(&content),
                mode: if source == lang {
                    Mode::Rewrite
                } else {
                    Mode::Translate
                },
                model: r.get(2)?,
            })
        },
    )
    .optional()
}

/// The id of a book's chapter at `index` (0-based reading order).
pub fn chapter_at(conn: &Connection, novel_id: i64, index: usize) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "select id from chapters where novel_id = ?1 order by number limit 1 offset ?2",
        params![novel_id, index as i64],
        |r| r.get(0),
    )
    .optional()
}

/// AI versions of a book: per language and style, how many chapters have one.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub lang: String,
    pub style: String,
    pub chapters: i64,
}

pub fn versions(conn: &Connection, novel_id: i64) -> rusqlite::Result<Vec<Version>> {
    let mut stmt = conn.prepare(
        "select t.lang, t.style, count(*) from chapter_translations t
         join chapters c on c.id = t.chapter_id
         where c.novel_id = ?1 and t.source = 'ai' and t.style is not null
         group by t.lang, t.style order by max(t.updated_at) desc",
    )?;
    let rows = stmt.query_map([novel_id], |r| {
        Ok(Version {
            lang: r.get(0)?,
            style: r.get(1)?,
            chapters: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// Takes the LLM settings and the AI queue out of a copy about to be synced.
pub fn strip(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("delete from llm_config; delete from ai_jobs; delete from ai_books;")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Answers like a model: the language code, or the text with a tag.
    pub struct Fake {
        pub detect: &'static str,
        pub calls: Mutex<Vec<(String, String)>>,
    }

    impl Fake {
        pub fn new(detect: &'static str) -> Self {
            Self {
                detect,
                calls: Mutex::new(Vec::new()),
            }
        }
        pub fn count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl Chat for Fake {
        fn chat(&self, system: &str, user: &str) -> Result<String, AiError> {
            self.calls
                .lock()
                .unwrap()
                .push((system.into(), user.into()));
            Ok(if system == DETECT_PROMPT {
                self.detect.to_string()
            } else if system.contains("editor") {
                split_paragraphs(user)
                    .iter()
                    .map(|p| format!("[re] {p}"))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else {
                format!(
                    "```\n{}\n```",
                    split_paragraphs(user)
                        .iter()
                        .map(|p| format!("[tr] {p}"))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                )
            })
        }
        fn model(&self) -> &str {
            "fake-1"
        }
    }

    pub fn book(name: &str) -> (Connection, i64) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("bennovel-ai-{name}-{nanos}"));
        let conn = super::super::db::open(&dir.join("lib.sqlite3")).unwrap();
        conn.execute_batch(
            "insert into authors (id, slug, name) values (1, 'a', 'A');
             insert into novels (id, slug, author_id, original_lang, chapter_count, book_key) values (1, 'b', 1, 'en', 2, 'k');
             insert into novel_translations (novel_id, lang, title) values (1, 'en', 'Book');
             insert into chapters (id, novel_id, number) values (10, 1, 1), (11, 1, 2);
             insert into chapter_translations (chapter_id, lang, source, state, title, content) values
               (10, 'en', 'original', 'published', 'Chapter 1', 'Once upon a time.\n\n\u{FFFC}pic1\n\nThe end.'),
               (11, 'en', 'original', 'published', 'Chapter 2', 'Second chapter.');",
        )
        .unwrap();
        (conn, 1)
    }

    fn req(chapter_id: i64, lang: &str) -> Request {
        Request {
            novel_id: 1,
            chapter_id,
            lang: lang.into(),
            style: "natural".into(),
            glossary: true,
        }
    }

    #[test]
    fn translates_when_the_language_differs() {
        let (conn, _) = book("tr");
        let llm = Fake::new("en");
        let out = run(&conn, &llm, req(10, "vi")).unwrap();
        assert_eq!(out.mode, Mode::Translate);
        assert_eq!(
            out.paragraphs,
            vec!["[tr] Once upon a time.", "\u{FFFC}pic1", "[tr] The end."]
        );
        assert_eq!(out.title, "[tr] Chapter 1");
        // detect + two chunks around the picture + title
        assert_eq!(llm.count(), 4);
        let saved = get(&conn, 10, "vi", "natural").unwrap().unwrap();
        assert_eq!(saved.paragraphs, out.paragraphs);
        assert_eq!(saved.model.as_deref(), Some("fake-1"));
        // The language is asked once per book.
        run(&conn, &llm, req(11, "vi")).unwrap();
        assert_eq!(llm.count(), 6);
        assert_eq!(
            versions(&conn, 1).unwrap(),
            vec![Version {
                lang: "vi".into(),
                style: "natural".into(),
                chapters: 2
            }]
        );
    }

    #[test]
    fn rewrites_the_original_when_the_language_matches() {
        let (conn, _) = book("re");
        let llm = Fake::new("en");
        let out = run(&conn, &llm, req(11, "en")).unwrap();
        assert_eq!(out.mode, Mode::Rewrite);
        assert_eq!(out.paragraphs, vec!["[re] Second chapter."]);
        assert_eq!(out.title, "Chapter 2");
        // Again: the input is still the original, not the rewrite.
        let again = run(&conn, &llm, req(11, "en")).unwrap();
        assert_eq!(again.paragraphs, vec!["[re] Second chapter."]);
        assert_eq!(
            get(&conn, 11, "en", "natural").unwrap().unwrap().mode,
            Mode::Rewrite
        );
        // The reader's original text is untouched.
        let original: String = conn
            .query_row("select content from chapter_translations where chapter_id = 11 and source = 'original'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(original, "Second chapter.");
    }

    #[test]
    fn strip_forgets_the_llm() {
        let (conn, _) = book("strip");
        super::super::llm::save(
            &conn,
            Some(&super::super::llm::Config {
                provider: "custom".into(),
                base_url: "http://x/v1/".into(),
                api_key: "sk".into(),
                model: "m".into(),
            }),
        )
        .unwrap();
        assert!(super::super::llm::settings(&conn).unwrap().unwrap().has_key);
        strip(&conn).unwrap();
        assert!(super::super::llm::load(&conn).unwrap().is_none());
    }

    #[test]
    fn parses_language_replies() {
        assert_eq!(parse_lang("vi").as_deref(), Some("vi"));
        assert_eq!(parse_lang("Language: EN.").as_deref(), Some("en"));
        assert_eq!(parse_lang("The text is zh").as_deref(), Some("zh"));
        assert_eq!(parse_lang("???"), None);
        assert_eq!(first_words("a b c d", 2), "a b");
    }
}
