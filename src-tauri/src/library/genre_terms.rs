//! The shared terms the AI translation must write one way, whatever the book: sects,
//! places, famous people, realms, titles ("Võ Đang tông" → "Võ Đang phái",
//! "Sichuan" / "사천" → "Tứ Xuyên"). Unlike a book's own glossary (`glossary_entries`),
//! these come with the app.
//!
//! The data is `resources/genre-terms/`: `global/` for every book, and one folder per
//! style (genre: `wuxia`, `xianxia`, …) whose terms win over the global ones. It is
//! built into the app and kept as its own SQLite file, `<app data>/rag/genre-terms.sqlite`,
//! rebuilt when the app ships another version of the data (`version.txt`, or the data
//! itself changed). Before a chapter's part goes to the LLM, the part's text is compared
//! with the table and the part gets only the terms it names (`Terms::prompt`).
//!
//! Vector layer: the Latin-script spellings (English, Vietnamese convert) are also kept as
//! embeddings of the user's embedding model in `<app data>/rag/genre-terms-vec.sqlite`
//! (`term_vectors`), made the first time a book is translated with that model and kept
//! across data versions. Before a chapter is translated, its phrases that look like names
//! and match nothing exactly ("Vo Dang Tong") are embedded and looked up there; one close
//! enough to a term counts as a spelling of it (`Terms::near`). Chinese, Japanese and
//! Korean spellings are matched exactly only.
//!
//! Two shapes of TSV file (README.md there):
//! - a term file (header has `term`): one source spelling per row, one column per target
//!   language;
//! - a name file (header has no `term`): one sect / place / person per row, its target
//!   names (`vi`, `en`) and its spellings in each source language (`zh`, `ko`, `ja`, `en`,
//!   `convert` = Vietnamese convert text), several split by `|`.

use super::llm::Chat;
use aho_corasick::{AhoCorasick, MatchKind};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The data's version: raise it with every change to the TSV files.
const VERSION: &str = include_str!("../../resources/genre-terms/version.txt");

macro_rules! tsv {
    ($genre:literal, $file:literal) => {
        (
            $genre,
            concat!($genre, "/", $file),
            include_str!(concat!("../../resources/genre-terms/", $genre, "/", $file)),
        )
    };
}

/// (genre, file, TSV). `global` holds for every book, with a genre or not.
const SOURCES: &[(&str, &str, &str)] = &[
    tsv!("global", "address.tsv"),
    tsv!("global", "sects.tsv"),
    tsv!("global", "places.tsv"),
    tsv!("global", "people.tsv"),
    tsv!("wuxia", "terms.tsv"),
    tsv!("wuxia", "people.tsv"),
    tsv!("xianxia", "terms.tsv"),
    tsv!("supernatural", "terms.tsv"),
    tsv!("politics", "terms.tsv"),
];

/// How a sect, a gang, a clan… is called by the end of its name (派 → phái, 宗 → tông).
const SUFFIXES: &str = include_str!("../../resources/genre-terms/global/org-suffixes.tsv");
/// The genres whose prompts get the suffix rules.
const SUFFIX_GENRES: &[&str] = &["wuxia", "xianxia"];

/// The genre whose terms every book gets.
const GLOBAL: &str = "global";
/// Target languages a name file has a column for.
const TARGETS: &[&str] = &["vi", "en"];
/// Source columns of a name file besides the targets, and the language each stands for.
const SOURCE_COLS: &[(&str, &str)] = &[("zh", "zh"), ("ko", "ko"), ("ja", "ja"), ("convert", "vi")];
/// Terms put in one part's prompt at most.
const MAX_PER_PART: usize = 60;
/// The file, under the app's data folder.
const FILE: &str = "rag/genre-terms.sqlite";
/// The vector table's file: embeddings of the terms by the user's embedding model. Kept
/// apart, so a new version of the data does not embed again what it already has.
const VEC_FILE: &str = "rag/genre-terms-vec.sqlite";
/// Terms embedded per request.
const EMBED_BATCH: usize = 64;
/// Phrases of a chapter looked up in the vector table at most.
const NEAR_MAX: usize = 64;
/// A phrase is a term when closer than this (cosine distance)…
const NEAR: f32 = 0.12;
/// …or closer than this and sharing a word with it.
const FIND: f32 = 0.25;

