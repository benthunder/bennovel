//! AI translation of an imported book's chapter with the user's own LLM (llm.rs).
//!
//! The flow is a LangGraph graph, built with the `rust-langgraph` crate:
//!
//! ```text
//! START ─► detect ─► recall ─► translate ─► rewrite ─► save ─► learn ─► END
//! ```
//!
//! - detect: the LLM names the language of the first 50 words of the book's first
//!   chapter (asked once per book, kept in `ai_books`), and picks the mode: another
//!   language than the one asked for → translate; the same → rewrite (polish it in
//!   the chosen style, correct in that language).
//! - recall: the book's wiki entries the chapter names, and those nearest to it in
//!   meaning (sqlite-vec), go in the prompt with their relations (wiki.rs).
//! - translate and rewrite each run only in their mode and pass the state on
//!   otherwise. (rust-langgraph 0.1.1 accepts `add_conditional_edges` but `compile`
//!   drops them, so the branch is made by the nodes; once fixed upstream it can be a
//!   conditional edge from detect.)
//! - save: the result goes in `chapter_translations` as `source = 'ai'` with its
//!   style. That row is the mark that the chapter is done: the queue skips it, and the
//!   input is always the original text (`source <> 'ai'`), never an AI result, so a
//!   rewrite is never rewritten again. The graph has no loop and a step limit too.
//! - learn: the LLM reads the chapter into the wiki with tool calls (wiki.rs), when
//!   the book's wiki is updated by translating; a failure there is kept for the wiki
//!   screen and does not fail the translation. A chapter translated from the reader
//!   learns in the background, so the reader gets the text as soon as it is saved.
//!
//! A chapter goes to the LLM in parts of about `CHUNK_CHARS`, `PARALLEL` at a time.

use super::llm::{AiError, Chat};
use rusqlite::{params, Connection, OptionalExtension};
use rust_langgraph::{Config, Error as GraphError, State, StateGraph};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Target languages offered: English and Vietnamese.
pub const LANGS: &[&str] = &["en", "vi"];
pub const STYLES: &[&str] = &["literal", "natural", "literary", "casual"];

/// Paragraphs that stand for a picture (U+FFFC + key) pass through untouched.
const IMAGE_MARK: char = '\u{FFFC}';
/// Text sent to the LLM in one go, in characters.
const CHUNK_CHARS: usize = 3500;
/// Words of the first chapter the language is told from.
const DETECT_WORDS: usize = 50;
/// Supersteps allowed: the six nodes, run once each.
const MAX_STEPS: usize = 8;
/// Parts of a chapter sent to the LLM at the same time.
const PARALLEL: usize = 3;

/// Called with (parts done, parts in all) as a chapter's parts come back.
pub type OnProgress = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// A node of the graph, as the app shows where a chapter is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Detect,
    Recall,
    Translate,
    Rewrite,
    Save,
    Learn,
}

/// Called as each node starts its work.
pub type OnStage = Arc<dyn Fn(Stage) + Send + Sync>;

/// How a run reports and finishes; the default suits the background queue.
#[derive(Default, Clone)]
pub struct Hooks {
    pub progress: Option<OnProgress>,
    pub stage: Option<OnStage>,
    /// Learn the wiki on another thread after the save instead of before returning.
    pub learn_in_background: bool,
}

/// What to do with one chapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// The graph's state: everything the nodes read and write. Loaded from the library
/// before the run; `save` writes it back.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Flow {
    db: String,
    req: Request,
    /// The book's language when it was asked before.
    known_lang: Option<String>,
    /// The first 50 words, to ask it from; the book's guess when there is no text.
    sample: Option<String>,
    guess_lang: String,
    source_lang: Option<String>,
    mode: Option<Mode>,
    terms: Vec<(String, String)>,
    /// The wiki part of the prompt (recall).
    wiki: String,
    title: String,
    paragraphs: Vec<String>,
    out_title: String,
    out: Vec<String>,
}

impl State for Flow {
    /// Each node hands back the whole state, so its write replaces the last one.
    fn merge(&mut self, other: Self) -> rust_langgraph::Result<()> {
        *self = other;
        Ok(())
    }
}

