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
//! - recall: the book's wiki entries the chapter names go in the prompt with their
//!   relations (wiki.rs), found by name (names.rs).
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
//! Each part's reply is kept in `ai_parts` as it comes back, so a chapter that is
//! paused, or that fails on one part, carries on from the parts it has the next time
//! instead of sending them all again; the save node clears them.
//!
//! A part is whole blocks of the original's `content` (blocks.rs), never a cut one,
//! and the translation keeps one block per block of the original, in the same place
//! of the array. As a part comes back its blocks are written at their places in the
//! translation's `content` (the row is a `draft`, the blocks still to come `null`);
//! the save node publishes it. Only a published row counts as a translation.

use super::llm::{AiError, Chat};
use rusqlite::{params, Connection, OptionalExtension};
use rust_langgraph::{Config, Error as GraphError, State, StateGraph};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Target languages offered: English and Vietnamese.
pub const LANGS: &[&str] = &["en", "vi"];
pub const STYLES: &[&str] = &["literal", "natural", "literary", "casual"];
/// Genres a book can be set to (`ai_book_prefs`); forms of address follow them. '' is
/// none chosen.
pub const GENRES: &[&str] = &[
    "wuxia",
    "xianxia",
    "supernatural",
    "romance",
    "politics",
    "nonfiction",
];