const SCHEMA: &str = "
create table meta (key text primary key, value text not null);
-- One source spelling of a term and the name to use for it in one target language.
create table terms (
  id     integer primary key,
  genre  text not null,
  term   text not null,
  src    text not null,
  kind   text not null default 'term',
  lang   text not null,
  value  text not null,
  note   text,
  file   text not null,
  unique (genre, term, lang)
);
create index terms_genre_lang on terms (genre, lang);
-- How a kind of organisation is called by the end of its name.
create table org_suffixes (
  id      integer primary key,
  vi      text not null,
  en      text not null,
  zh      text not null,
  ko      text not null,
  ja      text not null,
  meaning text not null
);
";

/// One term: a source spelling (in `src`), its kind and note, and its name per target.
#[derive(Debug, PartialEq)]
struct Row {
    term: String,
    src: String,
    kind: String,
    note: String,
    values: Vec<(String, String)>,
}

/// The lines of a TSV that are not empty or comments, with their line numbers, and the
/// header's columns.
fn table(tsv: &str) -> (Vec<String>, Vec<(usize, Vec<&str>)>) {
    let mut lines = tsv
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let head = lines
        .next()
        .map(|(_, h)| h.split('\t').map(|c| c.trim().to_string()).collect())
        .unwrap_or_default();
    let rows = lines
        .map(|(n, l)| (n + 1, l.split('\t').map(str::trim).collect()))
        .collect();
    (head, rows)
}

/// The language a spelling is written in, as far as its letters tell.
fn script(text: &str) -> &'static str {
    if text.chars().any(|c| ('\u{AC00}'..='\u{D7AF}').contains(&c)) {
        "ko"
    } else if text.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)) {
        "ja"
    } else if text.chars().any(is_cjk) {
        "zh"
    } else {
        "vi"
    }
}

/// Reads one TSV file of either shape into terms.
fn parse(file: &str, tsv: &str) -> Result<Vec<Row>, String> {
    let (head, lines) = table(tsv);
    if head.is_empty() {
        return Ok(Vec::new());
    }
    let col = |name: &str| head.iter().position(|h| h == name);
    let (kind_col, note_col) = (col("kind"), col("note"));
    let mut rows = Vec::new();
    for (n, cells) in lines {
        if cells.len() > head.len() {
            return Err(format!("{file} line {n}: too many columns"));
        }
        let cell = |i: Option<usize>| i.and_then(|i| cells.get(i)).copied().unwrap_or("");
        let kind = match cell(kind_col) {
            "" => "term",
            k => k,
        }
        .to_string();
        let note = cell(note_col).to_string();
        if let Some(term_col) = col("term") {
            // A term file: the term, then a column per target language.
            let term = cell(Some(term_col));
            if term.is_empty() {
                return Err(format!("{file} line {n}: no term"));
            }
            let values: Vec<(String, String)> = head
                .iter()
                .enumerate()
                .filter(|(_, h)| !matches!(h.as_str(), "term" | "kind" | "note"))
                .map(|(i, h)| (h.clone(), cell(Some(i)).to_string()))
                .filter(|(_, v)| !v.is_empty())
                .collect();
            if values.is_empty() {
                return Err(format!("{file} line {n}: {term} has no translation"));
            }
            rows.push(Row {
                term: term.to_string(),
                src: script(term).to_string(),
                kind,
                note,
                values,
            });
            continue;
        }
        // A name file: the names to use, then the spellings of the sources.
        let first = |c: &str| cell(col(c)).split('|').next().unwrap_or("").trim().to_string();
        let targets: Vec<(String, String)> = TARGETS
            .iter()
            .map(|t| (t.to_string(), first(t)))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        let Some((_, vi)) = targets.iter().find(|(t, _)| t == "vi") else {
            return Err(format!("{file} line {n}: no vi name"));
        };
        let spellings = |c: &str| {
            cell(col(c))
                .split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let mut count = 0;
        for (c, src) in SOURCE_COLS {
            for term in spellings(c) {
                if term.to_lowercase() == vi.to_lowercase() {
                    continue;
                }
                count += 1;
                rows.push(Row {
                    term,
                    src: src.to_string(),
                    kind: kind.clone(),
                    note: note.clone(),
                    values: targets.clone(),
                });
            }
        }
        // English spellings, for an English book read in Vietnamese.
        for term in spellings("en") {
            count += 1;
            rows.push(Row {
                term,
                src: "en".into(),
                kind: kind.clone(),
                note: note.clone(),
                values: vec![("vi".into(), vi.clone())],
            });
        }
        if count == 0 {
            return Err(format!("{file} line {n}: {vi} has no source spelling"));
        }
    }
    Ok(rows)
}

/// The org suffix rows: (vi, en, zh, ko, ja, meaning).
fn suffixes() -> Result<Vec<[String; 6]>, String> {
    let (head, lines) = table(SUFFIXES);
    let col = |name: &str| head.iter().position(|h| h == name);
    let cols = ["vi", "en", "zh", "ko", "ja", "meaning"].map(col);
    lines
        .into_iter()
        .map(|(n, cells)| {
            let r = cols.map(|i| i.and_then(|i| cells.get(i)).copied().unwrap_or("").to_string());
            if r[0].is_empty() || r[2].is_empty() {
                return Err(format!("global/org-suffixes.tsv line {n}: needs vi and zh"));
            }
            Ok(r)
        })
        .collect()
}

/// The data the app carries, to tell a file built from older data.
fn data_hash() -> String {
    let h = SOURCES
        .iter()
        .flat_map(|(_, f, t)| [*f, *t])
        .chain([SUFFIXES])
        .flat_map(|s| s.bytes().chain([0]))
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{h:016x}")
}

fn version() -> &'static str {
    VERSION.trim()
}