/// Runs the graph for one chapter and saves the result. `conn` must be a library
/// file on disk (the save node opens it again).
#[cfg(test)]
pub fn run(conn: &Connection, llm: Arc<dyn Chat>, req: Request) -> Result<Translated, AiError> {
    run_with(conn, llm, req, Hooks::default())
}

/// `run`, reporting progress and learning the wiki as `hooks` say.
pub fn run_with(
    conn: &Connection,
    llm: Arc<dyn Chat>,
    req: Request,
    hooks: Hooks,
) -> Result<Translated, AiError> {
    check_request(&req)?;
    let (title, content) = source_text(conn, req.chapter_id)?;
    let (known_lang, sample, guess_lang) = detect_input(conn, req.novel_id)?;
    let terms = if req.glossary {
        glossary(conn, req.novel_id, &req.lang)?
    } else {
        Vec::new()
    };
    let flow = Flow {
        db: conn.path().unwrap_or_default().to_string(),
        req,
        known_lang,
        sample,
        guess_lang,
        source_lang: None,
        mode: None,
        terms,
        wiki: String::new(),
        title,
        paragraphs: split_paragraphs(&content),
        out_title: String::new(),
        out: Vec::new(),
    };
    let failed: Arc<Mutex<Option<AiError>>> = Arc::default();
    let model = llm.model().to_string();
    let mut app = graph(llm, failed.clone(), hooks)
        .compile(None)
        .map_err(|e| AiError::Other(e.to_string()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AiError::Other(e.to_string()))?;
    let config = Config::default().with_recursion_limit(MAX_STEPS);
    let out = runtime.block_on(app.invoke(flow, config)).map_err(|e| {
        failed
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| AiError::Other(e.to_string()))
    })?;
    if out.out.is_empty() && !out.paragraphs.is_empty() {
        return Err(AiError::Other("translation flow did not finish".into()));
    }
    Ok(Translated {
        title: out.out_title,
        paragraphs: out.out,
        mode: out.mode.unwrap_or(Mode::Translate),
        model: Some(model),
    })
}