/// Paragraphs that stand for a picture (U+FFFC + key) pass through untouched.
const IMAGE_MARK: char = '\u{FFFC}';
/// Text sent to the LLM in one go, in characters.
const CHUNK_CHARS: usize = 3500;
/// Words of the first chapter the language is told from.
/// Supersteps allowed: the six nodes, run once each.
const MAX_STEPS: usize = 8;
/// Parts of a chapter sent to the LLM at the same time.
const PARALLEL: usize = 3;
/// Where a reply's translation ends and the wiki facts of that part begin.
const WIKI_MARK: &str = "<<<WIKI>>>";

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
    /// Leave the wiki facts the requests brought (kept by the save node) to the wiki
    /// queue (queue.rs); learn here only when they brought none.
    pub learn_found_later: bool,
    /// Set to stop the chapter: no new part goes to the LLM, the parts in flight are
    /// kept as they come back, and the run ends with `AiError::Stopped`.
    pub stop: Option<Arc<AtomicBool>>,
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
    /// `AUTO`: the model finds the language itself.
    source_lang: Option<String>,
    mode: Option<Mode>,
    terms: Vec<(String, String)>,
    /// The proper nouns the chapter names and what the wiki knows of them (recall), put
    /// in each part's prompt.
    recall: super::wiki::Recall,
    /// The book's genre (`GENRES`, or '') and the user's own extra instructions.
    genre: String,
    instructions: String,
    /// The cast the chapter names: genders, ages and forms of address found before
    /// (cast.rs), put in each part's prompt.
    cast: super::cast::Cast,
    title: String,
    paragraphs: Vec<String>,
    out_title: String,
    out: Vec<String>,
    /// The wiki facts each request found in its part, one JSON object per part: kept
    /// with the chapter, and read into the wiki when the book's wiki learns.
    found: Vec<String>,
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
    let cast = super::cast::load(conn, req.novel_id, &content, &req.lang)?;
    let terms = if req.glossary {
        glossary(conn, req.novel_id, &req.lang)?
    } else {
        Vec::new()
    };
    let Prefs {
        genre,
        instructions,
    } = prefs(conn, req.novel_id)?;
    let flow = Flow {
        db: conn.path().unwrap_or_default().to_string(),
        req,
        source_lang: None,
        mode: None,
        terms,
        recall: super::wiki::Recall::default(),
        genre,
        instructions,
        cast,
        title,
        paragraphs: split_paragraphs(&content),
        out_title: String::new(),
        out: Vec::new(),
        found: Vec::new(),
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
        // `out` has a place for every block; the reader gets those with text.
        paragraphs: out.out.into_iter().filter(|p| !p.is_empty()).collect(),
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
        let (failed, enter) = (failed.clone(), enter.clone());
        g.add_node("detect", move |mut f: Flow, _: &Config| {
            let failed = failed.clone();
            enter(Stage::Detect);
            async move {
                // The app does not tell the book's language any more: the model finds
                // it from the text itself (titles or names in another script must not
                // decide it), and translates, or rewrites a text already in the target
                // language, in the same request.
                f.mode = Some(Mode::Translate);
                f.source_lang = Some(AUTO.to_string());
                step(&failed, Ok(f))
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
                    super::wiki::index_names(&conn, None, f.req.novel_id, &f.req.lang)?;
                    f.recall = super::wiki::recall(
                        &conn,
                        Some(llm.as_ref()),
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
        let (enter, stop) = (enter.clone(), hooks.stop.clone());
        g.add_node("translate", move |mut f: Flow, _: &Config| {
            let (llm, failed, progress) = (llm.clone(), failed.clone(), progress.clone());
            let stop = stop.clone();
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
                        "",
                    ) + &book_rules(&f.genre, &f.req.lang, &f.instructions);
                    let extract =
                        super::wiki::extract_prompt(super::wiki::WIKI_LANG, WIKI_MARK, &f.genre);
                    let system = |part: &str| {
                        format!("{system}{}{}", part_recall(&f, part), f.cast.prompt(part))
                    };
                    let lookup = |q: &str| lookup_name(&f, llm.as_ref(), q);
                    let keep = |facts: &str| keep_part(&f, facts);
                    let tools = part_tools(&f, &lookup, &keep);
                    let title = (!f.title.trim().is_empty())
                        .then(|| (title_prompt(&f.req.lang), f.title.clone()));
                    let cache = PartCache::open(&f.db, &f.req)?
                        .map(|c| c.with_draft(&f.title, &f.paragraphs));
                    let (out, title, found) = run_chunks_with(
                        llm.as_ref(),
                        &system,
                        &extract,
                        Some(&tools),
                        &f.paragraphs,
                        title,
                        progress.as_deref(),
                        cache.as_ref(),
                        stop.as_deref(),
                    )?;
                    f.out = out;
                    f.found = found;
                    f.out_title = title
                        .map(|t| clean(&t).lines().next().unwrap_or_default().to_string())
                        .unwrap_or_default();
                    settle_names(&mut f);
                    Ok(f)
                })();
                step(&failed, r)
            }
        });
    }
    {
        let (llm, failed, progress) = (llm.clone(), failed.clone(), hooks.progress.clone());
        let (enter, stop) = (enter.clone(), hooks.stop.clone());
        g.add_node("rewrite", move |mut f: Flow, _: &Config| {
            let (llm, failed, progress) = (llm.clone(), failed.clone(), progress.clone());
            let stop = stop.clone();
            if f.mode == Some(Mode::Rewrite) {
                enter(Stage::Rewrite);
            }
            async move {
                if f.mode != Some(Mode::Rewrite) {
                    return Ok(f);
                }
                let r = (|| {
                    let system = rewrite_prompt(&f.req.lang, &f.req.style, "")
                        + &book_rules(&f.genre, &f.req.lang, &f.instructions);
                    let extract =
                        super::wiki::extract_prompt(super::wiki::WIKI_LANG, WIKI_MARK, &f.genre);
                    let system = |part: &str| {
                        format!("{system}{}{}", part_recall(&f, part), f.cast.prompt(part))
                    };
                    let lookup = |q: &str| lookup_name(&f, llm.as_ref(), q);
                    let keep = |facts: &str| keep_part(&f, facts);
                    let tools = part_tools(&f, &lookup, &keep);
                    let cache = PartCache::open(&f.db, &f.req)?
                        .map(|c| c.with_draft(&f.title, &f.paragraphs));
                    let (out, _, found) = run_chunks_with(
                        llm.as_ref(),
                        &system,
                        &extract,
                        Some(&tools),
                        &f.paragraphs,
                        None,
                        progress.as_deref(),
                        cache.as_ref(),
                        stop.as_deref(),
                    )?;
                    (f.out, f.found) = (out, found);
                    // Same language: the chapter keeps its title.
                    f.out_title = f.title.clone();
                    settle_names(&mut f);
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
                    save(&conn, &f.req, &f.out_title, &f.out, llm.model())?;
                    PartCache::clear(&conn, &f.req)?;
                    super::wiki::keep_found(
                        &conn,
                        f.req.chapter_id,
                        super::wiki::WIKI_LANG,
                        &f.found,
                    )?;
                    super::cast::keep(
                        &conn,
                        f.req.novel_id,
                        f.req.chapter_id,
                        &f.req.lang,
                        &f.found,
                    )?;
                    Ok(f)
                })();
                step(&failed, r)
            }
        });
    }
    {
        let (llm, failed, background) = (llm.clone(), failed.clone(), hooks.learn_in_background);
        let later = hooks.learn_found_later;
        g.add_node("learn", move |f: Flow, _: &Config| {
            let (llm, failed) = (llm.clone(), failed.clone());
            let queued = later && super::wiki::has_found(&f.found);
            if !background && !queued {
                enter(Stage::Learn);
            }
            async move {
                if queued {
                    return Ok(f);
                }
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
        super::wiki::WIKI_LANG,
        &f.paragraphs.join("\n\n"),
        &f.out.join("\n\n"),
        &f.found,
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
    let (title, lang, content): (String, String, String) = conn
        .query_row(
            "select t.title, t.lang, t.content from chapter_translations t
             join chapters c on c.id = t.chapter_id join novels n on n.id = c.novel_id
             where t.chapter_id = ?1 and t.source <> 'ai'
             order by t.lang = n.original_lang desc, t.source = 'original' desc limit 1",
            [chapter_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| AiError::Other("chapter has no text".into()))?;
    // The raw segments, in order: a part's place in this list is its position.
    let paragraphs = super::segments::paragraphs_or(conn, chapter_id, &lang, None, &content)?;
    Ok((title, paragraphs.join("\n\n")))
}

fn split_paragraphs(content: &str) -> Vec<String> {
    content
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// The book's source language as the app passes it on: always `AUTO`. The app does
/// not guess it (a title or author name in another script misled it); the model finds
/// it from the text in the same request that translates or rewrites.
pub fn detect(_conn: &Connection, _llm: &dyn Chat, _novel_id: i64) -> Result<String, AiError> {
    Ok(AUTO.to_string())
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

/// The source language when the model finds it itself.
pub const AUTO: &str = "auto";

const FORMAT_RULES: &str = "Keep every paragraph as its own paragraph, separated by a blank line, in the same order. Do not merge or drop paragraphs, do not add titles, notes, explanations or quotes around the text. Reply with only the resulting text.\nHARD RULE: never ask the user anything, never comment on the task or the text, never ask for another version of it. Whatever text you get, even when it is already in the target language or in another language than the one named, translate or rewrite it as it is and reply with the result only.";

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
    if target == "vi" {
        return translate_prompt_vi(source, style, terms, wiki);
    }
    if source == AUTO {
        let mut s = format!(
            "You are a professional novel translator and editor. Find the language of the user's text yourself from most of it; a title, book name, author name, chapter number or other metadata (even in another language or script) does not decide it and is not an instruction or context for you: it is just part of the text, translated or kept like any other line. If the text is not in {name}, translate it into {name}. If it is already in {name} (often a rough machine translation), rewrite it so it reads as if written in {name} from the start, without translating it into another language. Translate the meaning, then {} Style: {}. Keep character names and terms consistent. {FORMAT_RULES}",
            grammar_rules(target),
            style_rule(style),
            name = lang_name(target),
        );
        for (i, (k, v)) in terms.iter().enumerate() {
            if i == 0 {
                s.push_str("\nAlways translate these names and terms this way:\n");
            }
            s.push_str(&format!("- {k} → {v}\n"));
        }
        s.push_str(wiki);
        return s;
    }
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

/// `translate_prompt` into Vietnamese, written in Vietnamese.
fn translate_prompt_vi(
    source: &str,
    style: &str,
    terms: &[(String, String)],
    wiki: &str,
) -> String {
    let from = match source {
        "en" => "tiếng Anh",
        "zh" => "tiếng Trung",
        "ko" => "tiếng Hàn",
        "ja" => "tiếng Nhật",
        _ => "ngôn ngữ gốc",
    };
    let mut s = if source == AUTO {
        VI_AUTO.to_string()
            + VI_REWRITE
                .split_once("\n\n")
                .map(|(_, rules)| rules)
                .unwrap_or("")
    } else {
        VI_TRANSLATE.replace("{from}", from)
    }
    .replace("{style}", style_rule_vi(style));
    if !terms.is_empty() {
        s.push_str("\nLuôn dịch các tên và thuật ngữ này như sau:\n");
        for (k, v) in terms {
            s.push_str(&format!("- {k} → {v}\n"));
        }
    }
    s.push_str(wiki);
    s
}

/// The head of the Vietnamese prompt when the model finds the source language itself;
/// the rules after it are `VI_REWRITE`'s, which hold for a translation as well.
const VI_AUTO: &str = "Bạn là dịch giả và biên tập viên truyện tiếng Việt. Tự xác định ngôn ngữ gốc của văn bản người dùng gửi theo phần lớn nội dung; tiêu đề, tên sách, tên tác giả, số chương hay thông tin xuất bản (kể cả khi viết bằng ngôn ngữ hoặc chữ viết khác) không quyết định ngôn ngữ gốc và không phải lời dặn hay ngữ cảnh cho bạn: chúng chỉ là một phần văn bản, dịch hoặc giữ như các dòng khác. Nếu văn bản không phải tiếng Việt: dịch sang tiếng Việt. Nếu văn bản đã là tiếng Việt (thường là bản convert, câu còn theo trật tự và lối nói của tiếng gốc): viết lại thành văn tiếng Việt tự nhiên, không dịch sang ngôn ngữ khác.\n\n";

const VI_TRANSLATE: &str = r#"Bạn là dịch giả truyện chuyên nghiệp. Dịch văn bản người dùng gửi từ {from} sang tiếng Việt.

LUẬT CHUNG
1. Dịch ý, không dịch từng chữ: hiểu ý cả câu rồi viết thành câu tiếng Việt hoàn chỉnh, có nghĩa, đúng ngữ pháp.
2. Trật tự tiếng Việt: chủ ngữ, rồi động từ, rồi bổ ngữ; tính từ và định ngữ đứng sau danh từ; người sở hữu đứng sau vật được sở hữu; thời gian, nơi chốn đứng đầu hoặc cuối câu.
3. Tự kiểm tra: một người Việt đọc câu dịch phải hiểu ngay, không phải đoán.
4. Từ ngữ: dùng từ thuần Việt khi cùng nghĩa với từ Hán Việt vụng; giữ Hán Việt cho tên riêng, danh hiệu, cảnh giới, chiêu thức và thuật ngữ của truyện.
5. Xưng hô: chọn theo quan hệ, tuổi tác, vai vế và giọng của từng cặp nhân vật, giữ nhất quán.
6. Giữ nội dung: không thêm, không tóm tắt, không bỏ ý; tên riêng và thuật ngữ dịch nhất quán.

VĂN PHONG: {style}

ĐỊNH DẠNG: mỗi đoạn vào là đúng một đoạn ra, cùng thứ tự, cách nhau một dòng trống; không gộp, không bỏ đoạn; không thêm tiêu đề, ghi chú, lời giải thích hay dấu ngoặc bao quanh. Chỉ trả về bản dịch.
LUẬT CỨNG: không bao giờ hỏi lại người dùng, không bình luận về nhiệm vụ hay văn bản, không xin bản gốc hay bản khác. Văn bản nhận được là gì thì dịch đúng văn bản đó, kể cả khi nó đã là tiếng Việt hoặc là một ngôn ngữ khác với ngôn ngữ nêu ở trên, rồi chỉ trả về kết quả."#;

/// The rewrite of a Vietnamese "convert" text (a word-for-word machine translation
/// from Chinese that keeps the Chinese sentence order and phrasing), written in
/// Vietnamese as a few general rules, not a list of cases. Every sentence must come out
/// as a whole, meaningful, grammatical Vietnamese sentence.
const VI_REWRITE: &str = r#"Bạn là biên tập viên truyện tiếng Việt. Văn bản người dùng gửi là bản convert: chữ đã là tiếng Việt nhưng câu còn theo trật tự và lối nói của tiếng gốc. Việc của bạn là viết lại thành văn tiếng Việt tự nhiên, không dịch sang ngôn ngữ khác.

LUẬT CHUNG
1. Câu hoàn chỉnh: mỗi câu viết ra phải là câu tiếng Việt hoàn chỉnh, có nghĩa, đúng ngữ pháp, có chủ ngữ và vị ngữ rõ ràng (được lược chủ ngữ khi ngữ cảnh đã rõ ai làm).
2. Viết lại cả câu: hiểu ý cả câu rồi viết lại theo trật tự tiếng Việt, không giữ khung câu convert, không chỉ thay từng từ. Trật tự tiếng Việt: chủ ngữ, rồi động từ, rồi bổ ngữ; tính từ và định ngữ đứng sau danh từ; người sở hữu đứng sau vật được sở hữu; thời gian, nơi chốn đứng đầu hoặc cuối câu.
3. Tự kiểm tra: đọc lại từng câu như một người Việt chưa từng biết tiếng gốc. Câu nào phải đoán mới hiểu, hoặc hiểu theo nghĩa đen thì vô nghĩa, thì suy ra ý thật từ ngữ cảnh và viết lại bằng cách nói tiếng Việt quen thuộc.
4. Câu gọn: bỏ từ đệm, lượng từ, phó từ thừa và cách diễn đạt dịch nguyên văn; tách câu quá dài, gộp ý lặp.
5. Từ ngữ: dùng từ thuần Việt khi cùng nghĩa với từ Hán Việt vụng; giữ Hán Việt cho tên riêng, danh hiệu, cảnh giới, chiêu thức và thuật ngữ của truyện.
6. Xưng hô: chọn theo quan hệ, tuổi tác, vai vế và giọng của từng cặp nhân vật, giữ nhất quán suốt truyện.
7. Giữ nội dung: không thêm tình tiết, cảm xúc hay chi tiết mới; không tóm tắt; không bỏ ý.
8. Giữ tên riêng đúng như danh sách tên riêng bên dưới (nếu có).

VĂN PHONG: {style}

ĐỊNH DẠNG: mỗi đoạn vào là đúng một đoạn ra, cùng thứ tự, cách nhau một dòng trống; không gộp, không tách, không bỏ đoạn; không thêm tiêu đề, ghi chú, lời giải thích hay dấu ngoặc bao quanh. Chỉ trả về văn bản đã viết lại.
LUẬT CỨNG: không bao giờ hỏi lại người dùng, không bình luận về nhiệm vụ hay văn bản, không xin bản gốc hay bản khác. Văn bản nhận được là gì thì viết lại đúng văn bản đó, kể cả khi nó là một ngôn ngữ khác, rồi chỉ trả về kết quả."#;

/// `style_rule` in Vietnamese, for `VI_REWRITE`.
fn style_rule_vi(style: &str) -> &'static str {
    match style {
        "literal" => "Sát nghĩa: bám sát từng ý của bản gốc, không thêm thắt, chỉ sửa trật tự câu và từ ngữ cho đúng tiếng Việt.",
        "literary" => "Văn chương: giàu hình ảnh, chọn từ tinh tế, nhịp câu theo không khí từng cảnh; vẫn không thêm chi tiết mới.",
        "casual" => "Đời thường: giản dị, hiện đại, câu ngắn, từ thông dụng; hạn chế Hán Việt trừ tên riêng và thuật ngữ.",
        _ => "Tự nhiên: trôi chảy, dễ đọc như truyện mạng tiếng Việt viết tốt; câu gọn, nhịp đều, thoại đúng giọng từng nhân vật.",
    }
}

fn rewrite_prompt(lang: &str, style: &str, wiki: &str) -> String {
    if lang == "vi" {
        return VI_REWRITE.replace("{style}", style_rule_vi(style)) + wiki;
    }
    let s = format!(
        "You are a professional {name} novel editor. The user's text is already in {name} but may be a rough machine translation; do not translate it into another language. Rewrite it so it reads as if written in {name} from the start. {} Fix spelling, punctuation and wrong word choices. Style: {}. Keep the meaning, events, dialogue and names; do not summarize, shorten or add anything. {FORMAT_RULES}",
        grammar_rules(lang),
        style_rule(style),
        name = lang_name(lang),
    );
    s + wiki
}

/// The tools a chapter's parts are asked with.
fn part_tools<'a>(
    f: &Flow,
    lookup: &'a (dyn Fn(&str) -> String + Sync),
    keep: &'a (dyn Fn(&str) + Sync),
) -> PartTools<'a> {
    PartTools {
        // The wiki is kept in Vietnamese whatever the translation's language.
        defs: super::wiki::part_tools(super::wiki::WIKI_LANG),
        note: super::wiki::tool_prompt(super::wiki::WIKI_LANG, &f.genre),
        lookup,
        keep,
        off: AtomicBool::new(false),
    }
}

/// The names part of a part's prompt: the wiki looked up again just before the part is
/// sent, so it has the names the parts before it put in, with what the chapter's recall
/// found.
fn part_recall(f: &Flow, part: &str) -> String {
    let fresh = super::db::open(std::path::Path::new(&f.db))
        .and_then(|conn| super::wiki::recall(&conn, None, f.req.novel_id, part, &f.req.lang));
    match fresh {
        Ok(r) => r.with(&f.recall).prompt(part),
        Err(e) => {
            eprintln!("ai part recall: {e}");
            f.recall.prompt(part)
        }
    }
}

/// Puts a part's wiki facts in the book's wiki right away (`wiki::learn_part`).
fn keep_part(f: &Flow, facts: &str) {
    let r = super::db::open(std::path::Path::new(&f.db))
        .map_err(AiError::from)
        .and_then(|conn| {
            super::wiki::learn_part(&conn, f.req.novel_id, f.req.chapter_id, &f.req.lang, facts)
        });
    if let Err(e) = r {
        eprintln!("ai wiki part: {e}");
    }
}

/// The chapter with the names its parts wrote differently from the wiki written the
/// wiki's way (`wiki::settled_names`). Only for a Vietnamese translation, the wiki's
/// language.
fn settle_names(f: &mut Flow) {
    if f.req.lang != super::wiki::WIKI_LANG || f.found.is_empty() {
        return;
    }
    let pairs = super::db::open(std::path::Path::new(&f.db))
        .and_then(|conn| super::wiki::settled_names(&conn, f.req.novel_id, &f.found));
    let pairs = match pairs {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ai settle names: {e}");
            return;
        }
    };
    for (from, to) in &pairs {
        for p in f.out.iter_mut() {
            *p = super::wiki::replace_name(p, from, to);
        }
        f.out_title = super::wiki::replace_name(&f.out_title, from, to);
    }
}

/// `tra_ten` for a chapter: what the book's wiki has under `name`, as JSON.
fn lookup_name(f: &Flow, llm: &dyn Chat, name: &str) -> String {
    let found = super::db::open(std::path::Path::new(&f.db))
        .map_err(|e| e.to_string())
        .and_then(|conn| {
            super::wiki::lookup(&conn, Some(llm), f.req.novel_id, &f.req.lang, name)
                .map_err(|e| e.to_string())
        });
    match found {
        Ok(v) => v.to_string(),
        Err(e) => format!("{{\"error\": {}}}", serde_json::Value::String(e)),
    }
}

/// How characters address each other and are referred to in a genre, for a
/// translation into `lang`.
fn genre_rules(genre: &str, lang: &str) -> &'static str {
    match (genre, lang) {
        ("wuxia", "vi") => "Thể loại: kiếm hiệp. Xưng hô trong thoại: ta / ngươi giữa người ngang hàng hoặc với người dưới; tại hạ / các hạ khi lịch sự với người lạ; huynh / đệ / tỷ / muội giữa anh em kết nghĩa, đồng môn; sư phụ / đồ nhi giữa thầy và trò; lão phu, lão nạp cho người già, nhà sư; bổn cô nương cho thiếu nữ kiêu kỳ. Trong lời kể: hắn hoặc y cho nam, nàng cho thiếu nữ, lão cho người già. Giữ thuật ngữ võ học bằng Hán Việt (nội công, khinh công, chưởng môn); giọng văn gọn, hào sảng.",
        ("xianxia", "vi") => "Thể loại: tiên hiệp. Xưng hô trong thoại: mặc định ta / ngươi; bản tọa, bổn tọa hoặc lão phu khi bậc tiền bối mạnh tự xưng; tiền bối / vãn bối giữa người trên và người dưới; đạo hữu giữa tu sĩ ngang hàng; sư tôn / sư phụ và đệ tử giữa thầy và trò; sư huynh / sư đệ / sư tỷ / sư muội trong cùng tông môn. Trong lời kể: hắn cho nam, nàng cho nữ. Giữ mọi cảnh giới, công pháp, pháp bảo, danh hiệu tông môn bằng Hán Việt (Luyện Khí, Trúc Cơ, Kim Đan, Nguyên Anh, linh thạch, pháp bảo).",
        ("supernatural", "vi") => "Thể loại: linh dị. Thường là bối cảnh hiện đại: tôi / anh / em / cậu / chị trong thoại theo tuổi và độ thân; mày / tao chỉ giữa bạn bè thô lỗ hoặc khi tức giận. Trong lời kể: anh ta / cô ta, hắn cho kẻ đáng sợ, nó cho ma quỷ, xác chết hay đồ vật. Giọng văn căng thẳng, rờn rợn, câu ngắn ở đoạn đáng sợ; thuật ngữ Đạo giáo, ma quỷ giữ Hán Việt (âm khí, lệ quỷ, phù chú).",
        ("romance", "vi") => "Thể loại: tình yêu - lãng mạn. Bối cảnh hiện đại: người yêu xưng anh / em, người chưa thân xưng tôi / anh / cô, xưng hô ấm dần theo mối quan hệ; trong lời kể dùng anh / cô (anh ấy / cô ấy), không dùng hắn / nàng. Bối cảnh cổ đại: chàng / nàng, ta / chàng, thiếp / phu quân giữa vợ chồng. Cảm xúc và suy nghĩ nội tâm viết mềm mại, tự nhiên, không cứng.",
        ("politics", "vi") => "Thể loại: chính trị. Chốn cung đình: trẫm / khanh giữa vua và quan, thần / bệ hạ khi quan nói với vua, bản cung / bổn cung cho hoàng hậu, phi tần, điện hạ cho hoàng tử, nô tài / nô tì cho người hầu. Bối cảnh hiện đại: tôi / ông / bà / ngài / đồng chí theo cấp bậc và mức trang trọng; lời kể dùng ông ta / bà ta. Giữ chức danh, quan hàm đúng dạng Hán Việt hoặc chính thức; giọng văn trang trọng, chừng mực.",
        ("nonfiction", "vi") => "Thể loại: tài liệu thực tế. Giữ nguyên mọi danh từ riêng (người, nơi chốn, tổ chức, công ty, sản phẩm, sự kiện, tác phẩm) đúng tên gốc tiếng Anh, không dịch, không phiên âm sang tiếng Việt; nếu tên gốc không phải tiếng Anh thì dùng dạng tiếng Anh thông dụng của tên đó. Gọi người một cách trung tính theo tuổi và vai trò: ông / bà / anh / chị + tên, tác giả xưng tôi / chúng tôi; giọng văn rõ ràng, khách quan, không bay bướm.",
        ("wuxia", _) => "Genre: wuxia. Use martial-world forms of address in English: Brother, Sister, Master, Senior, Junior, \"this old man\" for an elder speaking of himself; keep a terse, knightly tone.",
        ("xianxia", _) => "Genre: xianxia. Use cultivation forms of address in English: Senior, Junior, Fellow Daoist, Master, Senior Brother, Junior Sister, \"this venerable one\" for a proud elder; keep cultivation realms and techniques as consistent English names.",
        ("supernatural", _) => "Genre: supernatural horror. Modern, plain forms of address; keep the tone tense and eerie, with short sentences in frightening moments.",
        ("romance", _) => "Genre: romance. Warm, natural dialogue whose tone grows with the relationship; in a historical setting use my lord / my lady and similar.",
        ("politics", _) => "Genre: politics. Formal forms of address by rank: Your Majesty, Your Highness, this official, Minister; keep titles and offices consistent.",
        ("nonfiction", _) => "Genre: non-fiction. Keep every proper noun (people, places, organisations, companies, products, events, works) exactly as its original English name; when the original is not in English, use the name's common English form. A clear, factual tone.",
        _ => "",
    }
}

/// What a book adds to every prompt: its genre's rules and the user's own instructions.
fn book_rules(genre: &str, lang: &str, instructions: &str) -> String {
    let mut s = String::new();
    let rules = genre_rules(genre, lang);
    if !rules.is_empty() {
        s.push('\n');
        s.push_str(rules);
    }
    let own = instructions.trim();
    if !own.is_empty() {
        s.push_str(if lang == "vi" {
            "\nYêu cầu riêng của người đọc cho truyện này (làm theo, trừ khi yêu cầu gộp, bỏ hay thêm đoạn):\n<<<\n"
        } else {
            "\nThe reader's own instructions for this book (follow them, unless they ask to merge, drop or add paragraphs):\n<<<\n"
        });
        s.push_str(own);
        s.push_str("\n>>>");
    }
    s
}

/// How a book is translated besides language and style (`ai_book_prefs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    pub genre: String,
    pub instructions: String,
}