/// Writes the file at `path` from the data the app carries. It is built next to it
/// and moved in place, so a reader never sees half a file.
pub fn build(path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("sqlite.tmp");
    let _ = std::fs::remove_file(&tmp);
    let r = (|| -> Result<(), String> {
        let e = |e: rusqlite::Error| e.to_string();
        let mut conn = Connection::open(&tmp).map_err(e)?;
        let tx = conn.transaction().map_err(e)?;
        tx.execute_batch(SCHEMA).map_err(e)?;
        {
            // The first row of a term in a genre is kept: a file lists it once.
            let mut put = tx
                .prepare(
                    "insert into terms (genre, term, src, kind, lang, value, note, file)
                     values (?1, ?2, ?3, ?4, ?5, ?6, nullif(?7, ''), ?8)
                     on conflict (genre, term, lang) do nothing",
                )
                .map_err(e)?;
            for (genre, file, tsv) in SOURCES {
                for row in parse(file, tsv)? {
                    for (lang, value) in &row.values {
                        put.execute(params![
                            genre, row.term, row.src, row.kind, lang, value, row.note, file
                        ])
                        .map_err(e)?;
                    }
                }
            }
            let mut suf = tx
                .prepare("insert into org_suffixes (vi, en, zh, ko, ja, meaning) values (?1, ?2, ?3, ?4, ?5, ?6)")
                .map_err(e)?;
            for r in suffixes()? {
                suf.execute(rusqlite::params_from_iter(r.iter())).map_err(e)?;
            }
            let mut meta = tx
                .prepare("insert into meta (key, value) values (?1, ?2)")
                .map_err(e)?;
            for (k, v) in [
                ("version", version().to_string()),
                ("data_hash", data_hash()),
                ("app_version", env!("CARGO_PKG_VERSION").to_string()),
            ] {
                meta.execute(params![k, v]).map_err(e)?;
            }
        }
        tx.commit().map_err(e)?;
        conn.close().map_err(|(_, err)| err.to_string())
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The file's (version, data hash), if it is a file this module built.
fn stamp(path: &Path) -> Option<(String, String)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let get = |k: &str| -> Option<String> {
        conn.query_row("select value from meta where key = ?1", [k], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    };
    Some((get("version")?, get("data_hash")?))
}

/// Builds the file at `path` unless it already holds the data the app carries.
pub fn ensure(path: &Path) -> Result<(), String> {
    if stamp(path) == Some((version().to_string(), data_hash())) {
        return Ok(());
    }
    build(path)
}

/// Where the app keeps the file, set once the app knows its data folder.
static PATH: OnceLock<PathBuf> = OnceLock::new();
/// Where the app keeps the vector table.
static VEC_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Whether `PATH` was checked (and built if needed) in this run.
static READY: Mutex<bool> = Mutex::new(false);

/// Sets the app's data folder and makes the file ready, off the caller's thread.
pub fn init(app_data: &Path) {
    let _ = PATH.set(app_data.join(FILE));
    let _ = VEC_PATH.set(app_data.join(VEC_FILE));
    std::thread::spawn(|| {
        if let Err(e) = ready() {
            eprintln!("genre terms: {e}");
        }
    });
}

/// The file, built if needed. None before `init` (tests, or no data folder).
fn ready() -> Result<Option<&'static Path>, String> {
    let Some(path) = PATH.get() else {
        return Ok(None);
    };
    let mut done = READY.lock().unwrap();
    if !*done {
        ensure(path)?;
        *done = true;
    }
    Ok(Some(path.as_path()))
}

/// The terms of `genre` (and the global ones) for a translation into `lang`, from the
/// app's file. Empty when the file cannot be had: a translation goes on without it.
pub fn load(genre: &str, lang: &str) -> Terms {
    let r = ready().and_then(|path| {
        let Some(path) = path else {
            return Ok(Terms::default());
        };
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        load_from(&conn, genre, lang).map_err(|e| e.to_string())
    });
    r.unwrap_or_else(|e| {
        eprintln!("genre terms: {e}");
        Terms::default()
    })
}

/// `load` from an open file. The genre's own term wins over the global one.
pub fn load_from(conn: &Connection, genre: &str, lang: &str) -> rusqlite::Result<Terms> {
    let mut stmt = conn.prepare(
        "select term, value from terms where lang = ?2 and genre in (?1, ?3)
         order by genre = ?3, term",
    )?;
    let rows = stmt.query_map(params![genre, lang, GLOBAL], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let (term, value) = row?;
        if seen.insert(term.to_lowercase()) {
            entries.push((term, value));
        }
    }
    let rules = rules(conn, genre, lang)?;
    Ok(Terms {
        lang: lang.to_string(),
        entries,
        rules,
        matcher: OnceLock::new(),
    })
}

/// What every part of a book gets besides the terms it names: how foreign names are
/// written and, in a martial genre, how each kind of organisation is called.
fn rules(conn: &Connection, genre: &str, lang: &str) -> rusqlite::Result<String> {
    let mut s = String::new();
    if lang == "vi" {
        s.push_str("\nTên riêng: địa danh, tên người, môn phái của Trung Quốc viết bằng Hán Việt (Sichuan / 사천 / 四川 → Tứ Xuyên); địa danh, tên người, tổ chức của Nhật, Hàn, phương Tây giữ nguyên tên gốc viết chữ Latin (東京 → Tokyo, 서울 → Seoul, California).\n");
    } else {
        s.push_str("\nNames: Chinese places, people and sects in pinyin or their usual English name; Japanese and Korean ones in their usual romanisation (Tokyo, Seoul).\n");
    }
    if !SUFFIX_GENRES.contains(&genre) {
        return Ok(s);
    }
    let mut stmt = conn.prepare("select vi, en, zh, ko, ja, meaning from org_suffixes order by id")?;
    let rows = stmt.query_map([], |r| {
        Ok((0..6).map(|i| r.get::<_, String>(i)).collect::<rusqlite::Result<Vec<_>>>())
    })?;
    s.push_str(if lang == "vi" {
        "Loại tổ chức theo chữ cuối tên gốc (tên không có trong danh sách thì dịch theo bảng này, giữ đúng loại, không đổi phái thành tông hay ngược lại):\n"
    } else {
        "Kinds of organisation by the end of the original name:\n"
    });
    for r in rows {
        let r = r??;
        let mut from: Vec<&str> = Vec::new();
        for c in [&r[2], &r[3], &r[4]] {
            for p in c.split('|').filter(|p| !p.is_empty()) {
                if !from.contains(&p) {
                    from.push(p);
                }
            }
        }
        if lang == "vi" {
            s.push_str(&format!("- {} → {}: {}\n", from.join(" / "), r[0], r[5]));
        } else {
            s.push_str(&format!("- {} → {}\n", from.join(" / "), r[1].replace('|', " / ")));
        }
    }
    Ok(s)
}

/// A genre's terms for one target language, found in a part's text.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Terms {
    lang: String,
    /// (term as the source writes it, the name to use).
    entries: Vec<(String, String)>,
    /// Put in every part's prompt (`rules`).
    rules: String,
    #[serde(skip)]
    matcher: OnceLock<Option<AhoCorasick>>,
}

impl Terms {
    /// Drops the terms a book's own glossary already names: the book's word wins.
    pub fn without(mut self, book: &[(String, String)]) -> Self {
        let keys: std::collections::HashSet<String> =
            book.iter().map(|(k, _)| k.to_lowercase()).collect();
        self.entries
            .retain(|(t, _)| !keys.contains(&t.to_lowercase()));
        self.matcher = OnceLock::new();
        self
    }

    fn matcher(&self) -> Option<&AhoCorasick> {
        self.matcher
            .get_or_init(|| {
                AhoCorasick::builder()
                    .match_kind(MatchKind::Standard)
                    .build(self.entries.iter().map(|(t, _)| t.to_lowercase()))
                    .ok()
            })
            .as_ref()
    }

    /// The terms `text` names, in the order they first appear. A term in a script with
    /// spaces between words ("Võ Đang tông", "Sichuan") must stand as whole words; a
    /// Chinese, Japanese or Korean one may sit anywhere. A term found inside a longer
    /// one ("武当" in "武当派") is left out.
    pub fn found(&self, text: &str) -> Vec<(&str, &str)> {
        let Some(ac) = self.matcher() else {
            return Vec::new();
        };
        let hay = text.to_lowercase();
        let mut hits: Vec<(usize, usize, usize)> = ac
            .find_overlapping_iter(&hay)
            .filter(|m| whole_word(&hay, m.start(), m.end()))
            .map(|m| (m.start(), m.end(), m.pattern().as_usize()))
            .collect();
        // Longest first at each place, then drop what a longer hit covers.
        hits.sort_by_key(|&(s, e, _)| (s, std::cmp::Reverse(e)));
        let mut out: Vec<(&str, &str)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut covered_to = 0;
        for (s, e, p) in hits {
            if e <= covered_to && s < covered_to {
                continue;
            }
            covered_to = covered_to.max(e);
            if seen.insert(p) {
                let (t, v) = &self.entries[p];
                out.push((t.as_str(), v.as_str()));
            }
        }
        out
    }

    /// Adds the phrases of `text` the vector table finds close to a term, as spellings
    /// of that term, so the parts that have them get the term (`prompt`). Needs `llm`
    /// with an embedding model; otherwise, or when anything fails, it adds nothing.
    /// Returns the spellings added.
    pub fn near(&mut self, llm: &dyn Chat, text: &str) -> usize {
        let Some(path) = VEC_PATH.get() else {
            return 0;
        };
        let r = open_vectors(path).and_then(|conn| self.near_with(&conn, llm, text));
        r.unwrap_or_else(|e| {
            eprintln!("genre terms near: {e}");
            0
        })
    }

    /// `near` with the vector table open.
    fn near_with(&mut self, conn: &Connection, llm: &dyn Chat, text: &str) -> Result<usize, String> {
        let Some(model) = llm.embed_model().map(str::to_string) else {
            return Ok(0);
        };
        // Phrases of two words or more that are not already a term, a part of one the
        // text has, or a name to use: one word alone ("Hắn", "Bang") is too often not
        // a name. Longest first, so a phrase inside an accepted one is skipped below.
        let exact: Vec<String> = self.found(text).iter().map(|(t, _)| t.to_lowercase()).collect();
        let values: std::collections::HashSet<String> =
            self.entries.iter().map(|(_, v)| fold(v)).collect();
        let mut phrases: Vec<String> = super::wiki::name_phrases(text)
            .into_iter()
            .filter(|p| p.split_whitespace().count() >= 2)
            .filter(|p| self.found(p).is_empty())
            .filter(|p| !exact.iter().any(|t| t.contains(&p.to_lowercase())))
            .filter(|p| !values.contains(&fold(p)))
            .take(NEAR_MAX)
            .collect();
        phrases.sort_by_key(|p| std::cmp::Reverse(p.split_whitespace().count()));
        if phrases.is_empty() {
            return Ok(0);
        }
        let table = vectors(conn, llm, &model, &self.entries)?;
        if table.is_empty() {
            return Ok(0);
        }
        let asked = llm.embed(&phrases).map_err(|e| e.to_string())?;
        let mut added = 0;
        let mut taken: Vec<String> = Vec::new();
        for (p, v) in phrases.iter().zip(asked) {
            if taken.iter().any(|t| t.contains(&p.to_lowercase())) {
                continue;
            }
            let best = table
                .iter()
                .map(|(i, t)| (*i, cosine_distance(&v, t)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((i, d)) = best else { continue };
            let (term, value) = self.entries[i].clone();
            // A capitalised word at the start of a sentence ("Hắn") is not a name: a
            // phrase must share a word with the term (marks aside), or be of two words
            // and very close.
            let close = (d < FIND && (shares_word(p, &term) || shares_word(p, &value))) || d < NEAR;
            if close && !self.entries.iter().any(|(t, _)| t.to_lowercase() == p.to_lowercase()) {
                self.entries.push((p.clone(), value));
                taken.push(p.to_lowercase());
                added += 1;
            }
        }
        if added > 0 {
            self.matcher = OnceLock::new();
        }
        Ok(added)
    }

    /// The part of a part's prompt: the rules, then the terms it names.
    pub fn prompt(&self, part: &str) -> String {
        let found: Vec<(&str, &str)> = self.found(part).into_iter().take(MAX_PER_PART).collect();
        let mut s = self.rules.clone();
        if found.is_empty() {
            return s;
        }
        s.push_str(if self.lang == "vi" {
            "\nThuật ngữ có trong đoạn này (dịch đúng như sau, trừ khi danh sách tên riêng của truyện ghi khác):\n"
        } else {
            "\nTerms in this text (always write them this way, unless the book's own names say otherwise):\n"
        });
        for (t, v) in found {
            s.push_str(&format!("- {t} → {v}\n"));
        }
        s
    }
}

/// Vietnamese letters without their marks, lower case ("Võ Đang" → "vo dang").
fn fold(text: &str) -> String {
    const FROM: &str = "àáảãạăằắẳẵặâầấẩẫậèéẻẽẹêềếểễệìíỉĩịòóỏõọôồốổỗộơờớởỡợùúủũụưừứửữựỳýỷỹỵđ";
    const TO: &str = "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooouuuuuuuuuuuyyyyyd";
    text.to_lowercase()
        .chars()
        .map(|c| FROM.chars().position(|f| f == c).and_then(|i| TO.chars().nth(i)).unwrap_or(c))
        .collect()
}

/// Whether `phrase` and `name` share a word, ignoring case and Vietnamese marks.
fn shares_word(phrase: &str, name: &str) -> bool {
    let name = fold(name);
    let words: Vec<&str> = name.split_whitespace().collect();
    fold(phrase).split_whitespace().any(|w| words.contains(&w))
}

const VEC_SCHEMA: &str = "
create table if not exists term_vectors (
  term      text not null,
  model     text not null,
  dims      integer not null,
  embedding blob not null,
  primary key (term, model)
);
";

fn open_vectors(path: &Path) -> Result<Connection, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(VEC_SCHEMA).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::MAX;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return f32::MAX;
    }
    1.0 - dot / (na.sqrt() * nb.sqrt())
}

/// The vectors of the Latin-script terms of `entries` by `model`: (index in `entries`,
/// vector). The ones the table does not have yet are embedded and kept.
fn vectors(
    conn: &Connection,
    llm: &dyn Chat,
    model: &str,
    entries: &[(String, String)],
) -> Result<Vec<(usize, Vec<f32>)>, String> {
    let latin: Vec<usize> = (0..entries.len())
        .filter(|&i| script(&entries[i].0) == "vi")
        .collect();
    let mut have: std::collections::HashMap<String, Vec<f32>> = Default::default();
    {
        let mut stmt = conn
            .prepare("select term, embedding from term_vectors where model = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (t, b) = row.map_err(|e| e.to_string())?;
            have.insert(t, from_blob(&b));
        }
    }
    let missing: Vec<String> = latin
        .iter()
        .map(|&i| entries[i].0.clone())
        .filter(|t| !have.contains_key(t))
        .collect();
    for batch in missing.chunks(EMBED_BATCH) {
        let got = llm.embed(batch).map_err(|e| e.to_string())?;
        if got.len() != batch.len() {
            return Err("embeddings: wrong count".into());
        }
        for (t, v) in batch.iter().zip(got) {
            conn.execute(
                "insert or replace into term_vectors (term, model, dims, embedding) values (?1, ?2, ?3, ?4)",
                params![t, model, v.len() as i64, to_blob(&v)],
            )
            .map_err(|e| e.to_string())?;
            have.insert(t.clone(), v);
        }
    }
    Ok(latin
        .into_iter()
        .filter_map(|i| have.get(&entries[i].0).map(|v| (i, v.clone())))
        .collect())
}

/// Whether the hit at `start..end` of `hay` is not part of a longer word. Only a term
/// whose ends are letters of a spaced script (Latin, Vietnamese) needs it.
fn whole_word(hay: &str, start: usize, end: usize) -> bool {
    let spaced = |c: char| c.is_alphanumeric() && !is_cjk(c);
    let first = hay[start..end].chars().next();
    let last = hay[start..end].chars().next_back();
    let before = hay[..start].chars().next_back();
    let after = hay[end..].chars().next();
    let ok_start = !first.is_some_and(spaced) || !before.is_some_and(spaced);
    let ok_end = !last.is_some_and(spaced) || !after.is_some_and(spaced);
    ok_start && ok_end
}

/// Chinese, Japanese and Korean letters, written without spaces between words.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0xAC00..=0xD7AF | 0x20000..=0x2FA1F)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("bennovel-gt-{name}-{nanos}/genre-terms.sqlite"))
    }

    /// Writes the data as a readable table, for review:
    /// `GENRE_TERMS_REVIEW=out.md cargo test --lib genre_terms::tests::review`.
    #[test]
    fn review() {
        let Ok(out) = std::env::var("GENRE_TERMS_REVIEW") else {
            return;
        };
        let mut s = format!("# Bảng thuật ngữ dùng khi dịch (version {})\n", version());
        for (_, file, tsv) in SOURCES.iter().copied().chain([("global", "global/org-suffixes.tsv", SUFFIXES)]) {
            let (head, rows) = table(tsv);
            s.push_str(&format!("\n## {file} ({} dòng)\n\n| {} |\n|{}\n", rows.len(), head.join(" | "), "---|".repeat(head.len())));
            for (_, cells) in rows {
                let cells: Vec<String> = (0..head.len())
                    .map(|i| cells.get(i).copied().unwrap_or("").replace('|', " / "))
                    .collect();
                s.push_str(&format!("| {} |\n", cells.join(" | ")));
            }
        }
        std::fs::write(out, s).unwrap();
    }

    #[test]
    fn every_tsv_reads_and_no_term_means_two_things() {
        let mut seen: std::collections::HashMap<(String, String, String), (String, String)> =
            Default::default();
        for (genre, file, tsv) in SOURCES {
            let rows = parse(file, tsv).unwrap_or_else(|e| panic!("{e}"));
            assert!(!rows.is_empty(), "{file} is empty");
            for r in &rows {
                for (lang, v) in &r.values {
                    let key = (genre.to_string(), r.term.to_lowercase(), lang.clone());
                    if let Some((f, old)) = seen.insert(key, (file.to_string(), v.clone())) {
                        assert_eq!(&old, v, "{}: {} is {old} in {f} but {v} in {file}", genre, r.term);
                    }
                }
                assert!(r.values.iter().any(|(l, _)| l == "vi") || r.src == "en", "{file}: {} has no vi", r.term);
            }
        }
        assert!(suffixes().unwrap().len() > 20);
        assert!(version().parse::<u32>().is_ok(), "version.txt must be a number");
    }

    #[test]
    fn parse_reads_both_shapes() {
        let rows = parse("x", "term\tkind\ten\tvi\n# c\n\nA tông\tsect\tA Sect\tA phái\n").unwrap();
        assert_eq!(
            rows,
            vec![Row {
                term: "A tông".into(),
                src: "vi".into(),
                kind: "sect".into(),
                note: "".into(),
                values: vec![("en".into(), "A Sect".into()), ("vi".into(), "A phái".into())],
            }]
        );
        assert!(parse("x", "term\tvi\nA\t\n").is_err());
        assert!(parse("x", "term\tvi\nA\tb\tc\n").is_err());
        let rows = parse(
            "x",
            "kind\tvi\ten\tzh\tko\tja\tconvert\nsect\tVõ Đang phái\tWudang Sect|Wudang Clan\t武当派|武當派\t무당파\t武当派\tVõ Đang tông\n",
        )
        .unwrap();
        let got: Vec<(&str, &str, Vec<(&str, &str)>)> = rows
            .iter()
            .map(|r| {
                (
                    r.term.as_str(),
                    r.src.as_str(),
                    r.values.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect(),
                )
            })
            .collect();
        let both = vec![("vi", "Võ Đang phái"), ("en", "Wudang Sect")];
        assert_eq!(
            got,
            vec![
                ("武当派", "zh", both.clone()),
                ("武當派", "zh", both.clone()),
                ("무당파", "ko", both.clone()),
                ("武当派", "ja", both.clone()),
                ("Võ Đang tông", "vi", both.clone()),
                ("Wudang Sect", "en", vec![("vi", "Võ Đang phái")]),
                ("Wudang Clan", "en", vec![("vi", "Võ Đang phái")]),
            ]
        );
        assert!(parse("x", "kind\tvi\tzh\nsect\t\t武当派\n").is_err());
    }

    #[test]
    fn builds_once_and_again_when_the_data_changes() {
        let path = temp("build");
        ensure(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let v: String = conn
            .query_row("select value from meta where key = 'version'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, version());
        let n: i64 = conn.query_row("select count(*) from terms", [], |r| r.get(0)).unwrap();
        assert!(n > 500, "{n}");
        // An older file is built again.
        conn.execute("update meta set value = 'old' where key = 'data_hash'", []).unwrap();
        drop(conn);
        ensure(&path).unwrap();
        assert_eq!(stamp(&path), Some((version().to_string(), data_hash())));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_part_gets_the_terms_it_names_in_every_source_language() {
        let path = temp("find");
        build(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let terms = load_from(&conn, "wuxia", "vi").unwrap();
        let found = terms.found("Chưởng môn võ đang tông tới. 武当派弟子 và 师兄. 사천당가의 무당파. He left Sichuan for the Wudang Sect.");
        assert_eq!(
            found,
            vec![
                ("Võ Đang tông", "Võ Đang phái"),
                ("武当派", "Võ Đang phái"),
                ("师兄", "sư huynh"),
                ("사천당가", "Tứ Xuyên Đường gia"),
                ("무당파", "Võ Đang phái"),
                ("Sichuan", "Tứ Xuyên"),
                ("Wudang Sect", "Võ Đang phái"),
            ]
        );
        let prompt = terms.prompt("Người của Võ Đang tông tới.");
        assert!(prompt.contains("- Võ Đang tông → Võ Đang phái"), "{prompt}");
        assert!(prompt.contains("→ phái:"), "wuxia gets the suffix rules: {prompt}");
        assert!(prompt.contains("Tokyo"), "{prompt}");
        // English, and the book's own glossary wins.
        let en = load_from(&conn, "wuxia", "en").unwrap();
        assert_eq!(en.found("少林寺"), vec![("少林寺", "Shaolin Temple")]);
        let mine = en.without(&[("少林寺".into(), "Shaolin Monastery".into())]);
        assert!(mine.found("少林寺").is_empty());
        // Another genre gets the global names but not wuxia's own terms nor the suffixes.
        let romance = load_from(&conn, "romance", "vi").unwrap();
        assert!(romance.found("内功").is_empty());
        assert_eq!(romance.found("四川"), vec![("四川", "Tứ Xuyên")]);
        assert!(!romance.prompt("x").contains("→ phái:"));
        let xx = load_from(&conn, "xianxia", "vi").unwrap();
        assert_eq!(xx.found("筑基 师兄"), vec![("筑基", "Trúc Cơ"), ("师兄", "sư huynh")]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Embeds a text by its letters without marks and case, so "Vo Dang tong" lands on
    /// "Võ Đang tông".
    struct Embedder(std::sync::Mutex<usize>);

    impl Chat for Embedder {
        fn chat(&self, _: &str, _: &str) -> Result<String, super::super::llm::AiError> {
            Ok(String::new())
        }
        fn model(&self) -> &str {
            "m"
        }
        fn embed_model(&self) -> Option<&str> {
            Some("emb")
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, super::super::llm::AiError> {
            *self.0.lock().unwrap() += texts.len();
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; 64];
                    for c in t.to_lowercase().chars() {
                        let c = match c {
                            'õ' | 'ơ' | 'ô' | 'ó' | 'ò' | 'ộ' | 'ồ' | 'ố' => 'o',
                            'đ' => 'd',
                            'á' | 'à' | 'â' | 'ă' | 'ạ' => 'a',
                            c => c,
                        };
                        if c.is_alphanumeric() {
                            v[(c as usize) % 64] += 1.0;
                        }
                    }
                    v
                })
                .collect())
        }
    }

    #[test]
    fn the_vector_table_finds_a_spelling_the_table_does_not_have() {
        let path = temp("vec");
        build(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let mut terms = load_from(&conn, "wuxia", "vi").unwrap();
        let vec = open_vectors(&path.with_file_name("vec.sqlite")).unwrap();
        let llm = Embedder(Default::default());
        let text = "Hắn là đệ tử Vo Dang Tong, quen biết người Cái Bang.";
        assert!(terms.found(text).is_empty());
        let added = terms.near_with(&vec, &llm, text).unwrap();
        assert_eq!(added, 1, "{:?}", &terms.entries[terms.entries.len() - added..]);
        assert_eq!(terms.found(text).first(), Some(&("Vo Dang Tong", "Võ Đang phái")));
        // The terms were embedded once and kept: a second chapter embeds its phrases only.
        let first = *llm.0.lock().unwrap();
        terms.near_with(&vec, &llm, "Gặp Vo Dang Tong lần nữa.").unwrap();
        let n = *llm.0.lock().unwrap() - first;
        assert!(n < 10, "{n} {:?}", &terms.entries[terms.entries.len() - added..]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_spaced_term_must_be_whole_words() {
        let t = Terms {
            lang: "vi".into(),
            entries: vec![("Hoa Sơn tông".into(), "Hoa Sơn phái".into())],
            ..Default::default()
        };
        assert!(t.found("Hoa Sơn tôngx").is_empty());
        assert!(t.found("xHoa Sơn tông").is_empty());
        assert_eq!(t.found("(hoa sơn tông)").len(), 1);
    }
}