/// The nodes and edges. A node's AiError goes in `failed` (the graph only carries
/// its own error type), so the caller gets it back as it was.
fn graph(
    llm: Arc<dyn Chat>,
    failed: Arc<Mutex<Option<AiError>>>,
    hooks: Hooks,
) -> StateGraph<Flow> {
    // Runs a node's work and turns an AiError into a graph error, keeping it.
    fn step(
        failed: &Arc<Mutex<Option<AiError>>>,
        r: Result<Flow, AiError>,
    ) -> rust_langgraph::Result<Flow> {
        r.map_err(|e| {
            let text = e.to_string();
            *failed.lock().unwrap() = Some(e);
            GraphError::ExecutionError(text)
        })
    }

    // Tells the hook a node started.
    let enter = {
        let hook = hooks.stage.clone();
        move |s: Stage| {
            if let Some(h) = &hook {
                h(s);
            }
        }
    };

    let mut g = StateGraph::new();
    {
        let (llm, failed, enter) = (llm.clone(), failed.clone(), enter.clone());
        g.add_node("detect", move |mut f: Flow, _: &Config| {
            let (llm, failed) = (llm.clone(), failed.clone());
            enter(Stage::Detect);
            async move {
                let r = (|| {
                    let lang = match (&f.known_lang, &f.sample) {
                        (Some(l), _) => l.clone(),
                        (None, Some(sample)) => {
                            ask_lang(llm.as_ref(), sample)?.unwrap_or_else(|| f.guess_lang.clone())
                        }
                        (None, None) => f.guess_lang.clone(),
                    };
                    f.mode = Some(if lang == f.req.lang {
                        Mode::Rewrite
                    } else {
                        Mode::Translate
                    });
                    f.source_lang = Some(lang);
                    Ok(f)
                })();
                step(&failed, r)
            }
        });
    }
    {
        let (llm, failed, enter) = (llm.clone(), failed.clone(), enter.clone());
        g.add_node("recall", move |mut f: Flow, _: &Config| {
            let (llm, failed) = (llm.clone(), failed.clone());
            enter(Stage::Recall);
            async move {
                let r = (|| {
                    let conn = super::db::open(std::path::Path::new(&f.db))?;
                    f.wiki = super::wiki::context(
                        &conn,
                        llm.as_ref(),
                        f.req.novel_id,
                        &f.paragraphs.join("\n\n"),
                        &f.req.lang,
                    )?;
                    Ok(f)
                })();
                step(&failed, r)
            }
        });
    }
    {
        let (llm, failed, progress) = (llm.clone(), failed.clone(), hooks.progress.clone());
        let enter = enter.clone();
        g.add_node("translate", move |mut f: Flow, _: &Config| {
            let (llm, failed, progress) = (llm.clone(), failed.clone(), progress.clone());
            if f.mode == Some(Mode::Translate) {
                enter(Stage::Translate);
            }
            async move {
                if f.mode != Some(Mode::Translate) {
                    return Ok(f);
                }
                let r = (|| {
                    let system = translate_prompt(
                        f.source_lang.as_deref().unwrap_or("auto"),
                        &f.req.lang,
                        &f.req.style,
                        &f.terms,
                        &f.wiki,
                    );
                    let title = (!f.title.trim().is_empty())
                        .then(|| (title_prompt(&f.req.lang), f.title.clone()));
                    let (out, title) = run_chunks(
                        llm.as_ref(),
                        &system,
                        &f.paragraphs,
                        title,
                        progress.as_deref(),
                    )?;
                    f.out = out;
                    f.out_title = title
                        .map(|t| clean(&t).lines().next().unwrap_or_default().to_string())
                        .unwrap_or_default();
                    Ok(f)
                })();
                step(&failed, r)
            }
        });
    }
    {
        let (llm, failed, progress) = (llm.clone(), failed.clone(), hooks.progress.clone());
        let enter = enter.clone();
        g.add_node("rewrite", move |mut f: Flow, _: &Config| {
            let (llm, failed, progress) = (llm.clone(), failed.clone(), progress.clone());
            if f.mode == Some(Mode::Rewrite) {
                enter(Stage::Rewrite);
            }
            async move {
                if f.mode != Some(Mode::Rewrite) {
                    return Ok(f);
                }
                let r = (|| {
                    let system = rewrite_prompt(&f.req.lang, &f.req.style, &f.wiki);
                    f.out =
                        run_chunks(llm.as_ref(), &system, &f.paragraphs, None, progress.as_deref())?
                            .0;
                    // Same language: the chapter keeps its title.
                    f.out_title = f.title.clone();
                    Ok(f)
                })();
                step(&failed, r)
            }
        });
    }
    {
        let (llm, failed, enter) = (llm.clone(), failed.clone(), enter.clone());
        g.add_node("save", move |f: Flow, _: &Config| {
            let (llm, failed) = (llm.clone(), failed.clone());
            enter(Stage::Save);
            async move {
                let r = (|| {
                    let conn = super::db::open(std::path::Path::new(&f.db))?;
                    if f.known_lang.is_none() {
                        if let Some(lang) = &f.source_lang {
                            remember_lang(&conn, f.req.novel_id, lang)?;
                        }
                    }
                    save(&conn, &f.req, &f.out_title, &f.out, llm.model())?;
                    Ok(f)
                })();
                step(&failed, r)
            }
        });
    }
    {
        let (llm, failed, background) = (llm.clone(), failed.clone(), hooks.learn_in_background);
        g.add_node("learn", move |f: Flow, _: &Config| {
            let (llm, failed) = (llm.clone(), failed.clone());
            if !background {
                enter(Stage::Learn);
            }
            async move {
                if background {
                    let later = f.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = learn(&later, llm.as_ref()) {
                            eprintln!("wiki learn: {e}");
                        }
                    });
                    return Ok(f);
                }
                let r = learn(&f, llm.as_ref()).map(|_| f);
                step(&failed, r)
            }
        });
    }
    g.set_entry_point("detect");
    g.add_edge("detect", "recall");
    g.add_edge("recall", "translate");
    g.add_edge("translate", "rewrite");
    g.add_edge("rewrite", "save");
    g.add_edge("save", "learn");
    g.set_finish_point("learn");
    g
}