/// Instructions the user may write for a book, in characters.
const MAX_INSTRUCTIONS: usize = 2000;

pub fn prefs(conn: &Connection, novel_id: i64) -> rusqlite::Result<Prefs> {
    Ok(conn
        .query_row(
            "select genre, instructions from ai_book_prefs where novel_id = ?1",
            [novel_id],
            |r| {
                Ok(Prefs {
                    genre: r.get(0)?,
                    instructions: r.get(1)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

/// Keeps a book's genre (an unknown one is none) and instructions (cut to
/// `MAX_INSTRUCTIONS`).
pub fn set_prefs(conn: &Connection, novel_id: i64, p: &Prefs) -> rusqlite::Result<()> {
    let genre = if GENRES.contains(&p.genre.as_str()) {
        p.genre.as_str()
    } else {
        ""
    };
    let instructions: String = p
        .instructions
        .trim()
        .chars()
        .take(MAX_INSTRUCTIONS)
        .collect();
    conn.execute(
        "insert into ai_book_prefs (novel_id, genre, instructions) values (?1, ?2, ?3)
         on conflict (novel_id) do update set genre = excluded.genre, instructions = excluded.instructions",
        params![novel_id, genre, instructions],
    )?;
    Ok(())
}

fn title_prompt(target: &str) -> String {
    if target == "vi" {
        return "Dịch tiêu đề chương truyện này sang tiếng Việt. Chỉ trả về tiêu đề đã dịch, trên một dòng.".into();
    }
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

/// A chapter back from the LLM: its paragraphs, the title's reply, the wiki facts.
type Parts = (Vec<String>, Option<String>, Vec<String>);

/// Sends the text paragraphs in parts of about `CHUNK_CHARS`, `PARALLEL` at a time,
/// each with the system prompt `system` makes for it,
/// and puts the replies back in order; pictures stay where they are. `title` (a
/// system prompt and the title) is asked alongside the parts. Whatever a part's reply
/// has after `WIKI_MARK` comes back on its own, one per part. The first error stops
/// what has not started yet and is returned; so does `stop` (as `AiError::Stopped`).
/// Replies found in `cache` are not asked again, and each new one goes in it.
///
/// Where two parts meet the text must read on as one, so each part after the first
/// is sent with the end of the part before it (as context, not to translate); a
/// reply cut off by the model's length limit is asked again in two halves instead
/// of losing its end; and a reply that put its paragraphs on single lines is split
/// on those lines, so the parts do not run into each other.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn run_chunks(
    llm: &dyn Chat,
    system: &(dyn Fn(&str) -> String + Sync),
    paragraphs: &[String],
    title: Option<(String, String)>,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
    cache: Option<&PartCache>,
    stop: Option<&AtomicBool>,
) -> Result<Parts, AiError> {
    run_chunks_with(
        llm, system, "", None, paragraphs, title, progress, cache, stop,
    )
}

/// `run_chunks` where each part is first asked through `tools` (when given and the
/// model takes them), and otherwise as text with `extract` added to its prompt.
#[allow(clippy::too_many_arguments)]
fn run_chunks_with(
    llm: &dyn Chat,
    system: &(dyn Fn(&str) -> String + Sync),
    extract: &str,
    tools: Option<&PartTools>,
    paragraphs: &[String],
    title: Option<(String, String)>,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
    cache: Option<&PartCache>,
    stop: Option<&AtomicBool>,
) -> Result<Parts, AiError> {
    // Whole blocks are gathered up to about `CHUNK_CHARS`: a part is the blocks
    // `start..start + len` of the chapter, and a picture ends the part before it.
    let mut pieces: Vec<(usize, Vec<&str>)> = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    let mut size = 0;
    for (i, p) in paragraphs.iter().enumerate() {
        let picture = p.starts_with(IMAGE_MARK);
        let len = p.chars().count();
        if !chunk.is_empty() && (picture || size + len > CHUNK_CHARS) {
            pieces.push((i - chunk.len(), std::mem::take(&mut chunk)));
            size = 0;
        }
        if !picture {
            size += len;
            chunk.push(p);
        }
    }
    if !chunk.is_empty() {
        pieces.push((paragraphs.len() - chunk.len(), chunk));
    }

    // The requests: each text part with the end of the one before it, then the title.
    let mut before: Option<&str> = None;
    let mut asks: Vec<Ask> = Vec::new();
    for (start, paras) in &pieces {
        asks.push(Ask::Part {
            start: *start,
            paras: paras.clone(),
            context: before.map(context_of),
        });
        before = paras.last().copied();
    }
    let parts = asks.len();
    if let Some((prompt, text)) = &title {
        asks.push(Ask::Title(prompt, text));
    }
    let total = asks.len();
    let replies: Vec<Mutex<Option<String>>> = (0..total).map(|_| Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let failed: Mutex<Option<AiError>> = Mutex::new(None);
    // A part's key in the cache; the title's is told apart from the text parts'.
    let key = |i: usize, text: &str| {
        let kind = if i < parts { "p" } else { "t" };
        format!("{kind}:{:016x}:{}", fnv1a(text), text.len())
    };
    // The parts kept from an earlier try.
    let mut kept = 0;
    if let Some(c) = cache {
        for (i, ask) in asks.iter().enumerate() {
            if let Some(reply) = c.get(&key(i, &ask.text()))? {
                if let Some((start, blocks)) = ask.blocks(&reply) {
                    c.place(start, &blocks)?;
                }
                *replies[i].lock().unwrap() = Some(reply);
                kept += 1;
            }
        }
    }
    done.store(kept, Ordering::SeqCst);
    if let Some(p) = progress {
        p(kept, total);
    }
    std::thread::scope(|s| {
        for _ in 0..PARALLEL.min(total) {
            s.spawn(|| loop {
                if failed.lock().unwrap().is_some() {
                    return;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(ask) = asks.get(i) else {
                    return;
                };
                if replies[i].lock().unwrap().is_some() {
                    continue;
                }
                if stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
                    failed.lock().unwrap().get_or_insert(AiError::Stopped);
                    return;
                }
                let reply = match ask {
                    Ask::Part { paras, context, .. } => {
                        let prompt = |text: &str| match context {
                            Some(c) => format!("{}{}", system(text), context_prompt(c)),
                            None => system(text),
                        };
                        let by_tools = match tools {
                            Some(t) if !t.off.load(Ordering::SeqCst) => {
                                ask_part_tools(llm, &prompt, paras, t)
                            }
                            _ => Ok(None),
                        };
                        let with_extract = |text: &str| format!("{}{extract}", prompt(text));
                        // Facts that came through `ghi_wiki` are in the wiki already.
                        let mut kept = false;
                        let reply = match by_tools {
                            Ok(Some(reply)) => {
                                kept = true;
                                Ok(reply)
                            }
                            Ok(None) => ask_part(llm, &with_extract, paras),
                            Err(e) => Err(e),
                        };
                        // A reply that talks to the user (asks back, comments, asks
                        // for another text) instead of giving the text is never kept:
                        // asked once more as text with the rule in front, then failed.
                        match reply {
                            Ok(r) if chatter(split_reply(&r).0, paras) => {
                                kept = false;
                                let firm =
                                    |text: &str| format!("{}{NO_CHATTER}", with_extract(text));
                                match ask_part(llm, &firm, paras) {
                                    Ok(r) if chatter(split_reply(&r).0, paras) => {
                                        Err(AiError::Other(CHATTER_ERROR.into()))
                                    }
                                    other => other,
                                }
                            }
                            other => other,
                        }
                        .inspect(|r| {
                            // A text answer's facts go in the wiki as soon as it is in.
                            if let (false, Some(t), (_, Some(facts))) =
                                (kept, tools, split_reply(r))
                            {
                                (t.keep)(facts);
                            }
                        })
                    }
                    Ask::Title(prompt, text) => AiError::keep_cut(llm.chat(prompt, text)),
                };
                match reply {
                    Ok(reply) => {
                        if let Some(c) = cache {
                            // Not kept is only slower next time: the part is asked again.
                            if let Err(e) = c.put(&key(i, &ask.text()), &reply) {
                                eprintln!("ai part cache: {e}");
                            }
                            // The part's blocks go in the translation where they belong.
                            if let Some((start, blocks)) = ask.blocks(&reply) {
                                if let Err(e) = c.place(start, &blocks) {
                                    eprintln!("ai draft: {e}");
                                }
                            }
                        }
                        *replies[i].lock().unwrap() = Some(reply);
                        let n = done.fetch_add(1, Ordering::SeqCst) + 1;
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
    // Put back by place: pictures stay where they are, each part's blocks go where
    // the blocks it was made from are.
    let mut out: Vec<String> = paragraphs
        .iter()
        .map(|p| {
            if p.starts_with(IMAGE_MARK) {
                p.clone()
            } else {
                String::new()
            }
        })
        .collect();
    let mut found = Vec::new();
    for (start, paras) in &pieces {
        let reply = replies.next().unwrap_or_default();
        let (text, facts) = split_reply(&reply);
        if let Some(facts) = facts {
            found.push(facts.to_string());
        }
        for (k, block) in fit(reply_paragraphs(&clean(text), paras.len()), paras.len())
            .into_iter()
            .enumerate()
        {
            out[start + k] = block;
        }
    }
    let title = (total > parts).then(|| replies.next().unwrap_or_default());
    Ok((out, title, found))
}

/// One request of `run_chunks`.
enum Ask<'a> {
    /// Paragraphs of the chapter (blocks `start..` of it), with the end of the part
    /// before them.
    Part {
        start: usize,
        paras: Vec<&'a str>,
        context: Option<String>,
    },
    /// The title's system prompt and the title.
    Title(&'a str, &'a str),
}

impl Ask<'_> {
    /// The text it sends, which keys its reply in the part cache.
    fn text(&self) -> String {
        match self {
            Ask::Part { paras, .. } => paras.join("\n\n"),
            Ask::Title(_, text) => text.to_string(),
        }
    }

    /// Where a part's `reply` goes in the chapter: its first block and one paragraph
    /// per block sent. None for the title.
    fn blocks(&self, reply: &str) -> Option<(usize, Vec<String>)> {
        match self {
            Ask::Part { start, paras, .. } => {
                let text = clean(split_reply(reply).0);
                Some((
                    *start,
                    fit(reply_paragraphs(&text, paras.len()), paras.len()),
                ))
            }
            Ask::Title(..) => None,
        }
    }
}

/// A reply's paragraphs as exactly `n`, one per block sent, so each lands on the block
/// it translates: paragraphs past the last block join it (as lines), blocks the reply
/// has nothing for stay empty.
fn fit(mut paras: Vec<String>, n: usize) -> Vec<String> {
    if paras.len() > n && n > 0 {
        let rest = paras.split_off(n - 1);
        paras.push(rest.join("\n"));
    }
    paras.resize(n, String::new());
    paras
}

/// Characters of the part before that go with a part as context.
const CONTEXT_CHARS: usize = 400;
/// A paragraph shorter than this is not cut in two when its reply is cut off.
const MIN_SPLIT_CHARS: usize = 200;

/// The end of the paragraph a part follows, at most `CONTEXT_CHARS`.
fn context_of(paragraph: &str) -> String {
    let n = paragraph.chars().count();
    paragraph
        .chars()
        .skip(n.saturating_sub(CONTEXT_CHARS))
        .collect()
}

fn context_prompt(context: &str) -> String {
    format!(
        "\n\nThe user's text continues right after this passage of the same chapter. It is given only so the start of the text follows on from it (same names, pronouns and tone); do not translate, rewrite or repeat it:\n<<<\n{context}\n>>>"
    )
}

/// The reply for `paras`. A reply cut off by the model's length limit is asked again
/// in two halves (a long paragraph is cut between sentences), and the halves' replies
/// are put back together.
/// The tools a translation or rewrite offers each part (wiki.rs `part_tools`).
pub(crate) struct PartTools<'a> {
    defs: serde_json::Value,
    /// Added to the part's prompt: how to answer through the tools, the wiki's rules.
    note: String,
    /// The answer to `tra_ten` for a name.
    lookup: &'a (dyn Fn(&str) -> String + Sync),
    /// Puts a part's wiki facts in the wiki as soon as they come (`ghi_wiki`).
    keep: &'a (dyn Fn(&str) + Sync),
    /// The API or model refused tools: the remaining parts are asked as text.
    off: AtomicBool,
}

/// Lookups a part may make before it must hand in its text.
const MAX_LOOKUPS: usize = 3;

/// A part asked through `tools`: the text from `nop_ban_viet`, then `WIKI_MARK` and
/// the facts from `ghi_wiki` (the reply a text answer gives, so the cache and the
/// wiki read both alike). `tra_ten` is answered and the part asked again, up to
/// `MAX_LOOKUPS` times. None when the part has to be asked as text instead: the model
/// answered without the text tool, its arguments were not JSON, or the reply was cut
/// off (the text way splits the part). A refusal of tools turns them off for the rest.
fn ask_part_tools(
    llm: &dyn Chat,
    prompt: &dyn Fn(&str) -> String,
    paras: &[&str],
    tools: &PartTools,
) -> Result<Option<String>, AiError> {
    use serde_json::{json, Value};
    let text = paras.join("\n\n");
    let mut messages = vec![
        json!({ "role": "system", "content": format!("{}{}", prompt(&text), tools.note) }),
        json!({ "role": "user", "content": text }),
    ];
    let mut facts: Vec<String> = Vec::new();
    for round in 0..=MAX_LOOKUPS {
        let choice = if round < MAX_LOOKUPS {
            json!("auto")
        } else {
            json!({ "type": "function", "function": { "name": super::wiki::PART_TEXT } })
        };
        let msg = match llm.chat_tools_with(&messages, &tools.defs, &choice) {
            Ok(m) => m,
            Err(AiError::Truncated(_)) => return Ok(None),
            Err(e) if super::wiki::tools_refused(&e) => {
                tools.off.store(true, Ordering::SeqCst);
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let calls: Vec<Value> = msg
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if calls.is_empty() {
            // Answered in words: that is the text, as a text answer would be.
            let content = msg.get("content").and_then(Value::as_str).unwrap_or("");
            return Ok((!content.trim().is_empty()).then(|| content.to_string()));
        }
        let mut written: Option<String> = None;
        let mut answers: Vec<Value> = Vec::new();
        for call in &calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("");
            let f = call.get("function");
            let name = f
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let args: Option<Value> = f
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .and_then(|a| serde_json::from_str(a).ok());
            let answer = match name {
                n if n == super::wiki::PART_TEXT => {
                    let Some(doan) = args
                        .as_ref()
                        .and_then(|a| a.get("doan"))
                        .and_then(Value::as_array)
                    else {
                        return Ok(None);
                    };
                    let blocks: Vec<&str> = doan.iter().filter_map(Value::as_str).collect();
                    written = Some(blocks.join("\n\n"));
                    "ok".to_string()
                }
                n if n == super::wiki::PART_FACTS => {
                    if let Some(a) = args.filter(Value::is_object) {
                        // Into the wiki now: the parts sent after this one find it.
                        (tools.keep)(&a.to_string());
                        facts.push(a.to_string());
                    }
                    "ok".to_string()
                }
                n if n == super::wiki::PART_LOOKUP => {
                    let q = args
                        .as_ref()
                        .and_then(|a| a.get("ten"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    (tools.lookup)(q)
                }
                _ => "unknown tool".to_string(),
            };
            answers.push(json!({ "role": "tool", "tool_call_id": id, "content": answer }));
        }
        if let Some(written) = written {
            let mut reply = written;
            if !facts.is_empty() {
                reply.push_str(&format!("\n{WIKI_MARK}\n{}", facts.join("\n")));
            }
            return Ok(Some(reply));
        }
        // Looked up (or only recorded facts): answer every call and ask again.
        messages.push(msg);
        messages.extend(answers);
    }
    Ok(None)
}

fn ask_part(
    llm: &dyn Chat,
    prompt: &dyn Fn(&str) -> String,
    paras: &[&str],
) -> Result<String, AiError> {
    let text = paras.join("\n\n");
    match llm.chat(&prompt(&text), &text) {
        Err(AiError::Truncated(_)) if paras.len() > 1 => {
            let mid = half_by_size(paras);
            let a = ask_part(llm, prompt, &paras[..mid])?;
            let b = ask_part(llm, prompt, &paras[mid..])?;
            Ok(join_replies(&a, &b, "\n\n"))
        }
        Err(AiError::Truncated(partial)) => match split_sentences_half(paras[0]) {
            Some((x, y)) => {
                let a = ask_part(llm, prompt, &[x])?;
                let b = ask_part(llm, prompt, &[y])?;
                Ok(join_replies(&a, &b, " "))
            }
            None => Err(AiError::Truncated(partial)),
        },
        r => r,
    }
}

/// Where to cut `paras` so both halves hold about as much text (1..len).
fn half_by_size(paras: &[&str]) -> usize {
    let total: usize = paras.iter().map(|p| p.chars().count()).sum();
    let mut size = 0;
    for (i, p) in paras.iter().enumerate() {
        size += p.chars().count();
        if size * 2 >= total {
            return (i + 1).clamp(1, paras.len() - 1);
        }
    }
    paras.len() / 2
}

/// A long paragraph cut in two at the end of the sentence nearest its middle, or
/// at a space when it has no sentence end. None when it is too short to cut.
fn split_sentences_half(p: &str) -> Option<(&str, &str)> {
    let n = p.chars().count();
    if n < MIN_SPLIT_CHARS {
        return None;
    }
    let mid = p.char_indices().nth(n / 2).map(|(i, _)| i)?;
    let nearest = |is_cut: &dyn Fn(char) -> bool| {
        p.char_indices()
            .filter(|&(i, c)| is_cut(c) && i > 0 && i + c.len_utf8() < p.len())
            .map(|(i, c)| i + c.len_utf8())
            .min_by_key(|&i| i.abs_diff(mid))
    };
    let cut = nearest(&|c| matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '…'))
        .or_else(|| nearest(&char::is_whitespace))
        .unwrap_or(mid);
    let (a, b) = p.split_at(cut);
    let (a, b) = (a.trim(), b.trim());
    (!a.is_empty() && !b.is_empty()).then_some((a, b))
}

/// The text of a reply and the wiki facts after `WIKI_MARK`, if any.
fn split_reply(reply: &str) -> (&str, Option<&str>) {
    match reply.split_once(WIKI_MARK) {
        Some((text, facts)) => (text, Some(facts)),
        None => (reply, None),
    }
}

/// Two replies as one: the texts joined by `sep`, then the facts of both.
fn join_replies(a: &str, b: &str, sep: &str) -> String {
    let ((ta, fa), (tb, fb)) = (split_reply(a), split_reply(b));
    let mut out = format!("{}{sep}{}", clean(ta), clean(tb));
    let facts: Vec<&str> = [fa, fb].into_iter().flatten().map(str::trim).collect();
    if !facts.is_empty() {
        out.push_str(&format!("\n{WIKI_MARK}\n{}", facts.join("\n")));
    }
    out
}

/// The paragraphs of a part's reply, `sent` paragraphs having been sent. They should
/// be split by blank lines; some models put them on single lines instead, so when
/// that gives fewer paragraphs than were sent, the lines are used when they come
/// closer to the count.
fn reply_paragraphs(text: &str, sent: usize) -> Vec<String> {
    let paras = split_paragraphs(text);
    if paras.len() >= sent {
        return paras;
    }
    let lines: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if lines.len() > paras.len() && lines.len() <= sent {
        lines
    } else {
        paras
    }
}

/// FNV-1a: a hash that stays the same across builds, for the parts' keys.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The replies kept for one chapter's parts (`ai_parts`), in the library file.
struct PartCache {
    conn: Mutex<Connection>,
    chapter_id: i64,
    lang: String,
    style: String,
    /// The translation as it stands, one place per block of the original (`None`:
    /// not translated yet), written to `content` as a draft after each part.
    draft: Mutex<Option<Draft>>,
}

struct Draft {
    title: String,
    /// The original's pictures and their positions, put in with the first part.
    pictures: Vec<(usize, String)>,
}

impl PartCache {
    /// None for a library that is not a file (tests in memory).
    fn open(db: &str, req: &Request) -> Result<Option<Self>, AiError> {
        if db.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            conn: Mutex::new(super::db::open(std::path::Path::new(db))?),
            chapter_id: req.chapter_id,
            lang: req.lang.clone(),
            style: req.style.clone(),
            draft: Mutex::new(None),
        }))
    }

    /// Keeps the translation in `segments` as its parts come back (`place`), with
    /// the original's pictures. `title` stands in until the save node's.
    fn with_draft(self, title: &str, paragraphs: &[String]) -> Self {
        let pictures = paragraphs
            .iter()
            .enumerate()
            .filter(|(_, p)| p.starts_with(IMAGE_MARK))
            .map(|(i, p)| (i, p.clone()))
            .collect();
        *self.draft.lock().unwrap() = Some(Draft {
            title: title.to_string(),
            pictures,
        });
        self
    }

    /// Writes `blocks` as the translation's segments at positions `start..`, the
    /// positions of the raw segments they translate. The row is a draft until the
    /// save node publishes it; one already published (a chapter translated again) is
    /// left as it is, with its segments, until the save node replaces them.
    fn place(&self, start: usize, blocks: &[String]) -> rusqlite::Result<()> {
        let mut draft = self.draft.lock().unwrap();
        let Some(draft) = draft.as_mut() else {
            return Ok(());
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "insert into chapter_translations (chapter_id, lang, style, source, state, title, content)
             values (?1, ?2, ?3, 'ai', 'draft', ?4, '')
             on conflict (chapter_id, lang, style) do update set
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where chapter_translations.state = 'draft'",
            params![self.chapter_id, self.lang, self.style, draft.title],
        )?;
        let state: String = conn.query_row(
            "select state from chapter_translations
             where chapter_id = ?1 and lang = ?2 and style = ?3",
            params![self.chapter_id, self.lang, self.style],
            |r| r.get(0),
        )?;
        if state != "draft" {
            return Ok(());
        }
        let style = Some(self.style.as_str());
        for (position, picture) in draft.pictures.drain(..) {
            super::segments::put(
                &conn,
                self.chapter_id,
                &self.lang,
                style,
                position,
                &[picture],
            )?;
        }
        super::segments::put(&conn, self.chapter_id, &self.lang, style, start, blocks)
    }

    fn get(&self, part: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "select reply from ai_parts
                 where chapter_id = ?1 and lang = ?2 and style = ?3 and part = ?4",
                params![self.chapter_id, self.lang, self.style, part],
                |r| r.get(0),
            )
            .optional()
    }

    fn put(&self, part: &str, reply: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "insert into ai_parts (chapter_id, lang, style, part, reply, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6)
             on conflict (chapter_id, lang, style, part) do update set reply = excluded.reply",
            params![
                self.chapter_id,
                self.lang,
                self.style,
                part,
                reply,
                super::db::now_ms()
            ],
        )?;
        Ok(())
    }

    /// The chapter is saved: its parts are not needed any more.
    fn clear(conn: &Connection, req: &Request) -> rusqlite::Result<()> {
        conn.execute(
            "delete from ai_parts where chapter_id = ?1 and lang = ?2 and style = ?3",
            params![req.chapter_id, req.lang, req.style],
        )?;
        Ok(())
    }
}

/// Put in front again when a part came back as words to the user.
const NO_CHATTER: &str = "\n\nLAST TIME YOU ANSWERED WITH A QUESTION OR A COMMENT. DO NOT. Translate or rewrite the text exactly as given, whatever its language, and reply with the result only. / LẦN TRƯỚC BẠN ĐÃ HỎI LẠI HOẶC BÌNH LUẬN. KHÔNG ĐƯỢC LÀM VẬY. Hãy dịch hoặc viết lại đúng văn bản nhận được, bất kể ngôn ngữ nào, và chỉ trả về kết quả.";
const CHATTER_ERROR: &str =
    "the model answered with a question or a comment instead of the text, twice";

/// Phrases of a model talking to the user about the task instead of giving the text.
const CHATTER: &[&str] = &[
    "bạn có thể cung cấp",
    "vui lòng cung cấp",
    "văn bản bạn cung cấp",
    "văn bản bạn gửi",
    "bạn đã cung cấp",
    "tôi hiểu nhiệm vụ",
    "tôi cần văn bản",
    "tôi cần bản gốc",
    "bản gốc tiếng",
    "could you provide",
    "can you provide",
    "please provide",
    "the text you provided",
    "the text you've provided",
    "you have provided",
    "i understand the task",
    "i need the original",
    "as an ai",
];

/// Whether a part's text is the model talking to the user rather than the text: one
/// of the `CHATTER` phrases (that the original does not have) in a reply much
/// shorter than the part, or one that only asks.
fn chatter(text: &str, paras: &[&str]) -> bool {
    let reply = text.to_lowercase();
    let source = paras.join("\n").to_lowercase();
    let said = CHATTER
        .iter()
        .any(|p| reply.contains(p) && !source.contains(p));
    if !said {
        return false;
    }
    let short = reply.chars().count() * 2 < source.chars().count();
    let asks = reply.trim_end().ends_with('?') && !source.trim_end().ends_with('?');
    short || asks
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
    // The row and its segments change together. `paragraphs` has one entry per block
    // of the original, so an entry's index is the position of its segment.
    let tx = conn.unchecked_transaction()?;
    let content = "";
    tx.execute(
        "insert into chapter_translations (chapter_id, lang, style, source, state, title, content, word_count, model)
         values (?1, ?2, ?3, 'ai', 'published', ?4, ?5, ?6, ?7)
         on conflict (chapter_id, lang, style) do update set title = excluded.title,
           content = excluded.content, word_count = excluded.word_count, model = excluded.model,
           source = 'ai', state = 'published', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        params![
            req.chapter_id,
            req.lang,
            req.style,
            title,
            content,
            super::word_count(&paragraphs.join("\n\n")),
            model
        ],
    )?;
    super::segments::write(&tx, req.chapter_id, &req.lang, Some(&req.style), paragraphs)?;
    tx.commit()
}

/// What a chapter being translated has so far: the draft's title and the paragraphs
/// of the parts back from the LLM, in the original's order. None when there is no
/// draft (nothing back yet, or a published chapter being made again).
pub fn draft(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: &str,
) -> rusqlite::Result<Option<(String, Vec<String>)>> {
    let title: Option<String> = conn
        .query_row(
            "select title from chapter_translations
             where chapter_id = ?1 and lang = ?2 and style = ?3 and source = 'ai'
               and state = 'draft'",
            params![chapter_id, lang, style],
            |r| r.get(0),
        )
        .optional()?;
    let Some(title) = title else { return Ok(None) };
    let paragraphs = super::segments::paragraphs(conn, chapter_id, lang, Some(style))?;
    Ok(Some((title, paragraphs)))
}

/// The AI text of a chapter in `lang` and `style`, if it was made.
pub fn get(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: &str,
) -> rusqlite::Result<Option<Translated>> {
    let found = conn
        .query_row(
            "select t.title, t.content, t.model, coalesce(b.source_lang, n.original_lang)
         from chapter_translations t join chapters c on c.id = t.chapter_id
         join novels n on n.id = c.novel_id left join ai_books b on b.novel_id = n.id
         where t.chapter_id = ?1 and t.lang = ?2 and t.style = ?3 and t.source = 'ai'
           and t.state = 'published'",
            params![chapter_id, lang, style],
            |r| {
                let content: String = r.get(1)?;
                let source: String = r.get(3)?;
                Ok((
                    content,
                    Translated {
                        title: r.get(0)?,
                        paragraphs: Vec::new(),
                        mode: if source == lang {
                            Mode::Rewrite
                        } else {
                            Mode::Translate
                        },
                        model: r.get(2)?,
                    },
                ))
            },
        )
        .optional()?;
    let Some((content, mut translated)) = found else {
        return Ok(None);
    };
    translated.paragraphs =
        super::segments::paragraphs_or(conn, chapter_id, lang, Some(style), &content)?;
    Ok(Some(translated))
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
           and t.state = 'published'
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
         where t.source = 'ai' and t.style is not null and t.state = 'published'
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
    conn.execute_batch(
        "delete from llm_config; delete from ai_jobs; delete from ai_parts; delete from ai_books;",
    )
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
            // The model finds the language itself: a text already in the target
            // language is rewritten, any other is translated.
            let target = if system.contains("Tự xác định ngôn ngữ gốc") {
                Some("vi")
            } else if system.contains("Find the language of the user's text yourself") {
                Some("en")
            } else {
                None
            };
            let rewrite = match target {
                Some(t) => t == self.detect,
                None => system.contains("editor") || system.contains("biên tập viên"),
            };
            Ok(if rewrite {
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
    fn a_reply_that_asks_back_is_not_taken_as_the_text() {
        let src = ["Heavenly Demon Cultivation Simulation", "- 천마육성 시뮬레이션 -", "Chapter 101", "\u{201c}How did you get here?\u{201d}", "As the servants removed the body from the room, Geum Man-jang quickly handed a chair to Seol-Hwi."];
        let asked = "Tôi hiểu nhiệm vụ. Đang dịch chương 152 từ tiếng Hàn.\n\nTuy nhiên, văn bản bạn cung cấp đã là tiếng Anh.\n\nBạn có thể cung cấp bản tiếng Hàn gốc để tôi dịch không?";
        assert!(chatter(asked, &src));
        let real = "Mô phỏng tu luyện Thiên Ma\n\n- 천마육성 시뮬레이션 -\n\nChương 101\n\n“Sao ngươi tới được đây?”\n\nKhi đám người hầu khiêng thi thể ra khỏi phòng, Kim Vạn Trượng vội đưa ghế cho Tuyết Huy.";
        assert!(!chatter(real, &src));
        // A line of the story that says "you provided" is not chatter when the original says it too.
        assert!(!chatter(
            "the text you provided was a lie, he said.",
            &["the text you provided was a lie, he said."]
        ));
        // Both prompts tell the model to find the language and never ask back.
        let vi = translate_prompt(AUTO, "vi", "natural", &[], "");
        assert!(
            vi.contains("Tự xác định ngôn ngữ gốc")
                && vi.contains("LUẬT CỨNG")
                && vi.contains("Câu hoàn chỉnh")
        );
        assert!(!vi.contains("{from}") && !vi.contains("{style}"));
        let en = translate_prompt(AUTO, "en", "natural", &[], "");
        assert!(en.contains("Find the language") && en.contains("HARD RULE"));
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
        // Two chunks around the picture + title: the language is not asked apart.
        assert_eq!(llm.count(), 3);
        let saved = get(&conn, 10, "vi", "natural").unwrap().unwrap();
        assert_eq!(saved.paragraphs, out.paragraphs);
        assert_eq!(saved.model.as_deref(), Some("fake-1"));
        run(&conn, llm.clone(), req(11, "vi")).unwrap();
        assert_eq!(llm.count(), 5);
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
        // The model finds the text is already in English and rewrites it, in the
        // same request that would translate it.
        let out = run(&conn, llm.clone(), req(11, "en")).unwrap();
        assert_eq!(out.paragraphs, vec!["[re] Second chapter."]);
        // Again: the input is still the original, not the rewrite.
        let again = run(&conn, llm.clone(), req(11, "en")).unwrap();
        assert_eq!(again.paragraphs, vec!["[re] Second chapter."]);
        // The reader's original text is untouched.
        let original: String = conn
            .query_row("select content from chapter_translations where chapter_id = 11 and source = 'original'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(original, "Second chapter.");
        // The rewrite is its own segments, with the language and style.
        assert_eq!(
            super::super::segments::paragraphs(&conn, 11, "en", Some("natural")).unwrap(),
            vec!["[re] Second chapter."]
        );
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
    fn a_books_genre_and_instructions_go_in_its_prompts() {
        let (conn, novel) = book("prefs");
        assert_eq!(prefs(&conn, novel).unwrap(), Prefs::default());
        set_prefs(
            &conn,
            novel,
            &Prefs {
                genre: "xianxia".into(),
                instructions: "  Gọi nhân vật chính là \"Lâm thiếu\".  ".into(),
            },
        )
        .unwrap();
        let p = prefs(&conn, novel).unwrap();
        assert_eq!(p.instructions, "Gọi nhân vật chính là \"Lâm thiếu\".");
        let rules = book_rules(&p.genre, "vi", &p.instructions);
        assert!(rules.contains("đạo hữu") && rules.contains("Lâm thiếu"));
        // An unknown genre is none.
        set_prefs(
            &conn,
            novel,
            &Prefs {
                genre: "space opera".into(),
                instructions: String::new(),
            },
        )
        .unwrap();
        assert_eq!(prefs(&conn, novel).unwrap().genre, "");
        assert_eq!(book_rules("", "vi", ""), "");
        // Non-fiction keeps the English names, in the wiki's rules too.
        assert!(book_rules("nonfiction", "vi", "").contains("tên gốc tiếng Anh"));
        assert!(
            super::super::wiki::extract_prompt("vi", WIKI_MARK, "nonfiction")
                .contains("tên gốc tiếng Anh")
        );
        for g in GENRES {
            assert!(!genre_rules(g, "vi").is_empty() && !genre_rules(g, "en").is_empty());
        }
    }

    #[test]
    fn prompts_ask_for_the_target_word_order() {
        let vi = rewrite_prompt("vi", "natural", "");
        assert!(vi.contains("câu tiếng Việt hoàn chỉnh, có nghĩa, đúng ngữ pháp"));
        assert!(vi.contains("Tự nhiên: trôi chảy") && vi.contains("Tự kiểm tra"));
        // General rules, all in Vietnamese: no list of cases, no English.
        assert!(!vi.contains("→") && !vi.contains("You are"));
        assert!(translate_prompt("zh", "vi", "natural", &[], "")
            .contains("Dịch văn bản người dùng gửi từ tiếng Trung sang tiếng Việt"));
        assert!(rewrite_prompt("en", "natural", "").contains("subject, then verb, then object"));
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
        let translate = calls.iter().find(|(s, _)| s.contains("dịch giả")).unwrap();
        assert!(translate.0.contains("- Second → Thứ Hai (thuật ngữ)"));
        // Fake has no tools, so the wiki was asked for as JSON after the translation.
        assert!(calls.last().unwrap().0.contains("JSON"));
        drop(calls);
        let done: i64 = conn
            .query_row(
                "select count(*) from wiki_chapters where chapter_id = 11",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(done, 1);
    }

    /// Translates and adds the facts after the mark when the prompt asks for them.
    struct Both(Fake);
    impl Chat for Both {
        fn chat(&self, system: &str, user: &str) -> Result<String, AiError> {
            let reply = self.0.chat(system, user)?;
            Ok(if system.contains(WIKI_MARK) {
                format!("{reply}\n{WIKI_MARK}\n{{\"entries\": [{{\"name\": \"Lina\", \"kind\": \"character\", \"translated\": \"Lina\"}}], \"relations\": []}}")
            } else {
                reply
            })
        }
        fn model(&self) -> &str {
            "both-1"
        }
    }

    #[test]
    fn the_wiki_queue_learns_what_the_translation_queue_left() {
        let (conn, novel) = book("later");
        super::super::wiki::set_auto(&conn, novel, true).unwrap();
        let llm = Arc::new(Both(Fake::new("en")));
        let hooks = Hooks {
            learn_found_later: true,
            ..Default::default()
        };
        run_with(&conn, llm.clone(), req(11, "vi"), hooks).unwrap();
        // The names went in as soon as the part brought them; the chapter is read in
        // by the wiki queue.
        assert_eq!(
            super::super::wiki::entries(&conn, novel).unwrap()[0].name,
            "Lina"
        );
        let before = llm.0.count();
        assert_eq!(
            super::super::wiki::learn_pending(&conn).unwrap(),
            vec![(novel, 1)]
        );
        assert_eq!(llm.0.count(), before);
        assert_eq!(
            super::super::wiki::entries(&conn, novel).unwrap()[0].name,
            "Lina"
        );
        // Nothing is left for the next turn.
        assert!(super::super::wiki::learn_pending(&conn).unwrap().is_empty());
    }

    #[test]
    fn the_wiki_learns_from_facts_the_translation_requests_bring() {
        let (conn, novel) = book("found");
        let llm = Arc::new(Both(Fake::new("en")));
        let names = || -> Vec<String> {
            super::super::wiki::entries(&conn, novel)
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect()
        };
        // The wiki off: the facts are kept with the chapter, the wiki stays empty.
        let out = run(&conn, llm.clone(), req(11, "vi")).unwrap();
        assert_eq!(out.paragraphs, vec!["[tr] Second chapter."]);
        assert!(names().is_empty());
        // Turned on: the kept facts go in with no request.
        super::super::wiki::set_auto(&conn, novel, true).unwrap();
        let before = llm.0.count();
        assert_eq!(super::super::wiki::learn_kept(&conn, novel).unwrap(), 1);
        assert_eq!(names(), vec!["Lina"]);
        assert_eq!(llm.0.count(), before);
        // On while translating: the same requests bring the facts, none is made to learn.
        let out = run(&conn, llm.clone(), req(10, "vi")).unwrap();
        assert_eq!(
            out.paragraphs,
            vec!["[tr] Once upon a time.", "\u{FFFC}pic1", "[tr] The end."]
        );
        // Two text parts and the title.
        assert_eq!(llm.0.count() - before, 3);
        assert_eq!(names(), vec!["Lina"]);
        let learnt: i64 = conn
            .query_row("select count(*) from wiki_chapters", [], |r| r.get(0))
            .unwrap();
        assert_eq!(learnt, 2);
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
        let (out, title, _) = run_chunks(
            &llm,
            &|_: &str| "novel translator".to_string(),
            &paragraphs,
            Some(("title".into(), "Chapter 1".into())),
            Some(&progress),
            None,
            None,
        )
        .unwrap();
        let want: Vec<String> = paragraphs
            .iter()
            .map(|p| {
                if p.starts_with(IMAGE_MARK) {
                    p.clone()
                } else {
                    format!("[tr] {p}")
                }
            })
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
            run_chunks(
                &Down,
                &|_: &str| "x".to_string(),
                &paragraphs,
                None,
                None,
                None,
                None
            ),
            Err(AiError::Timeout)
        ));
    }

    /// Translates like `Fake`, but the part with "p4" fails once the others are back.
    struct Flaky(Fake);
    impl Chat for Flaky {
        fn chat(&self, system: &str, user: &str) -> Result<String, AiError> {
            if user.contains("p4 ") {
                std::thread::sleep(std::time::Duration::from_millis(150));
                return Err(AiError::Http(400, "bad part".into()));
            }
            self.0.chat(system, user)
        }
        fn model(&self) -> &str {
            "flaky"
        }
    }

    /// A chapter of three parts (two long paragraphs each).
    fn long_chapter(conn: &Connection) -> i64 {
        let content: Vec<String> = (0..6)
            .map(|i| format!("p{i} {}", "x".repeat(1200)))
            .collect();
        conn.execute(
            "insert into chapters (id, novel_id, number) values (12, 1, 3)",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into chapter_translations (chapter_id, lang, source, state, title, content)
             values (12, 'en', 'original', 'published', 'Chapter 3', ?1)",
            [content.join("\n\n")],
        )
        .unwrap();
        12
    }

    fn kept_parts(conn: &Connection) -> i64 {
        conn.query_row("select count(*) from ai_parts", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_failed_chapter_keeps_its_parts_and_carries_on_from_them() {
        let (conn, _) = book("parts");
        let chapter = long_chapter(&conn);
        let flaky = Arc::new(Flaky(Fake::new("en")));
        assert!(matches!(
            run(&conn, flaky, req(chapter, "vi")),
            Err(AiError::Http(400, _))
        ));
        // The two other parts and the title came back and are kept.
        assert_eq!(kept_parts(&conn), 3);
        assert!(get(&conn, chapter, "vi", "natural").unwrap().is_none());
        // Their blocks are already segments of the translation, each at the position
        // of the raw segment it translates; the failed part's two are still to come.
        let stored = |conn: &Connection| -> (String, Vec<(i64, String)>) {
            let state: String = conn
                .query_row(
                    "select state from chapter_translations where chapter_id = ?1 and source = 'ai'",
                    [chapter],
                    |r| r.get(0),
                )
                .unwrap();
            let segments = conn
                .prepare(
                    "select position, content from segments
                     where chapter_id = ?1 and lang = 'vi' and style = 'natural' order by position",
                )
                .unwrap()
                .query_map([chapter], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            (state, segments)
        };
        let (state, segments) = stored(&conn);
        assert_eq!(state, "draft");
        assert_eq!(
            segments.iter().map(|s| s.0).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        for (position, content) in &segments {
            assert!(
                content.starts_with(&format!("[\"[tr] p{position} ")),
                "{content}"
            );
        }
        assert!(versions(&conn, 1).unwrap().is_empty());
        // What the "watch live" sheet reads: the paragraphs back so far.
        let (_, so_far) = draft(&conn, chapter, "vi", "natural").unwrap().unwrap();
        assert_eq!(so_far.len(), 4);
        assert!(so_far[0].starts_with("[tr] p0 "), "{so_far:?}");

        let llm = Arc::new(Fake::new("en"));
        let out = run(&conn, llm.clone(), req(chapter, "vi")).unwrap();
        let (state, segments) = stored(&conn);
        assert_eq!(state, "published");
        assert_eq!(
            segments.iter().map(|s| s.0).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5]
        );
        // Only the failed part went to the LLM again.
        let asked: Vec<String> = llm
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, user)| user.clone())
            .collect();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].starts_with("p4 "));
        assert_eq!(out.paragraphs.len(), 6);
        assert!(out.paragraphs.iter().all(|p| p.starts_with("[tr] p")));
        assert_eq!(out.title, "[tr] Chapter 3");
        // Saved: the parts are not kept any more.
        assert_eq!(kept_parts(&conn), 0);
    }

    #[test]
    fn a_stopped_chapter_sends_nothing_more() {
        let (conn, _) = book("stop");
        let chapter = long_chapter(&conn);
        let llm = Arc::new(Fake::new("en"));
        let hooks = Hooks {
            stop: Some(Arc::new(AtomicBool::new(true))),
            ..Default::default()
        };
        assert!(matches!(
            run_with(&conn, llm.clone(), req(chapter, "vi"), hooks),
            Err(AiError::Stopped)
        ));
        assert!(llm.calls.lock().unwrap().is_empty());
        assert!(get(&conn, chapter, "vi", "natural").unwrap().is_none());
    }

    #[test]
    fn each_part_gets_the_end_of_the_part_before() {
        let paragraphs: Vec<String> = (0..4)
            .map(|i| format!("p{i} {}", "x".repeat(1500)))
            .collect();
        let llm = Fake::new("en");
        run_chunks(
            &llm,
            &|_: &str| "novel translator".to_string(),
            &paragraphs,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let calls = llm.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let first = calls.iter().find(|(_, u)| u.starts_with("p0")).unwrap();
        let second = calls.iter().find(|(_, u)| u.starts_with("p2")).unwrap();
        assert!(!first.0.contains("<<<"));
        // The second part sees the end of p1, not p1 itself in full.
        assert!(second
            .0
            .contains(&format!("<<<\n{}\n>>>", "x".repeat(CONTEXT_CHARS))));
        assert!(!second.1.contains("p1"));
    }

    /// Cuts every reply off when it was asked for more than `max` characters.
    struct Short {
        max: usize,
        calls: Mutex<Vec<String>>,
    }
    impl Chat for Short {
        fn chat(&self, _: &str, user: &str) -> Result<String, AiError> {
            self.calls.lock().unwrap().push(user.to_string());
            let reply = split_paragraphs(user)
                .iter()
                .map(|p| format!("[tr] {p}"))
                .collect::<Vec<_>>()
                .join("\n\n")
                + &format!("\n{WIKI_MARK}\n{{}}");
            if user.chars().count() > self.max {
                Err(AiError::Truncated(
                    reply.chars().take(self.max / 2).collect(),
                ))
            } else {
                Ok(reply)
            }
        }
        fn model(&self) -> &str {
            "short"
        }
    }

    #[test]
    fn a_cut_off_reply_is_asked_again_in_halves() {
        let paragraphs: Vec<String> = (0..6)
            .map(|i| format!("p{i} {}", "y".repeat(500)))
            .collect();
        let llm = Short {
            max: 1200,
            calls: Mutex::new(Vec::new()),
        };
        let (out, _, found) = run_chunks(
            &llm,
            &|_: &str| "t".to_string(),
            &paragraphs,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let want: Vec<String> = paragraphs.iter().map(|p| format!("[tr] {p}")).collect();
        assert_eq!(out, want);
        // One part, cut; its halves (3 paragraphs) cut again; then 4 pieces of 1-2.
        assert!(llm.calls.lock().unwrap().len() > 1);
        assert_eq!(found.len(), 1);
        assert!(found[0].matches("{}").count() >= 2);
    }

    #[test]
    fn a_long_paragraph_cut_off_is_split_between_sentences() {
        let sentence = format!("{}. ", "z".repeat(99));
        let long = sentence.repeat(10).trim().to_string();
        let llm = Short {
            max: 600,
            calls: Mutex::new(Vec::new()),
        };
        let (out, _, _) = run_chunks(
            &llm,
            &|_: &str| "t".to_string(),
            std::slice::from_ref(&long),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(out.len(), 1, "still one paragraph: {out:?}");
        assert_eq!(out[0].replace("[tr] ", ""), long);
        // Too short to cut: the error stands.
        let llm = Short {
            max: 10,
            calls: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            run_chunks(
                &llm,
                &|_: &str| "t".to_string(),
                &["a b c d e f g h i j k l".into()],
                None,
                None,
                None,
                None,
            ),
            Err(AiError::Truncated(_))
        ));
    }

    #[test]
    fn paragraphs_on_single_lines_are_split() {
        assert_eq!(reply_paragraphs("a\nb\nc", 3), vec!["a", "b", "c"]);
        assert_eq!(reply_paragraphs("a\n\nb", 2), vec!["a", "b"]);
        // More lines than were sent: a paragraph with line breaks, kept whole.
        assert_eq!(reply_paragraphs("a\nb\nc", 1), vec!["a\nb\nc"]);
        assert_eq!(half_by_size(&["aaaa", "b", "c", "d"]), 1);
        assert_eq!(half_by_size(&["a", "b"]), 1);
        // One paragraph per block sent, whatever the reply's count.
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(fit(p(&["a", "b", "c"]), 2), p(&["a", "b\nc"]));
        assert_eq!(fit(p(&["a"]), 3), p(&["a", "", ""]));
    }

    /// Answers tool requests from a script, one reply per request, and text requests
    /// as "[txt] …".
    struct Tooly {
        script: Mutex<std::collections::VecDeque<Result<serde_json::Value, AiError>>>,
        asked: Mutex<Vec<Vec<serde_json::Value>>>,
        texts: Mutex<usize>,
    }

    impl Tooly {
        fn new(script: Vec<Result<serde_json::Value, AiError>>) -> Self {
            Tooly {
                script: Mutex::new(script.into()),
                asked: Mutex::new(Vec::new()),
                texts: Mutex::new(0),
            }
        }
    }

    impl Chat for Tooly {
        fn chat(&self, _: &str, user: &str) -> Result<String, AiError> {
            *self.texts.lock().unwrap() += 1;
            Ok(user
                .split("\n\n")
                .map(|p| format!("[txt] {p}"))
                .collect::<Vec<_>>()
                .join("\n\n"))
        }
        fn model(&self) -> &str {
            "tooly"
        }
        fn chat_tools_with(
            &self,
            messages: &[serde_json::Value],
            _: &serde_json::Value,
            _: &serde_json::Value,
        ) -> Result<serde_json::Value, AiError> {
            self.asked.lock().unwrap().push(messages.to_vec());
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(AiError::Empty))
        }
    }

    fn call(id: &str, name: &str, args: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "id": id, "type": "function",
            "function": { "name": name, "arguments": args.to_string() } })
    }

    fn calls(list: Vec<serde_json::Value>) -> Result<serde_json::Value, AiError> {
        Ok(serde_json::json!({ "role": "assistant", "content": null, "tool_calls": list }))
    }

    fn tools_with<'a>(lookup: &'a (dyn Fn(&str) -> String + Sync)) -> PartTools<'a> {
        PartTools {
            defs: super::super::wiki::part_tools("vi"),
            note: super::super::wiki::tool_prompt("vi", ""),
            lookup,
            keep: &|_| {},
            off: AtomicBool::new(false),
        }
    }

    #[test]
    fn a_part_comes_back_through_the_tools_in_one_request() {
        let paragraphs = vec![
            "Trần Bình An đối với Tề tiên sinh nói.".to_string(),
            "Hắn đi.".to_string(),
        ];
        let facts =
            serde_json::json!({ "entries": [{ "name": "Tề tiên sinh", "kind": "character" }] });
        let llm = Tooly::new(vec![calls(vec![
            call(
                "c1",
                "nop_ban_viet",
                serde_json::json!({ "doan": ["Trần Bình An nói với Tề tiên sinh.", "Hắn rời đi."] }),
            ),
            call("c2", "ghi_wiki", facts.clone()),
        ])]);
        let lookup = |_: &str| "[]".to_string();
        let tools = tools_with(&lookup);
        let (out, _, found) = run_chunks_with(
            &llm,
            &|_: &str| "sys".to_string(),
            "EXTRACT",
            Some(&tools),
            &paragraphs,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            out,
            vec!["Trần Bình An nói với Tề tiên sinh.", "Hắn rời đi."]
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].trim(), facts.to_string());
        // One request, no text request; the prompt says how to answer, not the JSON way.
        let asked = llm.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(*llm.texts.lock().unwrap(), 0);
        let system = asked[0][0]["content"].as_str().unwrap();
        assert!(system.starts_with("sys") && system.contains("nop_ban_viet"));
        assert!(!system.contains("EXTRACT"));
    }

    #[test]
    fn a_name_looked_up_is_answered_and_the_part_asked_again() {
        let paragraphs = vec!["Tề tiên sinh cười.".to_string()];
        let llm = Tooly::new(vec![
            calls(vec![call(
                "t1",
                "tra_ten",
                serde_json::json!({ "ten": "Tề tiên sinh" }),
            )]),
            calls(vec![call(
                "c1",
                "nop_ban_viet",
                serde_json::json!({ "doan": ["Tề tiên sinh mỉm cười."] }),
            )]),
        ]);
        let asked_for = Mutex::new(Vec::<String>::new());
        let lookup = |q: &str| {
            asked_for.lock().unwrap().push(q.to_string());
            r#"[{"name":"Tề Tĩnh Xuân","aliases":["Tề tiên sinh"]}]"#.to_string()
        };
        let tools = tools_with(&lookup);
        let (out, _, found) = run_chunks_with(
            &llm,
            &|_: &str| "sys".to_string(),
            "",
            Some(&tools),
            &paragraphs,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(out, vec!["Tề tiên sinh mỉm cười."]);
        assert!(found.is_empty());
        assert_eq!(*asked_for.lock().unwrap(), vec!["Tề tiên sinh"]);
        // The second request carries the call and its answer.
        let asked = llm.asked.lock().unwrap();
        assert_eq!(asked.len(), 2);
        let second = &asked[1];
        assert_eq!(second.len(), 4);
        assert_eq!(second[3]["role"], "tool");
        assert_eq!(second[3]["tool_call_id"], "t1");
        assert!(second[3]["content"]
            .as_str()
            .unwrap()
            .contains("Tề Tĩnh Xuân"));
    }

    #[test]
    fn without_tools_the_parts_are_asked_as_text() {
        let paragraphs = vec!["Một.".to_string(), "Hai.".to_string()];
        // The API refuses tools: the part is asked as text, and tools stay off.
        let llm = Tooly::new(vec![Err(AiError::Http(400, "tools".into()))]);
        let lookup = |_: &str| "[]".to_string();
        let tools = tools_with(&lookup);
        let (out, _, _) = run_chunks_with(
            &llm,
            &|_: &str| "sys".to_string(),
            "EXTRACT",
            Some(&tools),
            &paragraphs,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(out, vec!["[txt] Một.", "[txt] Hai."]);
        assert!(tools.off.load(Ordering::SeqCst));
        assert_eq!(*llm.texts.lock().unwrap(), 1);
        // A reply in words instead of the tool is the text, as a text answer is.
        let llm = Tooly::new(vec![Ok(
            serde_json::json!({ "role": "assistant", "content": "Một!\n\nHai!" }),
        )]);
        let tools = tools_with(&lookup);
        let (out, _, _) = run_chunks_with(
            &llm,
            &|_: &str| "sys".to_string(),
            "",
            Some(&tools),
            &paragraphs,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(out, vec!["Một!", "Hai!"]);
        // Arguments that are not JSON: asked again as text.
        let llm = Tooly::new(vec![calls(vec![
            serde_json::json!({ "id": "x", "function": { "name": "nop_ban_viet", "arguments": "{\"doan\": [\"cut" } }),
        ])]);
        let tools = tools_with(&lookup);
        let (out, _, _) = run_chunks_with(
            &llm,
            &|_: &str| "sys".to_string(),
            "",
            Some(&tools),
            &paragraphs,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(out, vec!["[txt] Một.", "[txt] Hai."]);
    }
}
