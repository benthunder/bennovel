//! The shared terms of each genre, for the AI translation: how a sect, a realm, a title
//! is written in the target language whatever the book ("Võ Đang tông" → "Võ Đang
//! phái" in wuxia). Unlike a book's own glossary (`glossary_entries`), these come with
//! the app.
//!
//! The data is `resources/genre-terms/*.tsv` (one file per genre, `common.tsv` for every
//! book), built into the app. It is kept as its own SQLite file,
//! `<app data>/rag/genre-terms.sqlite`, rebuilt when the app ships another version of
//! the data (`version.txt`, or the data itself changed). A chapter's part gets only the
//! terms that part names (`Terms::prompt`).

use aho_corasick::{AhoCorasick, MatchKind};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The data's version: raise it with every change to the TSV files.
const VERSION: &str = include_str!("../../resources/genre-terms/version.txt");

/// (genre, TSV). `common` holds for every book, with a genre or not.
const SOURCES: &[(&str, &str)] = &[
    (
        "common",
        include_str!("../../resources/genre-terms/common.tsv"),
    ),
    (
        "wuxia",
        include_str!("../../resources/genre-terms/wuxia.tsv"),
    ),
    (
        "xianxia",
        include_str!("../../resources/genre-terms/xianxia.tsv"),
    ),
    (
        "supernatural",
        include_str!("../../resources/genre-terms/supernatural.tsv"),
    ),
    (
        "politics",
        include_str!("../../resources/genre-terms/politics.tsv"),
    ),
];

/// The genre whose terms every book gets.
const COMMON: &str = "common";
/// Terms put in one part's prompt at most.
const MAX_PER_PART: usize = 60;
/// The file, under the app's data folder.
const FILE: &str = "rag/genre-terms.sqlite";

const SCHEMA: &str = "
create table meta (key text primary key, value text not null);
create table terms (
  id     integer primary key,
  genre  text not null,
  term   text not null,
  kind   text not null default 'term',
  lang   text not null,
  value  text not null,
  note   text,
  unique (genre, term, lang)
);
create index terms_genre_lang on terms (genre, lang);
";

/// One row of a TSV file: the term, its kind, its note and its value per language.
#[derive(Debug, PartialEq)]
struct Row {
    term: String,
    kind: String,
    note: String,
    values: Vec<(String, String)>,
}

/// Reads one TSV file. The header names the columns: `term`, `kind`, `note` and one
/// column per target language.
fn parse(genre: &str, tsv: &str) -> Result<Vec<Row>, String> {
    let mut lines = tsv
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let Some((_, head)) = lines.next() else {
        return Ok(Vec::new());
    };
    let head: Vec<&str> = head.split('\t').map(str::trim).collect();
    let col = |name: &str| head.iter().position(|h| *h == name);
    let term_col = col("term").ok_or(format!("{genre}.tsv: no term column"))?;
    let (kind_col, note_col) = (col("kind"), col("note"));
    let langs: Vec<(usize, &str)> = head
        .iter()
        .enumerate()
        .filter(|(_, h)| !matches!(**h, "term" | "kind" | "note"))
        .map(|(i, h)| (i, *h))
        .collect();
    let mut rows = Vec::new();
    for (n, line) in lines {
        let cells: Vec<&str> = line.split('\t').map(str::trim).collect();
        if cells.len() > head.len() {
            return Err(format!("{genre}.tsv line {}: too many columns", n + 1));
        }
        let cell = |i: Option<usize>| i.and_then(|i| cells.get(i)).copied().unwrap_or("");
        let term = cell(Some(term_col));
        if term.is_empty() {
            return Err(format!("{genre}.tsv line {}: no term", n + 1));
        }
        let values: Vec<(String, String)> = langs
            .iter()
            .map(|(i, lang)| (lang.to_string(), cell(Some(*i)).to_string()))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        if values.is_empty() {
            return Err(format!(
                "{genre}.tsv line {}: {term} has no translation",
                n + 1
            ));
        }
        let kind = cell(kind_col);
        rows.push(Row {
            term: term.to_string(),
            kind: if kind.is_empty() { "term" } else { kind }.to_string(),
            note: cell(note_col).to_string(),
            values,
        });
    }
    Ok(rows)
}