/// The learn node's work: the chapter into the book's wiki (wiki.rs).
fn learn(f: &Flow, llm: &dyn Chat) -> Result<(), AiError> {
    let conn = super::db::open(std::path::Path::new(&f.db))?;
    super::wiki::learn_quietly(
        &conn,
        llm,
        f.req.novel_id,
        f.req.chapter_id,
        &f.req.lang,
        &f.paragraphs.join("\n\n"),
        &f.out.join("\n\n"),
    )?;
    Ok(())
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

/// What `detect` needs: the language found before, else the first 50 words of the
/// first chapter with text (None when there is none) and the import's guess.
fn detect_input(
    conn: &Connection,
    novel_id: i64,
) -> Result<(Option<String>, Option<String>, String), AiError> {
    let guess: String = conn.query_row(
        "select original_lang from novels where id = ?1",
        [novel_id],
        |r| r.get(0),
    )?;
    let known = conn
        .query_row(
            "select source_lang from ai_books where novel_id = ?1",
            [novel_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    if known.is_some() {
        return Ok((known, None, guess));
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
    let sample = match first {
        Some(id) => Some(first_words(&source_text(conn, id)?.1, DETECT_WORDS)),
        None => None,
    };
    Ok((None, sample, guess))
}

/// The language the LLM names for `sample`, if it gives a code.
fn ask_lang(llm: &dyn Chat, sample: &str) -> Result<Option<String>, AiError> {
    Ok(parse_lang(&llm.chat(DETECT_PROMPT, sample)?))
}

fn remember_lang(conn: &Connection, novel_id: i64, lang: &str) -> rusqlite::Result<()> {
    conn.execute(
        "insert into ai_books (novel_id, source_lang) values (?1, ?2)
         on conflict (novel_id) do update set source_lang = excluded.source_lang",
        params![novel_id, lang],
    )?;
    Ok(())
}

/// The book's language as the LLM tells it from the first 50 words of its first
/// chapter (the detect node, on its own); asked once per book.
pub fn detect(conn: &Connection, llm: &dyn Chat, novel_id: i64) -> Result<String, AiError> {
    let (known, sample, guess) = detect_input(conn, novel_id)?;
    if let Some(lang) = known {
        return Ok(lang);
    }
    let Some(sample) = sample else {
        return Ok(guess);
    };
    let lang = ask_lang(llm, &sample)?.unwrap_or(guess);
    remember_lang(conn, novel_id, &lang)?;
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

pub(crate) fn lang_name(code: &str) -> &str {
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
        "literal" => "stay close to the original meaning and wording, adding nothing, while still using correct sentence order",
        "literary" => "rich, vivid literary prose that keeps the mood of the story",
        "casual" => "simple, modern, easy everyday language",
        _ => "natural and fluent, as a native novelist would write it",
    }
}

const FORMAT_RULES: &str = "Keep every paragraph as its own paragraph, separated by a blank line, in the same order. Do not merge or drop paragraphs, do not add titles, notes, explanations or quotes around the text. Reply with only the resulting text.";

/// How a sentence must be built in the target language. Novels from Chinese often come
/// as word-for-word "convert" text that keeps the Chinese order (modifier before the
/// noun, adverbs and places in front of the verb, "ta đối với hắn nói"), so every
/// sentence is rebuilt, not just its words swapped.
fn grammar_rules(lang: &str) -> &'static str {
    match lang {
        "vi" => "Write correct, natural Vietnamese sentences, never word-for-word. Rebuild every sentence in Vietnamese order: subject, then verb, then object (\"Hắn nói với nàng\", not \"Hắn đối với nàng nói\"); the noun comes before what describes it and the owner comes last (\"thanh kiếm sắc bén của hắn\", not \"hắn sắc bén kiếm\"); time and place usually open or close the sentence, not sit between subject and verb. Prefer pure Vietnamese words over clumsy Sino-Vietnamese ones when the meaning is the same, but keep cultivation terms, titles and names in Hán Việt. Use pronouns that fit who speaks to whom (ta, ngươi, hắn, nàng, huynh, muội, sư phụ…) and keep them consistent.",
        _ => "Write correct, natural English sentences, never word-for-word. Rebuild every sentence in English order: subject, then verb, then object; put adjectives before nouns and time or place phrases at the start or end of the sentence; use the right tense (mostly past tense for narration), articles and pronouns (he, she, they), and split sentences that run on.",
    }
}

fn translate_prompt(
    source: &str,
    target: &str,
    style: &str,
    terms: &[(String, String)],
    wiki: &str,
) -> String {
    let mut s = format!(
        "You are a professional novel translator. Translate the user's text from {} into {}. Translate the meaning, then {} Style: {}. Keep character names and terms consistent. {FORMAT_RULES}",
        lang_name(source),
        lang_name(target),
        grammar_rules(target),
        style_rule(style),
    );
    if !terms.is_empty() {
        s.push_str("\nAlways translate these names and terms this way:\n");
        for (k, v) in terms {
            s.push_str(&format!("- {k} → {v}\n"));
        }
    }
    s.push_str(wiki);
    s
}

fn rewrite_prompt(lang: &str, style: &str, wiki: &str) -> String {
    let s = format!(
        "You are a professional {name} novel editor. The user's text is already in {name} but may be a rough machine translation; do not translate it into another language. Rewrite it so it reads as if written in {name} from the start. {} Fix spelling, punctuation and wrong word choices. Style: {}. Keep the meaning, events, dialogue and names; do not summarize, shorten or add anything. {FORMAT_RULES}",
        grammar_rules(lang),
        style_rule(style),
        name = lang_name(lang),
    );
    s + wiki
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

/// Sends the text paragraphs in parts of about `CHUNK_CHARS`, `PARALLEL` at a time,
/// and puts the replies back in order; pictures stay where they are. `title` (a
/// system prompt and the title) is asked alongside the parts. The first error stops
/// what has not started yet and is returned.
fn run_chunks(
    llm: &dyn Chat,
    system: &str,
    paragraphs: &[String],
    title: Option<(String, String)>,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> Result<(Vec<String>, Option<String>), AiError> {
    enum Piece<'a> {
        Picture(&'a str),
        Text(String),
    }
    let mut pieces = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    let mut size = 0;
    for p in paragraphs {
        if p.starts_with(IMAGE_MARK) {
            if !chunk.is_empty() {
                pieces.push(Piece::Text(chunk.join("\n\n")));
                chunk.clear();
            }
            size = 0;
            pieces.push(Piece::Picture(p));
            continue;
        }
        if size > 0 && size + p.len() > CHUNK_CHARS {
            pieces.push(Piece::Text(chunk.join("\n\n")));
            chunk.clear();
            size = 0;
        }
        size += p.len();
        chunk.push(p);
    }
    if !chunk.is_empty() {
        pieces.push(Piece::Text(chunk.join("\n\n")));
    }

    // The requests: each text part, then the title.
    let mut asks: Vec<(&str, &str)> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Text(t) => Some((system, t.as_str())),
            Piece::Picture(_) => None,
        })
        .collect();
    let parts = asks.len();
    if let Some((prompt, text)) = &title {
        asks.push((prompt.as_str(), text.as_str()));
    }
    let total = asks.len();
    let replies: Vec<Mutex<Option<String>>> = (0..total).map(|_| Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let failed: Mutex<Option<AiError>> = Mutex::new(None);
    if let Some(p) = progress {
        p(0, total);
    }
    std::thread::scope(|s| {
        for _ in 0..PARALLEL.min(total) {
            s.spawn(|| loop {
                if failed.lock().unwrap().is_some() {
                    return;
                }
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some((prompt, text)) = asks.get(i) else {
                    return;
                };
                match llm.chat(prompt, text) {
                    Ok(reply) => {
                        *replies[i].lock().unwrap() = Some(reply);
                        let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        if let Some(p) = progress {
                            p(n, total);
                        }
                    }
                    Err(e) => {
                        failed.lock().unwrap().get_or_insert(e);
                        return;
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().unwrap() {
        return Err(e);
    }
    let mut replies = replies
        .into_iter()
        .map(|r| r.into_inner().unwrap().unwrap_or_default());
    let mut out = Vec::with_capacity(paragraphs.len());
    for piece in &pieces {
        match piece {
            Piece::Picture(p) => out.push(p.to_string()),
            Piece::Text(_) => out.extend(split_paragraphs(&clean(&replies.next().unwrap_or_default()))),
        }
    }
    let title = (total > parts).then(|| replies.next().unwrap_or_default());
    Ok((out, title))
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

/// One AI version of a chapter, for the marks in the reader and the chapter lists.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    /// The chapter's place in reading order (0-based).
    pub index: i64,
    pub lang: String,
    pub style: String,
    pub model: Option<String>,
}

/// Every chapter's AI versions, newest first within a chapter.
pub fn marks(conn: &Connection, novel_id: i64) -> rusqlite::Result<Vec<Mark>> {
    let mut stmt = conn.prepare(
        "with c as (select id, row_number() over (order by number) - 1 as idx
                    from chapters where novel_id = ?1)
         select c.idx, t.lang, t.style, t.model from chapter_translations t join c on c.id = t.chapter_id
         where t.source = 'ai' and t.style is not null
         order by c.idx, t.updated_at desc",
    )?;
    let rows = stmt.query_map([novel_id], |r| {
        Ok(Mark {
            index: r.get(0)?,
            lang: r.get(1)?,
            style: r.get(2)?,
            model: r.get(3)?,
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
               (11, 'en', 'original', 'published', 'Chapter 2', 'Second chapter.');
             insert into wiki_books (novel_id, auto) values (1, 0);",
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
        let llm = std::sync::Arc::new(Fake::new("en"));
        let out = run(&conn, llm.clone(), req(10, "vi")).unwrap();
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
        run(&conn, llm.clone(), req(11, "vi")).unwrap();
        assert_eq!(llm.count(), 6);
        // Each chapter carries its own mark.
        assert_eq!(
            marks(&conn, 1)
                .unwrap()
                .iter()
                .map(|m| (m.index, m.lang.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "vi"), (1, "vi")]
        );
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
        let llm = std::sync::Arc::new(Fake::new("en"));
        let out = run(&conn, llm.clone(), req(11, "en")).unwrap();
        assert_eq!(out.mode, Mode::Rewrite);
        assert_eq!(out.paragraphs, vec!["[re] Second chapter."]);
        assert_eq!(out.title, "Chapter 2");
        // Again: the input is still the original, not the rewrite.
        let again = run(&conn, llm.clone(), req(11, "en")).unwrap();
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
                embed_model: String::new(),
            }),
        )
        .unwrap();
        assert!(super::super::llm::settings(&conn).unwrap().unwrap().has_key);
        strip(&conn).unwrap();
        assert!(super::super::llm::load(&conn).unwrap().is_none());
    }

    struct Refused;
    impl Chat for Refused {
        fn chat(&self, _: &str, _: &str) -> Result<String, AiError> {
            Err(AiError::Auth("bad key".into()))
        }
        fn model(&self) -> &str {
            "x"
        }
    }

    #[test]
    fn an_llm_error_comes_out_of_the_graph_as_it_was() {
        let (conn, _) = book("err");
        match run(&conn, std::sync::Arc::new(Refused), req(10, "vi")) {
            Err(AiError::Auth(m)) => assert_eq!(m, "bad key"),
            other => panic!("{other:?}"),
        }
        assert!(get(&conn, 10, "vi", "natural").unwrap().is_none());
    }

    #[test]
    fn translates_chapter_after_chapter_from_the_app_runtime() {
        // As the app does it: each request on a blocking thread of the async runtime.
        let (conn, _) = book("rt");
        let db = conn.path().unwrap().to_string();
        let llm = std::sync::Arc::new(Fake::new("en"));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for chapter in [10, 11, 10] {
            let (db, llm) = (db.clone(), llm.clone());
            let out = rt
                .block_on(async move {
                    tokio::task::spawn_blocking(move || {
                        let conn = super::super::db::open(std::path::Path::new(&db)).unwrap();
                        run(&conn, llm, req(chapter, "vi"))
                    })
                    .await
                })
                .unwrap()
                .unwrap();
            assert!(out.paragraphs[0].starts_with("[tr] "));
        }
        assert_eq!(marks(&conn, 1).unwrap().len(), 2);
    }

    #[test]
    fn prompts_ask_for_the_target_word_order() {
        assert!(rewrite_prompt("vi", "natural", "").contains("subject, then verb, then object"));
        assert!(translate_prompt("zh", "en", "literal", &[], "")
            .contains("subject, then verb, then object"));
        assert!(!rewrite_prompt("vi", "natural", "").contains("English order"));
    }

    #[test]
    fn the_wiki_goes_in_the_prompt_and_learns_from_the_chapter() {
        let (conn, _) = book("wiki");
        super::super::wiki::set_auto(&conn, 1, true).unwrap();
        conn.execute(
            "insert into wiki_entries (uid, novel_id, kind, name, names, updated_at)
             values ('u1', 1, 'term', 'Second', '{\"vi\":\"Thứ Hai\"}', 1)",
            [],
        )
        .unwrap();
        let llm = std::sync::Arc::new(Fake::new("en"));
        run(&conn, llm.clone(), req(11, "vi")).unwrap();
        let calls = llm.calls.lock().unwrap();
        let translate = calls.iter().find(|(s, _)| s.contains("novel translator")).unwrap();
        assert!(translate.0.contains("- Second → Thứ Hai (term)"));
        // Fake has no tools, so the wiki was asked for as JSON after the translation.
        assert!(calls.last().unwrap().0.contains("JSON"));
        drop(calls);
        let done: i64 = conn
            .query_row("select count(*) from wiki_chapters where chapter_id = 11", [], |r| r.get(0))
            .unwrap();
        assert_eq!(done, 1);
    }

    #[test]
    fn long_chapters_go_in_parts_at_once_and_come_back_in_order() {
        let paragraphs: Vec<String> = (0..12)
            .map(|i| format!("p{i} {}", "x".repeat(1200)))
            .chain(std::iter::once("\u{FFFC}pic".to_string()))
            .chain(std::iter::once("last".to_string()))
            .collect();
        let llm = Fake::new("en");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let progress = move |done: usize, total: usize| log.lock().unwrap().push((done, total));
        let (out, title) = run_chunks(
            &llm,
            "novel translator",
            &paragraphs,
            Some(("title".into(), "Chapter 1".into())),
            Some(&progress),
        )
        .unwrap();
        let want: Vec<String> = paragraphs
            .iter()
            .map(|p| if p.starts_with(IMAGE_MARK) { p.clone() } else { format!("[tr] {p}") })
            .collect();
        assert_eq!(out, want);
        assert_eq!(title.as_deref(), Some("```\n[tr] Chapter 1\n```"));
        // 12 long paragraphs make 6 parts, the last line a 7th, plus the title.
        assert_eq!(llm.count(), 8);
        assert_eq!(seen.lock().unwrap().last(), Some(&(8, 8)));
    }

    #[test]
    fn a_failed_part_fails_the_chapter() {
        struct Down;
        impl Chat for Down {
            fn chat(&self, _: &str, _: &str) -> Result<String, AiError> {
                Err(AiError::Timeout)
            }
            fn model(&self) -> &str {
                "down"
            }
        }
        let paragraphs = vec!["a".to_string(), "b".to_string()];
        assert!(matches!(
            run_chunks(&Down, "x", &paragraphs, None, None),
            Err(AiError::Timeout)
        ));
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