/// The data the app carries, to tell a file built from older data.
fn data_hash() -> String {
    let h = SOURCES
        .iter()
        .flat_map(|(g, t)| g.bytes().chain([0]).chain(t.bytes()).chain([0]))
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
        let mut conn = Connection::open(&tmp).map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        {
            let mut put = tx
                .prepare(
                    "insert into terms (genre, term, kind, lang, value, note) values (?1, ?2, ?3, ?4, ?5, nullif(?6, ''))
                     on conflict (genre, term, lang) do update set value = excluded.value",
                )
                .map_err(|e| e.to_string())?;
            for (genre, tsv) in SOURCES {
                for row in parse(genre, tsv)? {
                    for (lang, value) in &row.values {
                        put.execute(params![genre, row.term, row.kind, lang, value, row.note])
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            let mut meta = tx
                .prepare("insert into meta (key, value) values (?1, ?2)")
                .map_err(|e| e.to_string())?;
            for (k, v) in [
                ("version", version().to_string()),
                ("data_hash", data_hash()),
                ("app_version", env!("CARGO_PKG_VERSION").to_string()),
            ] {
                meta.execute(params![k, v]).map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        conn.close().map_err(|(_, e)| e.to_string())
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
/// Whether `PATH` was checked (and built if needed) in this run.
static READY: Mutex<bool> = Mutex::new(false);

/// Sets the app's data folder and makes the file ready, off the caller's thread.
pub fn init(app_data: &Path) {
    let _ = PATH.set(app_data.join(FILE));
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

/// The terms of `genre` (and `common`) for a translation into `lang`, from the app's
/// file. Empty when the file cannot be had: a translation goes on without it.
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

/// `load` from an open file. The genre's own term wins over `common`'s.
pub fn load_from(conn: &Connection, genre: &str, lang: &str) -> rusqlite::Result<Terms> {
    let mut stmt = conn.prepare(
        "select term, value from terms where lang = ?2 and genre in (?1, ?3)
         order by genre = ?3, term",
    )?;
    let rows = stmt.query_map(params![genre, lang, COMMON], |r| {
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
    Ok(Terms {
        lang: lang.to_string(),
        entries,
        matcher: OnceLock::new(),
    })
}

/// A genre's terms for one target language, found in a part's text.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Terms {
    lang: String,
    /// (term as the source writes it, the name to use).
    entries: Vec<(String, String)>,
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
    /// spaces between words ("Võ Đang tông") must stand as whole words; a Chinese one
    /// may sit anywhere. A term found inside a longer one found at the same place
    /// ("武当" in "武当派") is left out.
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

    /// The part of a part's prompt with the terms it names, or '' when none.
    pub fn prompt(&self, part: &str) -> String {
        let found: Vec<(&str, &str)> = self.found(part).into_iter().take(MAX_PER_PART).collect();
        if found.is_empty() {
            return String::new();
        }
        let mut s = if self.lang == "vi" {
            "\nThuật ngữ chung của thể loại (dịch đúng như sau, trừ khi danh sách tên riêng của truyện ghi khác):\n".to_string()
        } else {
            "\nCommon terms of this genre (always write them this way, unless the book's own names say otherwise):\n".to_string()
        };
        for (t, v) in found {
            s.push_str(&format!("- {t} → {v}\n"));
        }
        s
    }
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

    #[test]
    fn every_tsv_reads_and_has_no_twice_named_term() {
        for (genre, tsv) in SOURCES {
            let rows = parse(genre, tsv).unwrap_or_else(|e| panic!("{e}"));
            assert!(!rows.is_empty(), "{genre}.tsv is empty");
            let mut seen = std::collections::HashSet::new();
            for r in &rows {
                assert!(
                    seen.insert(r.term.to_lowercase()),
                    "{genre}.tsv: {} twice",
                    r.term
                );
                assert!(
                    r.values.iter().any(|(l, _)| l == "vi"),
                    "{genre}.tsv: {} has no vi",
                    r.term
                );
            }
        }
        assert!(
            version().parse::<u32>().is_ok(),
            "version.txt must be a number"
        );
    }

    #[test]
    fn parse_reads_columns_by_header() {
        let rows = parse(
            "x",
            "term\tkind\ten\tvi\n# c\n\nA tông\tsect\tA Sect\tA phái\nB\t\t\tb\n",
        )
        .unwrap();
        assert_eq!(
            rows,
            vec![
                Row {
                    term: "A tông".into(),
                    kind: "sect".into(),
                    note: "".into(),
                    values: vec![
                        ("en".into(), "A Sect".into()),
                        ("vi".into(), "A phái".into())
                    ],
                },
                Row {
                    term: "B".into(),
                    kind: "term".into(),
                    note: "".into(),
                    values: vec![("vi".into(), "b".into())],
                },
            ]
        );
        assert!(parse("x", "term\tvi\nA\t\n").is_err());
        assert!(parse("x", "term\tvi\nA\tb\tc\n").is_err());
    }

    #[test]
    fn builds_once_and_again_when_the_data_changes() {
        let path = temp("build");
        ensure(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let v: String = conn
            .query_row("select value from meta where key = 'version'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, version());
        let n: i64 = conn
            .query_row("select count(*) from terms", [], |r| r.get(0))
            .unwrap();
        assert!(n > 100);
        // An older file is built again.
        conn.execute("update meta set value = 'old' where key = 'data_hash'", [])
            .unwrap();
        drop(conn);
        ensure(&path).unwrap();
        assert_eq!(stamp(&path), Some((version().to_string(), data_hash())));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_wuxia_part_gets_the_terms_it_names() {
        let path = temp("wuxia");
        build(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let terms = load_from(&conn, "wuxia", "vi").unwrap();
        let found = terms.found("Chưởng môn võ đang tông cùng sư phụ đến. 武当派弟子 và 师兄.");
        assert_eq!(
            found,
            vec![
                ("Võ Đang tông", "Võ Đang phái"),
                ("武当派", "Võ Đang phái"),
                ("师兄", "sư huynh"),
            ]
        );
        let prompt = terms.prompt("Người của Võ Đang tông tới.");
        assert!(prompt.contains("- Võ Đang tông → Võ Đang phái"), "{prompt}");
        assert_eq!(terms.prompt("Không có gì."), "");
        // English, and the book's own glossary wins.
        let en = load_from(&conn, "wuxia", "en").unwrap();
        assert_eq!(en.found("少林寺"), vec![("少林寺", "Shaolin Temple")]);
        let mine = en.without(&[("少林寺".into(), "Shaolin Monastery".into())]);
        assert_eq!(mine.found("少林寺"), vec![("少林", "Shaolin")]);
        // Another genre does not get wuxia's terms, but gets the common ones.
        let xx = load_from(&conn, "xianxia", "vi").unwrap();
        assert!(xx.found("Võ Đang tông").is_empty());
        assert_eq!(
            xx.found("筑基 师兄"),
            vec![("筑基", "Trúc Cơ"), ("师兄", "sư huynh")]
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_spaced_term_must_be_whole_words() {
        let t = Terms {
            lang: "vi".into(),
            entries: vec![("Hoa Sơn tông".into(), "Hoa Sơn phái".into())],
            matcher: OnceLock::new(),
        };
        assert!(t.found("Hoa Sơn tôngx").is_empty());
        assert!(t.found("xHoa Sơn tông").is_empty());
        assert_eq!(t.found("(hoa sơn tông)").len(), 1);
    }
}
