//! The wiki of an imported book: its characters, places, factions, items and terms,
//! and the relations between them.
//!
//! - The AI fills it as it translates (`learn`, the last node of the graph in
//!   translate.rs): the model gets tools (`wiki_search`, `wiki_upsert`, `wiki_relate`)
//!   in the OpenAI tool-calling format and runs them in a loop; an API or model
//!   without tools gets the same job as one JSON answer instead.
//! - Before a chapter is translated, the entries it mentions (by name or alias, found
//!   in one pass by names.rs) go in the prompt (`context`), so names and relations
//!   stay the same through the book. There are no embeddings: what the translation
//!   requests find (`extract_prompt`) is kept in the library and found by name.
//! - An entry the user edited keeps their words: the AI may only add to it. One the
//!   user removed stays removed.
//! - What an entry is and how it links to others is the same in every language: the
//!   relation types (`REL_TYPES`) are codes the app names in each language, levels are
//!   ordered by `rank`, places nest by `located_in` (the map). Only the words are kept
//!   per language (`names`, `summaries`, a free relation's `labels`); the app shows
//!   them in the reading language, else the raw ones, and `translate_missing` fills a
//!   language in.

use super::llm::{AiError, Chat};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const KINDS: &[&str] = &[
    "character",
    "place",
    "faction",
    "item",
    "level",
    "skill",
    "term",
    "other",
];
/// Relation types, as `from` to `to` ("A master_of B"). The app shows each in the
/// reading language and from the other side too ("B disciple of A"). Anything else is
/// a free relation in someone's own words.
pub const REL_TYPES: &[&str] = &[
    "master_of",
    "disciple_of",
    "parent_of",
    "child_of",
    "sibling_of",
    "spouse_of",
    "lover_of",
    "friend_of",
    "enemy_of",
    "member_of",
    "leader_of",
    "located_in",
    "owns",
    "has_level",
    "has_skill",
];
/// The key of the descriptions written in the book's own language.
pub const RAW: &str = "raw";
/// Text of the entries or relations put in one request of `translate_missing`, in
/// characters (about 1,500 tokens): the reply is about as long, so both stay well inside
/// any model's limits however long the descriptions are.
const TRANSLATE_CHARS: usize = 5_000;
/// Entries or relations put in one request at most, however short.
const TRANSLATE_BATCH: usize = 25;
/// Requests of `translate_missing` running at once.
const TRANSLATE_PARALLEL: usize = 4;
/// Tool-call rounds allowed for one chapter.
const MAX_ROUNDS: usize = 12;
/// Entries put in a translation prompt at most.
const CONTEXT_ENTRIES: usize = 40;
/// Text of a chapter the AI reads to learn from it, in characters.
const ORIGINAL_CHARS: usize = 12_000;
const TRANSLATED_CHARS: usize = 6_000;
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub uid: String,
    pub kind: String,
    /// As written in the original text.
    pub name: String,
    /// The name to use per language.
    pub names: BTreeMap<String, String>,
    pub aliases: Vec<String>,
    /// The description per language; `RAW` in the book's own language.
    pub summaries: BTreeMap<String, String>,
    /// A level's place in its ladder, lowest first.
    pub rank: Option<i64>,
    /// The chapter it was first met in, in reading order from 1.
    pub first_chapter: Option<i64>,
    /// That chapter's own number as the book shows it ("Chương 629"), when it has one.
    pub first_chapter_shown: Option<f64>,
    /// "ai" or "user" (edited by the user).
    pub source: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub uid: String,
    pub from_uid: String,
    pub to_uid: String,
    /// One of `REL_TYPES`, or a free relation's words as first written.
    pub kind: String,
    /// A free relation's words per language.
    pub labels: BTreeMap<String, String>,
    pub note: String,
    pub first_chapter: Option<i64>,
    pub first_chapter_shown: Option<f64>,
}

/// An entry as the app sends it: a new one without `uid`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryInput {
    #[serde(default)]
    pub uid: Option<String>,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub summaries: BTreeMap<String, String>,
    #[serde(default)]
    pub rank: Option<i64>,
}

/// A book's wiki as the app shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wiki {
    pub entries: Vec<Entry>,
    pub relations: Vec<Relation>,
    /// Translating updates the wiki.
    pub auto: bool,
    pub last_error: Option<String>,
}

const ENTRY_COLS: &str =
    "e.uid, e.kind, e.name, e.names, e.aliases, e.summaries, e.first_chapter, e.source, e.updated_at, e.rank,
     (select c.display_number from chapters c where c.novel_id = e.novel_id and c.number = e.first_chapter)";
/// Columns after `ENTRY_COLS` start here.
const AFTER: usize = 11;

fn entry_row(r: &Row) -> rusqlite::Result<Entry> {
    let names: String = r.get(3)?;
    let aliases: String = r.get(4)?;
    let summaries: String = r.get(5)?;
    Ok(Entry {
        uid: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        names: serde_json::from_str(&names).unwrap_or_default(),
        aliases: serde_json::from_str(&aliases).unwrap_or_default(),
        summaries: serde_json::from_str(&summaries).unwrap_or_default(),
        first_chapter: r.get(6)?,
        source: r.get(7)?,
        updated_at: r.get(8)?,
        rank: r.get(9)?,
        first_chapter_shown: r.get(10)?,
    })
}

impl Entry {
    /// The description in `lang`, else the raw one, else any.
    pub fn summary_in(&self, lang: &str) -> &str {
        self.summaries
            .get(lang)
            .or_else(|| self.summaries.get(RAW))
            .or_else(|| self.summaries.values().next())
            .map(String::as_str)
            .unwrap_or("")
    }
}

/// A relation type's code for `kind` ("Master of" → master_of), if it is one.
pub fn rel_type(kind: &str) -> Option<&'static str> {
    let k = kind.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    REL_TYPES.iter().find(|t| **t == k).copied()
}

/// How a relation reads in prompts to the model (English words for the types).
fn rel_words(r: &Relation, lang: &str) -> String {
    match rel_type(&r.kind) {
        Some(t) => t.replace('_', " "),
        None => r
            .labels
            .get(lang)
            .cloned()
            .unwrap_or_else(|| r.kind.clone()),
    }
}

fn to_json<T: Serialize>(v: &T, empty: &str) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| empty.into())
}

fn new_uid(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row("select lower(hex(randomblob(16)))", [], |r| r.get(0))
}

pub fn entries(conn: &Connection, novel_id: i64) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare(&format!(
        "select {ENTRY_COLS} from wiki_entries e where e.novel_id = ?1 and e.deleted = 0
         order by e.kind, e.name"
    ))?;
    let rows = stmt.query_map([novel_id], entry_row)?;
    rows.collect()
}

pub fn relations(conn: &Connection, novel_id: i64) -> rusqlite::Result<Vec<Relation>> {
    let mut stmt = conn.prepare(
        "select r.uid, r.from_uid, r.to_uid, r.kind, r.note, r.first_chapter, r.labels,
           (select c.display_number from chapters c where c.novel_id = r.novel_id and c.number = r.first_chapter)
         from wiki_relations r
         join wiki_entries a on a.uid = r.from_uid join wiki_entries b on b.uid = r.to_uid
         where r.novel_id = ?1 and r.deleted = 0 and a.deleted = 0 and b.deleted = 0
         order by r.id",
    )?;
    let rows = stmt.query_map([novel_id], |r| {
        let labels: String = r.get(6)?;
        Ok(Relation {
            uid: r.get(0)?,
            from_uid: r.get(1)?,
            to_uid: r.get(2)?,
            kind: r.get(3)?,
            labels: serde_json::from_str(&labels).unwrap_or_default(),
            note: r.get(4)?,
            first_chapter: r.get(5)?,
            first_chapter_shown: r.get(7)?,
        })
    })?;
    rows.collect()
}

pub fn load(conn: &Connection, novel_id: i64) -> rusqlite::Result<Wiki> {
    let (auto, last_error) = book_state(conn, novel_id)?;
    Ok(Wiki {
        entries: entries(conn, novel_id)?,
        relations: relations(conn, novel_id)?,
        auto,
        last_error,
    })
}

fn book_state(conn: &Connection, novel_id: i64) -> rusqlite::Result<(bool, Option<String>)> {
    Ok(conn
        .query_row(
            "select auto, last_error from wiki_books where novel_id = ?1",
            [novel_id],
            |r| Ok((r.get::<_, bool>(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or((true, None)))
}

/// Whether translating this book updates its wiki (on unless turned off).
pub fn auto(conn: &Connection, novel_id: i64) -> rusqlite::Result<bool> {
    Ok(book_state(conn, novel_id)?.0)
}

pub fn set_auto(conn: &Connection, novel_id: i64, auto: bool) -> rusqlite::Result<()> {
    conn.execute(
        "insert into wiki_books (novel_id, auto) values (?1, ?2)
         on conflict (novel_id) do update set auto = excluded.auto",
        params![novel_id, auto],
    )?;
    Ok(())
}

fn set_error(conn: &Connection, novel_id: i64, error: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "insert into wiki_books (novel_id, last_error) values (?1, ?2)
         on conflict (novel_id) do update set last_error = excluded.last_error",
        params![novel_id, error],
    )?;
    Ok(())
}

/// The entry named `name` (or with it as an alias), removed ones too, with its row id
/// and whether it was removed.
fn find(
    conn: &Connection,
    novel_id: i64,
    name: &str,
) -> rusqlite::Result<Option<(i64, Entry, bool)>> {
    conn.query_row(
        &format!(
            "select {ENTRY_COLS}, e.id, e.deleted from wiki_entries e where e.novel_id = ?1 and (
               e.name = ?2 or exists (select 1 from json_each(e.aliases) a where a.value = ?2))
             order by e.name = ?2 desc, e.deleted limit 1"
        ),
        params![novel_id, name.trim()],
        |r| Ok((r.get(AFTER)?, entry_row(r)?, r.get::<_, bool>(AFTER + 1)?)),
    )
    .optional()
}

fn by_uid(conn: &Connection, uid: &str) -> rusqlite::Result<Option<Entry>> {
    conn.query_row(
        &format!("select {ENTRY_COLS} from wiki_entries e where e.uid = ?1"),
        [uid],
        entry_row,
    )
    .optional()
}

fn clean_list(items: impl IntoIterator<Item = String>, name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in items {
        let a = a.trim().to_string();
        if !a.is_empty() && a != name && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

fn clean_names(names: BTreeMap<String, String>) -> BTreeMap<String, String> {
    names
        .into_iter()
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

/// An entry's fields as written to its row.
struct Fields<'a> {
    kind: &'a str,
    name: &'a str,
    names: &'a BTreeMap<String, String>,
    aliases: &'a [String],
    summaries: &'a BTreeMap<String, String>,
    rank: Option<i64>,
    source: &'a str,
}

fn write_entry(conn: &Connection, id: i64, f: &Fields) -> rusqlite::Result<()> {
    conn.execute(
        "update wiki_entries set kind = ?2, name = ?3, names = ?4, aliases = ?5, summaries = ?6,
           rank = ?7, source = ?8, deleted = 0, updated_at = ?9 where id = ?1",
        params![
            id,
            f.kind,
            f.name,
            to_json(f.names, "{}"),
            to_json(&f.aliases, "[]"),
            to_json(f.summaries, "{}"),
            f.rank,
            f.source,
            super::db::now_ms()
        ],
    )?;
    Ok(())
}

fn insert_entry(
    conn: &Connection,
    novel_id: i64,
    f: &Fields,
    first_chapter: Option<i64>,
) -> rusqlite::Result<String> {
    let uid = new_uid(conn)?;
    conn.execute(
        "insert into wiki_entries (uid, novel_id, kind, name, names, aliases, summaries, rank,
           first_chapter, source, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            uid,
            novel_id,
            f.kind,
            f.name,
            to_json(f.names, "{}"),
            to_json(&f.aliases, "[]"),
            to_json(f.summaries, "{}"),
            f.rank,
            first_chapter,
            f.source,
            super::db::now_ms()
        ],
    )?;
    Ok(uid)
}

fn kind_of(kind: &str) -> &'static str {
    let k = kind.trim().to_ascii_lowercase();
    KINDS.iter().find(|x| **x == k).copied().unwrap_or("other")
}

/// Saves an entry the user wrote or edited; it is theirs from now on.
pub fn save(conn: &Connection, novel_id: i64, input: EntryInput) -> Result<Entry, AiError> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AiError::Other("wiki-name-empty".into()));
    }
    let kind = kind_of(&input.kind);
    let names = clean_names(input.names);
    let aliases = clean_list(input.aliases, &name);
    let summaries = clean_names(input.summaries);
    let fields = Fields {
        kind,
        name: &name,
        names: &names,
        aliases: &aliases,
        summaries: &summaries,
        rank: input.rank.filter(|_| kind == "level"),
        source: "user",
    };
    let id: Option<i64> = match input.uid.as_deref() {
        Some(uid) => Some(
            conn.query_row(
                "select id from wiki_entries where uid = ?1 and novel_id = ?2",
                params![uid, novel_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| AiError::Other("wiki-entry-missing".into()))?,
        ),
        // A name used before (even by a removed entry) brings that entry back.
        None => conn
            .query_row(
                "select id from wiki_entries where novel_id = ?1 and name = ?2",
                params![novel_id, name],
                |r| r.get(0),
            )
            .optional()?,
    };
    let taken: Option<i64> = conn
        .query_row(
            "select id from wiki_entries where novel_id = ?1 and name = ?2",
            params![novel_id, name],
            |r| r.get(0),
        )
        .optional()?;
    if taken.is_some() && taken != id {
        return Err(AiError::Other("wiki-name-taken".into()));
    }
    let uid = match id {
        Some(id) => {
            write_entry(conn, id, &fields)?;
            conn.query_row("select uid from wiki_entries where id = ?1", [id], |r| {
                r.get(0)
            })?
        }
        None => insert_entry(conn, novel_id, &fields, None)?,
    };
    Ok(by_uid(conn, &uid)?.expect("saved entry"))
}

/// Removes an entry and its relations; the removal is synced, and the AI does not add
/// the name back.
pub fn remove(conn: &Connection, uid: &str) -> rusqlite::Result<()> {
    let now = super::db::now_ms();
    conn.execute(
        "update wiki_entries set deleted = 1, updated_at = ?2 where uid = ?1",
        params![uid, now],
    )?;
    conn.execute(
        "update wiki_relations set deleted = 1, updated_at = ?2 where from_uid = ?1 or to_uid = ?1",
        params![uid, now],
    )?;
    Ok(())
}

/// Adds (or, from the user, brings back) a relation; returns its uid, or None when the
/// user removed it before and the AI asks again. `kind` is a relation type, or free
/// words written in `lang`.
#[allow(clippy::too_many_arguments)]
fn relate(
    conn: &Connection,
    novel_id: i64,
    from_uid: &str,
    to_uid: &str,
    kind: &str,
    lang: &str,
    note: &str,
    first_chapter: Option<i64>,
    by_user: bool,
) -> rusqlite::Result<Option<String>> {
    let typed = rel_type(kind);
    let kind = typed.unwrap_or(kind.trim());
    let note = note.trim();
    // A free relation is found by its words in any language.
    let kind: String = match typed {
        Some(_) => kind.to_string(),
        None => conn
            .query_row(
                "select kind from wiki_relations where from_uid = ?1 and to_uid = ?2 and (kind = ?3
                   or exists (select 1 from json_each(labels) l where l.value = ?3))",
                params![from_uid, to_uid, kind],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| kind.to_string()),
    };
    let label = typed.is_none().then(|| kind.clone());
    let existing: Option<(String, bool)> = conn
        .query_row(
            "select uid, deleted from wiki_relations where from_uid = ?1 and to_uid = ?2 and kind = ?3",
            params![from_uid, to_uid, &kind],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let now = super::db::now_ms();
    match existing {
        Some((_, true)) if !by_user => Ok(None),
        Some((uid, _)) => {
            conn.execute(
                "update wiki_relations set deleted = 0, note = case when ?2 = '' then note else ?2 end,
                   updated_at = ?3 where uid = ?1",
                params![uid, note, now],
            )?;
            Ok(Some(uid))
        }
        None => {
            let uid = new_uid(conn)?;
            let mut labels = BTreeMap::new();
            if let Some(label) = label {
                labels.insert(lang.to_string(), label);
            }
            conn.execute(
                "insert into wiki_relations (uid, novel_id, from_uid, to_uid, kind, labels, note, first_chapter, updated_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![uid, novel_id, from_uid, to_uid, kind, to_json(&labels, "{}"), note, first_chapter, now],
            )?;
            Ok(Some(uid))
        }
    }
}

/// A relation the user added between two entries.
pub fn add_relation(
    conn: &Connection,
    novel_id: i64,
    from_uid: &str,
    to_uid: &str,
    kind: &str,
    lang: &str,
    note: &str,
) -> Result<String, AiError> {
    if kind.trim().is_empty() || from_uid == to_uid {
        return Err(AiError::Other("wiki-relation-invalid".into()));
    }
    for uid in [from_uid, to_uid] {
        if by_uid(conn, uid)?.is_none() {
            return Err(AiError::Other("wiki-entry-missing".into()));
        }
    }
    Ok(relate(
        conn, novel_id, from_uid, to_uid, kind, lang, note, None, true,
    )?
    .expect("by user"))
}

pub fn remove_relation(conn: &Connection, uid: &str) -> rusqlite::Result<()> {
    conn.execute(
        "update wiki_relations set deleted = 1, updated_at = ?2 where uid = ?1",
        params![uid, super::db::now_ms()],
    )?;
    Ok(())
}

/* ---------- What the AI writes ---------- */

#[derive(Debug, Deserialize)]
struct AiEntry {
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    translated: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    rank: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AiRelation {
    from: String,
    to: String,
    relation: String,
    #[serde(default)]
    note: String,
}

/// Adds what the AI found about an entry: a new entry, or the new facts of a known one.
/// The user's own entries only gain aliases and missing names; a removed one is left
/// alone. Returns the entry's uid when there is one to relate. A description written
/// from a chapter is also kept as of that chapter (`wiki_notes`), so the reader can show
/// what was known by then.
fn upsert_ai(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    chapter: Option<i64>,
    item: AiEntry,
) -> rusqlite::Result<Option<String>> {
    let summary = item.summary.trim().to_string();
    let uid = upsert_ai_entry(conn, novel_id, lang, chapter, item)?;
    if let (Some(uid), Some(chapter)) = (&uid, chapter) {
        let kept = by_uid(conn, uid)?.and_then(|e| e.summaries.get(lang).cloned());
        if !summary.is_empty() && kept.as_deref() == Some(summary.as_str()) {
            conn.execute(
                "insert or replace into wiki_notes (entry_uid, lang, chapter, summary) values (?1, ?2, ?3, ?4)",
                params![uid, lang, chapter, summary],
            )?;
        }
    }
    Ok(uid)
}

fn upsert_ai_entry(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    chapter: Option<i64>,
    item: AiEntry,
) -> rusqlite::Result<Option<String>> {
    let name = item.name.trim().to_string();
    if name.is_empty() {
        return Ok(None);
    }
    let translated = item.translated.trim().to_string();
    match find(conn, novel_id, &name)? {
        Some((_, _, true)) => Ok(None),
        Some((id, e, false)) => {
            let mut names = e.names.clone();
            let mut aliases = e.aliases.clone();
            aliases.extend(item.aliases);
            if name != e.name {
                aliases.push(name.clone());
            }
            let aliases = clean_list(aliases, &e.name);
            let mine = e.source == "user";
            if !translated.is_empty() && (!mine || !names.contains_key(lang)) {
                names.insert(lang.to_string(), translated);
            }
            let mut summaries = e.summaries.clone();
            let summary = item.summary.trim();
            if !summary.is_empty() && (!mine || !summaries.contains_key(lang)) {
                summaries.insert(lang.to_string(), summary.to_string());
            }
            let kind = if mine || item.kind.trim().is_empty() {
                e.kind.as_str()
            } else {
                kind_of(&item.kind)
            };
            let rank = if kind != "level" {
                None
            } else if mine {
                e.rank.or(item.rank)
            } else {
                item.rank.or(e.rank)
            };
            if names != e.names
                || aliases != e.aliases
                || summaries != e.summaries
                || kind != e.kind
                || rank != e.rank
            {
                let f = Fields {
                    kind,
                    name: &e.name,
                    names: &names,
                    aliases: &aliases,
                    summaries: &summaries,
                    rank,
                    source: &e.source,
                };
                write_entry(conn, id, &f)?;
            }
            Ok(Some(e.uid))
        }
        None => {
            let mut names = BTreeMap::new();
            if !translated.is_empty() {
                names.insert(lang.to_string(), translated);
            }
            let mut summaries = BTreeMap::new();
            if !item.summary.trim().is_empty() {
                summaries.insert(lang.to_string(), item.summary.trim().to_string());
            }
            let aliases = clean_list(item.aliases, &name);
            let kind = kind_of(&item.kind);
            let f = Fields {
                kind,
                name: &name,
                names: &names,
                aliases: &aliases,
                summaries: &summaries,
                rank: item.rank.filter(|_| kind == "level"),
                source: "ai",
            };
            Ok(Some(insert_entry(conn, novel_id, &f, chapter)?))
        }
    }
}

fn relate_ai(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    chapter: Option<i64>,
    item: &AiRelation,
) -> rusqlite::Result<Result<String, &'static str>> {
    let from = find(conn, novel_id, &item.from)?;
    let to = find(conn, novel_id, &item.to)?;
    let (Some((_, from, false)), Some((_, to, false))) = (from, to) else {
        return Ok(Err("add both entries with wiki_upsert first"));
    };
    if from.uid == to.uid || item.relation.trim().is_empty() {
        return Ok(Err("a relation needs two different entries and a relation"));
    }
    Ok(relate(
        conn,
        novel_id,
        &from.uid,
        &to.uid,
        &item.relation,
        lang,
        &item.note,
        chapter,
        false,
    )?
    .ok_or("the user removed this relation"))
}

/* ---------- Search ---------- */

/// Entries matching `query` by name, alias or summary.
pub fn search(
    conn: &Connection,
    novel_id: i64,
    query: &str,
    limit: usize,
) -> Result<Vec<Entry>, AiError> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(entries(conn, novel_id)?.into_iter().take(limit).collect());
    }
    let mut stmt = conn.prepare(&format!(
        "select {ENTRY_COLS} from wiki_entries e where e.novel_id = ?1 and e.deleted = 0 and (
           instr(lower(e.name), ?2) > 0 or instr(lower(e.aliases), ?2) > 0
           or instr(lower(e.names), ?2) > 0 or instr(lower(e.summaries), ?2) > 0)
         order by lower(e.name) = ?2 desc, instr(lower(e.name), ?2) = 0, e.name limit ?3"
    ))?;
    let out: Vec<Entry> = stmt
        .query_map(params![novel_id, q, limit as i64], entry_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(out)
}

/// Entries named in `text` (by name or alias), the most mentioned first.
fn mentioned(conn: &Connection, novel_id: i64, text: &str) -> rusqlite::Result<Vec<Entry>> {
    let all = entries(conn, novel_id)?;
    let names: Vec<Vec<&str>> = all
        .iter()
        .map(|e| {
            std::iter::once(e.name.as_str())
                .chain(e.aliases.iter().map(String::as_str))
                .collect()
        })
        .collect();
    let hits = super::names::Finder::new(&names).hits(text);
    let mut found: Vec<(usize, Entry)> = hits
        .into_iter()
        .zip(all)
        .filter(|(hits, _)| *hits > 0)
        .collect();
    found.sort_by_key(|(hits, _)| std::cmp::Reverse(*hits));
    Ok(found.into_iter().map(|(_, e)| e).collect())
}

fn head(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// The wiki part of a translation prompt for chapter `text`: the entries it names,
/// with their relations. Empty when the wiki has nothing for it.
pub fn context(
    conn: &Connection,
    novel_id: i64,
    text: &str,
    lang: &str,
) -> rusqlite::Result<String> {
    let mut picked = mentioned(conn, novel_id, text)?;
    picked.truncate(CONTEXT_ENTRIES);
    if picked.is_empty() {
        return Ok(String::new());
    }
    let label = |e: &Entry| match e.names.get(lang) {
        Some(n) => format!("{} → {n}", e.name),
        None => e.name.clone(),
    };
    let mut s = String::from(
        "\nThe novel's wiki (use these names, forms of address and relationships consistently):\n",
    );
    for e in &picked {
        s.push_str(&format!("- {} ({})", label(e), e.kind));
        if !e.aliases.is_empty() {
            s.push_str(&format!("; also called {}", e.aliases.join(", ")));
        }
        if let Some(rank) = e.rank {
            s.push_str(&format!("; rank {rank}"));
        }
        let summary = e.summary_in(lang);
        if !summary.is_empty() {
            s.push_str(&format!(": {}", head(summary, 200)));
        }
        s.push('\n');
    }
    let names: BTreeMap<&str, &Entry> = picked.iter().map(|e| (e.uid.as_str(), e)).collect();
    let rels: Vec<String> = relations(conn, novel_id)?
        .into_iter()
        .filter_map(|r| {
            let a = names.get(r.from_uid.as_str())?;
            let b = names.get(r.to_uid.as_str())?;
            Some(format!(
                "- {} — {} — {}\n",
                a.name,
                rel_words(&r, lang),
                b.name
            ))
        })
        .take(CONTEXT_ENTRIES)
        .collect();
    if !rels.is_empty() {
        s.push_str("Relationships:\n");
        s.extend(rels);
    }
    Ok(s)
}

/* ---------- Learning from a chapter ---------- */

fn tools(lang: &str) -> Value {
    let lang = super::translate::lang_name(lang);
    json!([
        {
            "type": "function",
            "function": {
                "name": "wiki_search",
                "description": "Find entries of this novel's wiki by name, alias or description.",
                "parameters": {
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "wiki_upsert",
                "description": "Add an entry, or add new facts to an existing one (matched by its original name or an alias).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "The name exactly as written in the original text." },
                        "kind": { "type": "string", "enum": KINDS },
                        "translated": { "type": "string", "description": format!("The name as used in the {lang} translation.") },
                        "aliases": { "type": "array", "items": { "type": "string" }, "description": "Other names or titles in the original text." },
                        "summary": { "type": "string", "description": format!("In {lang}, at most three sentences: who or what it is, as known so far. Write the whole summary, old facts included.") },
                        "rank": { "type": "integer", "description": "Only for kind level: its place in its ladder of ranks or cultivation realms, 1 for the lowest." }
                    },
                    "required": ["name", "kind"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "wiki_relate",
                "description": "Record a relationship between two entries, by their original names.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "string" },
                        "to": { "type": "string" },
                        "relation": { "type": "string", "description": format!("What `from` is to `to`: one of {} (located_in for a place or faction inside a bigger place, has_level for a character's current rank or realm, has_skill for a technique or ability a character knows). Only when none fits, a few words in {lang}.", REL_TYPES.join(", ")) },
                        "note": { "type": "string" }
                    },
                    "required": ["from", "to", "relation"]
                }
            }
        }
    ])
}

fn learn_rules(lang: &str) -> String {
    format!(
        "You keep the wiki of a novel being translated into {}. From the chapter below, record the characters, places, factions or sects, important items, levels (ranks, cultivation realms, each with its rank number), skills (techniques, martial arts, spells, abilities) and other special terms that matter to the story, and the relationships: between characters, with factions, a character's current level, the skills a character has, and which place lies in which (the map). Skip passers-by and common words. Use the name exactly as written in the original text, and the name the translation uses. Only add what the chapter shows; do not guess.",
        super::translate::lang_name(lang)
    )
}

fn learn_prompt(lang: &str) -> String {
    format!(
        "{} Use the tools: wiki_search to check what is known, wiki_upsert to add or update an entry, wiki_relate for relationships (add both entries first). When the wiki is up to date, reply with DONE.",
        learn_rules(lang)
    )
}

/// The JSON object the AI answers with when it does not use the tools.
fn json_shape(lang: &str) -> String {
    format!(
        "{{\"entries\": [{{\"name\": original name, \"kind\": one of {}, \"translated\": name in the translation, \"aliases\": [other names], \"summary\": at most three sentences in {}, \"rank\": number for a level only}}], \"relations\": [{{\"from\": original name, \"to\": original name, \"relation\": one of {} (else a few words in {}), \"note\": \"\"}}]}}",
        KINDS.join(", "),
        super::translate::lang_name(lang),
        REL_TYPES.join(", "),
        super::translate::lang_name(lang),
    )
}

fn json_prompt(lang: &str) -> String {
    format!(
        "{} Reply with only one JSON object and nothing else: {}",
        learn_rules(lang),
        json_shape(lang)
    )
}

/// What a translation prompt adds so the same request also feeds the wiki: after the
/// translated text, `mark` on its own line and the facts of that part as JSON.
pub fn extract_prompt(lang: &str, mark: &str) -> String {
    format!(
        "\n\nAlso keep the novel's wiki. {} After the translated text, write a line with only {mark}, then one JSON object and nothing else: {}, with two more lists in it: {}. When the text shows nothing worth keeping, write {{\"entries\": [], \"relations\": []}}.",
        learn_rules(lang),
        json_shape(lang),
        super::cast::shape(lang)
    )
}

/// The user message: what the wiki knows of the chapter, then its text.
fn learn_input(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    original: &str,
    translated: &str,
) -> rusqlite::Result<String> {
    let known = mentioned(conn, novel_id, original)?;
    let mut s = String::new();
    if !known.is_empty() {
        s.push_str("Already in the wiki:\n");
        for e in known.iter().take(CONTEXT_ENTRIES) {
            s.push_str(&format!(
                "- {} ({}){}: {}\n",
                e.name,
                e.kind,
                e.names
                    .get(lang)
                    .map(|n| format!(" → {n}"))
                    .unwrap_or_default(),
                head(e.summary_in(lang), 160)
            ));
        }
        s.push('\n');
    }
    s.push_str("Original chapter:\n");
    s.push_str(&head(original, ORIGINAL_CHARS));
    if !translated.is_empty() {
        s.push_str("\n\nIts translation:\n");
        s.push_str(&head(translated, TRANSLATED_CHARS));
    }
    Ok(s)
}

/// Whether the API or model does not take tools, so the JSON way is used instead.
fn tools_refused(e: &AiError) -> bool {
    match e {
        AiError::Http(400 | 404 | 405 | 422 | 501, _) => true,
        AiError::Other(m) => m == "tools-unsupported",
        _ => false,
    }
}

/// Runs one tool call and returns its answer for the model.
#[allow(clippy::too_many_arguments)]
fn run_tool(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    chapter: Option<i64>,
    name: &str,
    args: &str,
    changes: &mut usize,
) -> rusqlite::Result<Value> {
    let args: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    Ok(match name {
        "wiki_search" => {
            let q = args
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or_default();
            match search(conn, novel_id, q, 8) {
                Ok(found) => json!(found
                    .iter()
                    .map(|e| json!({
                        "name": e.name, "kind": e.kind, "translated": e.names.get(lang),
                        "aliases": e.aliases, "summary": e.summary_in(lang), "rank": e.rank,
                    }))
                    .collect::<Vec<_>>()),
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        "wiki_upsert" => match serde_json::from_value::<AiEntry>(args) {
            Ok(item) => match upsert_ai(conn, novel_id, lang, chapter, item)? {
                Some(_) => {
                    *changes += 1;
                    json!({ "ok": true })
                }
                None => {
                    json!({ "ok": false, "error": "the user removed this entry; leave it out" })
                }
            },
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        "wiki_relate" => match serde_json::from_value::<AiRelation>(args) {
            Ok(item) => match relate_ai(conn, novel_id, lang, chapter, &item)? {
                Ok(_) => {
                    *changes += 1;
                    json!({ "ok": true })
                }
                Err(why) => json!({ "ok": false, "error": why }),
            },
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
        other => json!({ "ok": false, "error": format!("unknown tool {other}") }),
    })
}

#[derive(Debug, Default, Deserialize)]
struct AiAnswer {
    #[serde(default)]
    entries: Vec<AiEntry>,
    #[serde(default)]
    relations: Vec<AiRelation>,
}

/// Applies a JSON answer (the object anywhere in the reply); returns the changes made.
fn apply_json(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    chapter: Option<i64>,
    reply: &str,
) -> rusqlite::Result<usize> {
    let Some(answer) = parse_object::<AiAnswer>(reply) else {
        return Ok(0);
    };
    let mut changes = 0;
    for item in answer.entries {
        if upsert_ai(conn, novel_id, lang, chapter, item)?.is_some() {
            changes += 1;
        }
    }
    for item in &answer.relations {
        if relate_ai(conn, novel_id, lang, chapter, item)?.is_ok() {
            changes += 1;
        }
    }
    Ok(changes)
}

/// Reads one translated chapter into the book's wiki (once per chapter); returns the
/// changes made.
pub fn learn(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    chapter_id: i64,
    lang: &str,
    original: &str,
    translated: &str,
) -> Result<usize, AiError> {
    let done: bool = conn.query_row(
        "select exists (select 1 from wiki_chapters where chapter_id = ?1)",
        [chapter_id],
        |r| r.get(0),
    )?;
    if done || original.trim().is_empty() {
        return Ok(0);
    }
    let chapter: Option<i64> = conn
        .query_row(
            "select number from chapters where id = ?1",
            [chapter_id],
            |r| r.get(0),
        )
        .optional()?;
    let input = learn_input(conn, novel_id, lang, original, translated)?;
    let tools = tools(lang);
    let mut messages = vec![
        json!({ "role": "system", "content": learn_prompt(lang) }),
        json!({ "role": "user", "content": input }),
    ];
    let mut changes = 0;
    for round in 0..MAX_ROUNDS {
        let msg = match llm.chat_tools(&messages, &tools) {
            Ok(m) => m,
            Err(e) if round == 0 && tools_refused(&e) => {
                let reply = llm.chat(&json_prompt(lang), &input)?;
                changes += apply_json(conn, novel_id, lang, chapter, &reply)?;
                break;
            }
            Err(e) => return Err(e),
        };
        let calls = msg
            .get("tool_calls")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        if calls.is_empty() {
            // A model that answers in JSON instead of calling the tools still counts.
            if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                changes += apply_json(conn, novel_id, lang, chapter, text)?;
            }
            break;
        }
        messages.push(json!({
            "role": "assistant",
            "content": msg.get("content").cloned().unwrap_or(Value::Null),
            "tool_calls": calls,
        }));
        for call in &calls {
            let f = call.get("function").cloned().unwrap_or(Value::Null);
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or_default();
            let args = f.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
            let answer = run_tool(conn, novel_id, lang, chapter, name, args, &mut changes)?;
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.get("id").cloned().unwrap_or(Value::Null),
                "content": answer.to_string(),
            }));
        }
    }
    conn.execute(
        "insert or replace into wiki_chapters (chapter_id, done_at) values (?1, ?2)",
        params![chapter_id, super::db::now_ms()],
    )?;
    Ok(changes)
}

/// Whether any of the facts the translation requests brought reads as JSON.
pub fn has_found(found: &[String]) -> bool {
    found.iter().any(|f| parse_object::<AiAnswer>(f).is_some())
}

/// The wiki queue's turn (queue.rs): every book whose wiki learns gets the facts kept
/// for its chapters not read in yet. Returns (book, changes) for each book done; a
/// book's failure is kept for its wiki screen.
pub fn learn_pending(conn: &Connection) -> rusqlite::Result<Vec<(i64, usize)>> {
    let mut stmt = conn.prepare(
        "select distinct c.novel_id from chapter_wiki_facts f join chapters c on c.id = f.chapter_id
         where not exists (select 1 from wiki_chapters w where w.chapter_id = f.chapter_id)",
    )?;
    let books: Vec<i64> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut done = Vec::new();
    for novel_id in books {
        if !auto(conn, novel_id)? {
            continue;
        }
        match learn_kept(conn, novel_id) {
            Ok(n) => {
                set_error(conn, novel_id, None)?;
                done.push((novel_id, n));
            }
            Err(e) => set_error(conn, novel_id, Some(&e.to_string()))?,
        }
    }
    Ok(done)
}

/// Keeps what the translation requests found in a chapter, for when the wiki learns.
pub fn keep_found(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    found: &[String],
) -> rusqlite::Result<()> {
    if found.is_empty() {
        return Ok(());
    }
    conn.execute(
        "insert or replace into chapter_wiki_facts (chapter_id, lang, facts, created_at)
         values (?1, ?2, ?3, ?4)",
        params![chapter_id, lang, to_json(&found, "[]"), super::db::now_ms()],
    )?;
    Ok(())
}

/// Fills the wiki from the facts kept for the book's chapters not read into it yet,
/// in reading order, with no request; returns the changes made.
pub fn learn_kept(conn: &Connection, novel_id: i64) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare(
        "select f.chapter_id, f.lang, f.facts from chapter_wiki_facts f
         join chapters c on c.id = f.chapter_id
         where c.novel_id = ?1
           and not exists (select 1 from wiki_chapters w where w.chapter_id = f.chapter_id)
         order by c.number, f.created_at desc",
    )?;
    let kept: Vec<(i64, String, String)> = stmt
        .query_map([novel_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut changes = 0;
    for (chapter_id, lang, facts) in kept {
        let found: Vec<String> = serde_json::from_str(&facts).unwrap_or_default();
        changes += learn_found(conn, novel_id, chapter_id, &lang, &found)?.unwrap_or(0);
    }
    Ok(changes)
}

/// Puts in the wiki what the translation requests found in a chapter (`extract_prompt`),
/// with no request of its own. None when none of `found` reads as JSON, so the chapter
/// is learnt the usual way.
pub fn learn_found(
    conn: &Connection,
    novel_id: i64,
    chapter_id: i64,
    lang: &str,
    found: &[String],
) -> rusqlite::Result<Option<usize>> {
    let answers: Vec<AiAnswer> = found.iter().filter_map(|f| parse_object(f)).collect();
    if answers.is_empty() {
        return Ok(None);
    }
    let done: bool = conn.query_row(
        "select exists (select 1 from wiki_chapters where chapter_id = ?1)",
        [chapter_id],
        |r| r.get(0),
    )?;
    if done {
        return Ok(Some(0));
    }
    let chapter: Option<i64> = conn
        .query_row(
            "select number from chapters where id = ?1",
            [chapter_id],
            |r| r.get(0),
        )
        .optional()?;
    let mut changes = 0;
    // All entries first, so a relation can name an entry from a later part.
    let (entries, relations): (Vec<_>, Vec<_>) = answers
        .into_iter()
        .map(|a| (a.entries, a.relations))
        .unzip();
    for item in entries.into_iter().flatten() {
        if upsert_ai(conn, novel_id, lang, chapter, item)?.is_some() {
            changes += 1;
        }
    }
    for item in relations.iter().flatten() {
        if relate_ai(conn, novel_id, lang, chapter, item)?.is_ok() {
            changes += 1;
        }
    }
    conn.execute(
        "insert or replace into wiki_chapters (chapter_id, done_at) values (?1, ?2)",
        params![chapter_id, super::db::now_ms()],
    )?;
    Ok(Some(changes))
}

/// `learn` as the translation graph runs it: from what the translation found when
/// there is something, else with requests of its own. A failure is kept for the wiki
/// screen and never fails the translation.
#[allow(clippy::too_many_arguments)]
pub fn learn_quietly(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    chapter_id: i64,
    lang: &str,
    original: &str,
    translated: &str,
    found: &[String],
) -> rusqlite::Result<()> {
    if !auto(conn, novel_id)? {
        return Ok(());
    }
    let learnt = match learn_found(conn, novel_id, chapter_id, lang, found) {
        Ok(None) => learn(conn, llm, novel_id, chapter_id, lang, original, translated),
        Ok(Some(n)) => Ok(n),
        Err(e) => Err(e.into()),
    };
    match learnt {
        Ok(_) => set_error(conn, novel_id, None),
        Err(e) => set_error(conn, novel_id, Some(&e.to_string())),
    }
}

/* ---------- As of a chapter ---------- */

/// What the wiki knew by chapter `number` (reading order, from 1): the entries and
/// relations met by then, each entry's description as it was written then (when the
/// AI kept one per chapter; an entry it never wrote about keeps its own words).
pub fn at_chapter(conn: &Connection, novel_id: i64, number: i64) -> rusqlite::Result<Wiki> {
    let known = |first: Option<i64>| first.is_none_or(|c| c <= number);
    let mut notes = conn.prepare(
        "select lang, summary, chapter <= ?2 from wiki_notes where entry_uid = ?1 order by chapter",
    )?;
    let mut found = Vec::new();
    for mut e in entries(conn, novel_id)?
        .into_iter()
        .filter(|e| known(e.first_chapter))
    {
        let rows: Vec<(String, String, bool)> = notes
            .query_map(params![e.uid, number], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        if !rows.is_empty() {
            e.summaries = rows
                .into_iter()
                .filter(|(_, _, by_then)| *by_then)
                .map(|(lang, text, _)| (lang, text))
                .collect();
        }
        found.push(e);
    }
    let uids: std::collections::HashSet<&str> = found.iter().map(|e| e.uid.as_str()).collect();
    let relations = relations(conn, novel_id)?
        .into_iter()
        .filter(|r| {
            known(r.first_chapter)
                && uids.contains(r.from_uid.as_str())
                && uids.contains(r.to_uid.as_str())
        })
        .collect();
    let (auto, last_error) = book_state(conn, novel_id)?;
    Ok(Wiki {
        entries: found,
        relations,
        auto,
        last_error,
    })
}

/* ---------- Translating the wiki ---------- */

/// The language the book is written in: the one the LLM found, else the import's guess.
pub fn book_lang(conn: &Connection, novel_id: i64) -> rusqlite::Result<String> {
    conn.query_row(
        "select coalesce((select source_lang from ai_books where novel_id = ?1), original_lang)
         from novels where id = ?1",
        [novel_id],
        |r| r.get(0),
    )
}

/// What `lang` is still missing: entries without a name or description in it, and free
/// relations without words in it. `RAW` asks for descriptions in the book's language.
pub fn missing(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
) -> rusqlite::Result<(Vec<Entry>, Vec<Relation>)> {
    let raw = lang == RAW || lang == book_lang(conn, novel_id)?;
    let entries = entries(conn, novel_id)?
        .into_iter()
        .filter(|e| {
            let no_name = !raw && !e.names.contains_key(lang);
            let no_summary = !e.summaries.is_empty() && !e.summaries.contains_key(lang);
            no_name || no_summary
        })
        .collect();
    let relations = if raw {
        Vec::new()
    } else {
        relations(conn, novel_id)?
            .into_iter()
            .filter(|r| rel_type(&r.kind).is_none() && !r.labels.contains_key(lang))
            .collect()
    };
    Ok((entries, relations))
}

#[derive(Debug, Default, Deserialize)]
struct Translated {
    #[serde(default)]
    entries: Vec<TranslatedEntry>,
    #[serde(default)]
    relations: Vec<TranslatedRelation>,
}

#[derive(Debug, Deserialize)]
struct TranslatedEntry {
    #[serde(deserialize_with = "loose_id")]
    id: usize,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
}

#[derive(Debug, Deserialize)]
struct TranslatedRelation {
    #[serde(deserialize_with = "loose_id")]
    id: usize,
    #[serde(default)]
    label: String,
}

/// An id as models write it: 3 or "3".
fn loose_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    match Value::deserialize(d)? {
        Value::Number(n) => n.as_u64().map(|n| n as usize),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .ok_or_else(|| serde::de::Error::custom("id"))
}

/// Fills in what `lang` is missing (see `missing`) with the LLM. Each section (a kind of
/// entry, the relations) goes in batches of about `TRANSLATE_CHARS`, `TRANSLATE_PARALLEL`
/// requests at a time; a batch the model can't answer whole is asked again in halves.
/// Only adds: a name or description already in `lang` stays as it is. Returns how many
/// entries and relations changed. A failed batch is an error only when nothing at all
/// could be filled in, so the user hears of it instead of seeing nothing change; what
/// is still missing goes in on the next try.
pub fn translate_missing(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    lang: &str,
) -> Result<usize, AiError> {
    let book = book_lang(conn, novel_id)?;
    let target = if lang == RAW { book.as_str() } else { lang };
    let raw = lang == RAW || lang == book;
    let (entries, relations) = missing(conn, novel_id, lang)?;
    let system = format!(
        "You translate the wiki of a novel into {}. For each entry, give its name as a translation of this novel would write it (keep established forms; Sino-Vietnamese readings for Chinese names in Vietnamese) and its description in {0}; for each relation, its words in {0}. Fill only the fields asked for. Reply with only one JSON object and nothing else: {{\"entries\": [{{\"id\": number, \"name\": \"...\", \"summary\": \"...\"}}], \"relations\": [{{\"id\": number, \"label\": \"...\"}}]}}",
        super::translate::lang_name(target)
    );

    // The sections: one per kind of entry, then the relations.
    let mut kinds: BTreeMap<&str, Vec<(usize, Value)>> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        let mut v = json!({ "original_name": e.name, "kind": e.kind });
        if !raw && !e.names.contains_key(lang) {
            v["name"] = json!("?");
        }
        if !e.summaries.is_empty() && !e.summaries.contains_key(lang) {
            v["summary"] = json!("?");
            v["summary_now"] = json!(e.summary_in(lang));
        }
        if let Some(n) = e.names.values().next() {
            v["known_as"] = json!(n);
        }
        kinds.entry(e.kind.as_str()).or_default().push((i, v));
    }
    let rels: Vec<(usize, Value)> = relations
        .iter()
        .enumerate()
        .map(|(i, r)| {
            (
                i,
                json!({ "label": "?", "words_now": r.labels.values().next().unwrap_or(&r.kind) }),
            )
        })
        .collect();
    let mut jobs: Vec<(Section, Vec<(usize, Value)>)> = Vec::new();
    for items in kinds.into_values() {
        jobs.extend(batches(items).into_iter().map(|b| (Section::Entries, b)));
    }
    jobs.extend(batches(rels).into_iter().map(|b| (Section::Relations, b)));

    // The requests, a few at a time; a failure that would fail every request stops the rest.
    let answers: Vec<std::sync::Mutex<Option<Translated>>> =
        jobs.iter().map(|_| Default::default()).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed: std::sync::Mutex<Option<AiError>> = Default::default();
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        for _ in 0..TRANSLATE_PARALLEL.min(jobs.len()) {
            s.spawn(|| loop {
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some((section, items)) = jobs.get(i) else {
                    return;
                };
                let (answer, error) = ask_batch(llm, &system, *section, items);
                *answers[i].lock().unwrap() = Some(answer);
                if let Some(e) = error {
                    if matches!(e, AiError::Auth(_) | AiError::NotSet | AiError::Network(_)) {
                        stop.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    failed.lock().unwrap().get_or_insert(e);
                }
            });
        }
    });

    // Written here, one at a time: the connection stays on this thread.
    let mut changed = 0;
    for answer in answers.into_iter().filter_map(|a| a.into_inner().unwrap()) {
        for t in answer.entries {
            let Some(e) = entries.get(t.id) else { continue };
            let mut names = e.names.clone();
            let mut summaries = e.summaries.clone();
            if !raw && !t.name.trim().is_empty() {
                names
                    .entry(lang.to_string())
                    .or_insert_with(|| t.name.trim().to_string());
            }
            if !e.summaries.is_empty() && !t.summary.trim().is_empty() {
                summaries
                    .entry(lang.to_string())
                    .or_insert_with(|| t.summary.trim().to_string());
            }
            if names != e.names || summaries != e.summaries {
                conn.execute(
                    "update wiki_entries set names = ?2, summaries = ?3, updated_at = ?4 where uid = ?1",
                    params![e.uid, to_json(&names, "{}"), to_json(&summaries, "{}"), super::db::now_ms()],
                )?;
                changed += 1;
            }
        }
        for t in answer.relations {
            let (Some(r), label) = (relations.get(t.id), t.label.trim()) else {
                continue;
            };
            if label.is_empty() {
                continue;
            }
            let mut labels = r.labels.clone();
            labels.insert(lang.to_string(), label.to_string());
            conn.execute(
                "update wiki_relations set labels = ?2, updated_at = ?3 where uid = ?1",
                params![r.uid, to_json(&labels, "{}"), super::db::now_ms()],
            )?;
            changed += 1;
        }
    }
    match failed.into_inner().unwrap() {
        Some(e) if changed == 0 => Err(e),
        _ => Ok(changed),
    }
}

/// Which list of a `translate_missing` request the items go in.
#[derive(Debug, Clone, Copy)]
enum Section {
    Entries,
    Relations,
}

/// `items` in batches of about `TRANSLATE_CHARS` and at most `TRANSLATE_BATCH`, in
/// order; one longer than that goes alone.
fn batches(items: Vec<(usize, Value)>) -> Vec<Vec<(usize, Value)>> {
    let mut out: Vec<Vec<(usize, Value)>> = Vec::new();
    let mut size = 0;
    for item in items {
        let len = item.1.to_string().chars().count();
        match out.last_mut() {
            Some(b) if b.len() < TRANSLATE_BATCH && size + len <= TRANSLATE_CHARS => {
                b.push(item);
                size += len;
            }
            _ => {
                out.push(vec![item]);
                size = len;
            }
        }
    }
    out
}

/// Asks the LLM for one batch; the answer's ids are those of `items` (their place in
/// what `missing` found). A reply that can't be read, or a request or reply too long
/// for the model, is asked again in two halves, so a long batch keeps what can be had.
/// Returns what was translated and the first failure, if any.
fn ask_batch(
    llm: &dyn Chat,
    system: &str,
    section: Section,
    items: &[(usize, Value)],
) -> (Translated, Option<AiError>) {
    let list: Vec<Value> = items
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            let mut v = v.clone();
            v["id"] = json!(i);
            v
        })
        .collect();
    let user = match section {
        Section::Entries => json!({ "entries": list }),
        Section::Relations => json!({ "relations": list }),
    };
    let error = match llm.chat(system, &user.to_string()) {
        Ok(reply) => match parse_object::<Translated>(&reply) {
            Some(mut t) => {
                let id = |i: usize| items.get(i).map(|(id, _)| *id);
                t.entries = t
                    .entries
                    .into_iter()
                    .filter_map(|e| Some(TranslatedEntry { id: id(e.id)?, ..e }))
                    .collect();
                t.relations = t
                    .relations
                    .into_iter()
                    .filter_map(|r| Some(TranslatedRelation { id: id(r.id)?, ..r }))
                    .collect();
                return (t, None);
            }
            None => AiError::Other(format!(
                "unreadable reply: {}",
                reply.chars().take(200).collect::<String>()
            )),
        },
        Err(e) => e,
    };
    if items.len() < 2 || !too_big(&error) {
        return (Translated::default(), Some(error));
    }
    let (a, b) = items.split_at(items.len() / 2);
    let (mut first, e1) = ask_batch(llm, system, section, a);
    let (second, e2) = ask_batch(llm, system, section, b);
    first.entries.extend(second.entries);
    first.relations.extend(second.relations);
    (first, e1.or(e2))
}

/// Whether a failed request may go through in smaller pieces: a reply cut short or
/// unreadable, a slow answer, or the API saying the request is too long.
fn too_big(e: &AiError) -> bool {
    match e {
        AiError::Other(_) | AiError::Empty | AiError::Timeout => true,
        AiError::Http(413, _) => true,
        AiError::Http(400 | 422, msg) => {
            let m = msg.to_ascii_lowercase();
            [
                "token",
                "context",
                "too long",
                "length",
                "maximum",
                "too large",
            ]
            .iter()
            .any(|w| m.contains(w))
        }
        _ => false,
    }
}

/// The JSON object anywhere in a model's reply.
fn parse_object<T: serde::de::DeserializeOwned>(reply: &str) -> Option<T> {
    let (a, b) = (reply.find('{')?, reply.rfind('}')?);
    serde_json::from_str(reply.get(a..=b)?).ok()
}

/// Takes what stays on this device out of a copy about to be synced.
pub fn strip(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "delete from wiki_books; delete from wiki_chapters;
         delete from chapter_wiki_facts; delete from cast_people;
         delete from cast_address_raw; delete from cast_address;",
    )
}

/// Brings the other device's wiki entries and relations for the books both have: per
/// uid, the newer one wins (removals included). An entry whose name is already taken
/// here by another entry is left out.
pub fn merge_attached(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "
        insert into main.wiki_entries (uid, novel_id, kind, name, names, aliases, summaries,
            rank, first_chapter, source, deleted, updated_at)
          select e.uid, m.id, e.kind, e.name, e.names, e.aliases, e.summaries,
            e.rank, e.first_chapter, e.source, e.deleted, e.updated_at
          from r.wiki_entries e join r.novels rn on rn.id = e.novel_id
          join main.novels m on m.book_key = rn.book_key
          where not exists (select 1 from main.wiki_entries x
                            where x.novel_id = m.id and x.name = e.name and x.uid <> e.uid)
          on conflict (uid) do update set kind = excluded.kind, name = excluded.name,
            names = excluded.names, aliases = excluded.aliases, summaries = excluded.summaries,
            rank = excluded.rank, source = excluded.source, deleted = excluded.deleted, updated_at = excluded.updated_at
          where excluded.updated_at > wiki_entries.updated_at;

        insert into main.wiki_relations (uid, novel_id, from_uid, to_uid, kind, labels, note,
            first_chapter, deleted, updated_at)
          select w.uid, m.id, w.from_uid, w.to_uid, w.kind, w.labels, w.note, w.first_chapter,
            w.deleted, w.updated_at
          from r.wiki_relations w join r.novels rn on rn.id = w.novel_id
          join main.novels m on m.book_key = rn.book_key
          where exists (select 1 from main.wiki_entries a where a.uid = w.from_uid)
            and exists (select 1 from main.wiki_entries b where b.uid = w.to_uid)
            and not exists (select 1 from main.wiki_relations x
                            where x.from_uid = w.from_uid and x.to_uid = w.to_uid
                              and x.kind = w.kind and x.uid <> w.uid)
          on conflict (uid) do update set kind = excluded.kind, labels = excluded.labels,
            note = excluded.note, deleted = excluded.deleted, updated_at = excluded.updated_at
          where excluded.updated_at > wiki_relations.updated_at;

        insert or ignore into main.wiki_notes (entry_uid, lang, chapter, summary)
          select n.entry_uid, n.lang, n.chapter, n.summary from r.wiki_notes n
          where exists (select 1 from main.wiki_entries e where e.uid = n.entry_uid);
        ",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::book;
    use std::sync::Mutex;

    /// A model that calls the wiki tools from a script, then says DONE.
    struct Agent {
        script: Mutex<Vec<Value>>,
        seen: Mutex<Vec<Vec<Value>>>,
    }

    fn call(name: &str, args: Value) -> Value {
        json!({ "id": format!("c-{name}"), "type": "function",
                "function": { "name": name, "arguments": args.to_string() } })
    }

    impl Agent {
        fn new(rounds: Vec<Vec<Value>>) -> Self {
            let mut script: Vec<Value> = rounds
                .into_iter()
                .map(|calls| json!({ "role": "assistant", "content": null, "tool_calls": calls }))
                .collect();
            script.push(json!({ "role": "assistant", "content": "DONE" }));
            script.reverse();
            Self {
                script: Mutex::new(script),
                seen: Mutex::default(),
            }
        }
    }

    impl Chat for Agent {
        fn chat(&self, _: &str, _: &str) -> Result<String, AiError> {
            Ok("ok".into())
        }
        fn model(&self) -> &str {
            "agent"
        }
        fn chat_tools(&self, messages: &[Value], _: &Value) -> Result<Value, AiError> {
            self.seen.lock().unwrap().push(messages.to_vec());
            Ok(self
                .script
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(json!({ "content": "DONE" })))
        }
    }

    #[test]
    fn learns_a_chapter_through_tool_calls() {
        let (conn, novel) = book("learn");
        let llm = Agent::new(vec![
            vec![call("wiki_search", json!({ "query": "林动" }))],
            vec![
                call(
                    "wiki_upsert",
                    json!({ "name": "林动", "kind": "character", "translated": "Lâm Động", "summary": "Thiếu niên Lâm gia, cầm kiếm (sword)." }),
                ),
                call(
                    "wiki_upsert",
                    json!({ "name": "道宗", "kind": "faction", "translated": "Đạo Tông", "summary": "Một tông môn (sect)." }),
                ),
                call(
                    "wiki_relate",
                    json!({ "from": "林动", "to": "道宗", "relation": "đệ tử của" }),
                ),
                call(
                    "wiki_relate",
                    json!({ "from": "林动", "to": "nobody", "relation": "x" }),
                ),
            ],
        ]);
        let n = learn(
            &conn,
            &llm,
            novel,
            10,
            "vi",
            "林动 来到 道宗。",
            "Lâm Động đến Đạo Tông.",
        )
        .unwrap();
        assert_eq!(n, 3);
        let w = load(&conn, novel).unwrap();
        assert_eq!(w.entries.len(), 2);
        let lin = w.entries.iter().find(|e| e.name == "林动").unwrap();
        assert_eq!(lin.names.get("vi").map(String::as_str), Some("Lâm Động"));
        assert_eq!(lin.first_chapter, Some(1));
        assert_eq!(w.relations.len(), 1);
        // The book's own chapter number shows, not the reading order.
        conn.execute(
            "update chapters set display_number = 629 where novel_id = ?1 and number = 1",
            [novel],
        )
        .unwrap();
        let shown = load(&conn, novel).unwrap();
        assert_eq!(
            shown
                .entries
                .iter()
                .find(|e| e.name == "林动")
                .unwrap()
                .first_chapter_shown,
            Some(629.0)
        );
        assert_eq!(shown.relations[0].first_chapter_shown, Some(629.0));
        assert_eq!(w.relations[0].kind, "đệ tử của");
        // The tool answers went back to the model, the missing entry with an error.
        let seen = llm.seen.lock().unwrap();
        let last = seen.last().unwrap();
        assert_eq!(last.iter().filter(|m| m["role"] == "tool").count(), 5);
        assert!(last.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("wiki_upsert first"));
        drop(seen);
        // Search by name finds the sect.
        let found = search(&conn, novel, "道宗", 1).unwrap();
        assert_eq!(found[0].name, "道宗");
        // A chapter is read once.
        assert_eq!(learn(&conn, &llm, novel, 10, "vi", "林动", "").unwrap(), 0);
    }

    #[test]
    fn the_prompt_gets_the_entries_a_chapter_names() {
        let (conn, novel) = book("ctx");
        let a = save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: "林动".into(),
                names: BTreeMap::from([("vi".into(), "Lâm Động".into())]),
                aliases: vec!["动少".into()],
                summaries: BTreeMap::from([("vi".into(), "Nhân vật chính.".into())]),
                rank: None,
            },
        )
        .unwrap();
        let b = save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: "林琅天".into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
            },
        )
        .unwrap();
        save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "place".into(),
                name: "炎城".into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
            },
        )
        .unwrap();
        add_relation(&conn, novel, &b.uid, &a.uid, "kẻ thù của", "vi", "").unwrap();
        let ctx = context(&conn, novel, "动少 看着 林琅天。", "vi").unwrap();
        assert!(ctx.contains("林动 → Lâm Động (character); also called 动少: Nhân vật chính."));
        assert!(ctx.contains("林琅天 — kẻ thù của — 林动"));
        assert!(!ctx.contains("炎城"));
        assert_eq!(context(&conn, novel, "nothing here", "vi").unwrap(), "");
    }

    #[test]
    fn the_users_words_win_and_removals_stay() {
        let (conn, novel) = book("user");
        let e = save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: "林动".into(),
                names: BTreeMap::from([("vi".into(), "Lâm Động".into())]),
                aliases: vec![],
                summaries: BTreeMap::from([("vi".into(), "Viết bởi tôi.".into())]),
                rank: None,
            },
        )
        .unwrap();
        let ai = |name: &str, translated: &str| AiEntry {
            name: name.into(),
            kind: "item".into(),
            translated: translated.into(),
            aliases: vec!["小动".into()],
            summary: "AI summary".into(),
            rank: None,
        };
        upsert_ai(&conn, novel, "vi", Some(2), ai("林动", "Lâm Đông")).unwrap();
        upsert_ai(&conn, novel, "en", Some(2), ai("林动", "Lin Dong")).unwrap();
        let now = by_uid(&conn, &e.uid).unwrap().unwrap();
        assert_eq!(now.summaries["vi"], "Viết bởi tôi.");
        // The AI adds a description in a language the user left empty.
        assert_eq!(now.summaries["en"], "AI summary");
        assert_eq!(now.kind, "character");
        assert_eq!(now.names["vi"], "Lâm Động");
        assert_eq!(now.names["en"], "Lin Dong");
        assert_eq!(now.aliases, vec!["小动"]);
        // An alias finds it too.
        assert_eq!(find(&conn, novel, "小动").unwrap().unwrap().1.uid, e.uid);
        remove(&conn, &e.uid).unwrap();
        assert_eq!(
            upsert_ai(&conn, novel, "vi", None, ai("林动", "x")).unwrap(),
            None
        );
        assert!(entries(&conn, novel).unwrap().is_empty());
        // The user can bring it back by its name.
        let back = save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: "林动".into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
            },
        )
        .unwrap();
        assert_eq!(back.uid, e.uid);
    }

    #[test]
    fn a_model_without_tools_answers_in_json() {
        struct NoTools;
        impl Chat for NoTools {
            fn chat(&self, system: &str, _: &str) -> Result<String, AiError> {
                assert!(system.contains("JSON"));
                Ok(r#"Here: {"entries":[{"name":"萧炎","kind":"character","translated":"Tiêu Viêm"},{"name":"药老","kind":"character"}],"relations":[{"from":"药老","to":"萧炎","relation":"sư phụ của"}]}"#.into())
            }
            fn model(&self) -> &str {
                "plain"
            }
        }
        let (conn, novel) = book("json");
        assert_eq!(
            learn(&conn, &NoTools, novel, 11, "vi", "萧炎 与 药老", "").unwrap(),
            3
        );
        assert_eq!(relations(&conn, novel).unwrap().len(), 1);
    }

    #[test]
    fn syncs_entries_and_removals_between_devices() {
        let (a, novel) = book("sync-a");
        let path = std::path::PathBuf::from(a.path().unwrap());
        let (mut b, _) = book("sync-b");
        // Same book on both devices.
        b.execute("update novels set book_key = 'k'", []).unwrap();
        let e = save(
            &a,
            novel,
            EntryInput {
                uid: None,
                kind: "place".into(),
                name: "炎城".into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::from([("vi".into(), "Thành.".into())]),
                rank: None,
            },
        )
        .unwrap();
        let f = save(
            &a,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: "林动".into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
            },
        )
        .unwrap();
        add_relation(&a, novel, &f.uid, &e.uid, "located_in", "vi", "").unwrap();
        let snap = crate::library::sync::export(&a, &path.with_extension("tmp")).unwrap();
        crate::library::sync::merge(&mut b, &snap, &path.with_extension("in")).unwrap();
        assert_eq!(entries(&b, 1).unwrap().len(), 2);
        assert_eq!(relations(&b, 1).unwrap().len(), 1);
        // Removed on B, then synced back to A.
        std::thread::sleep(std::time::Duration::from_millis(5));
        remove(&b, &e.uid).unwrap();
        let bpath = std::path::PathBuf::from(b.path().unwrap());
        let snap = crate::library::sync::export(&b, &bpath.with_extension("tmp")).unwrap();
        let mut a = a;
        crate::library::sync::merge(&mut a, &snap, &path.with_extension("in2")).unwrap();
        assert_eq!(entries(&a, novel).unwrap().len(), 1);
        assert!(relations(&a, novel).unwrap().is_empty());
    }

    #[test]
    fn relation_types_are_codes_and_free_words_are_kept_per_language() {
        let (conn, novel) = book("types");
        let entry = |name: &str, kind: &str| {
            save(
                &conn,
                novel,
                EntryInput {
                    uid: None,
                    kind: kind.into(),
                    name: name.into(),
                    names: BTreeMap::new(),
                    aliases: vec![],
                    summaries: BTreeMap::new(),
                    rank: Some(3),
                },
            )
            .unwrap()
        };
        let a = entry("药老", "character");
        let b = entry("萧炎", "character");
        let realm = entry("斗师", "level");
        assert_eq!(realm.rank, Some(3));
        // A rank only belongs to a level.
        assert_eq!(a.rank, None);
        add_relation(&conn, novel, &a.uid, &b.uid, "Master of", "vi", "").unwrap();
        add_relation(&conn, novel, &b.uid, &realm.uid, "has_level", "en", "").unwrap();
        add_relation(&conn, novel, &b.uid, &a.uid, "nghĩa phụ", "vi", "").unwrap();
        let rels = relations(&conn, novel).unwrap();
        assert_eq!(rels[0].kind, "master_of");
        assert!(rels[0].labels.is_empty());
        assert_eq!(rels[1].kind, "has_level");
        assert_eq!(rels[2].kind, "nghĩa phụ");
        assert_eq!(rels[2].labels["vi"], "nghĩa phụ");
        // The same free relation written in another language is found by its words.
        let mut labels = rels[2].labels.clone();
        labels.insert("en".into(), "godfather".into());
        conn.execute(
            "update wiki_relations set labels = ?2 where uid = ?1",
            params![rels[2].uid, to_json(&labels, "{}")],
        )
        .unwrap();
        let again = add_relation(&conn, novel, &b.uid, &a.uid, "godfather", "en", "").unwrap();
        assert_eq!(again, rels[2].uid);
        assert_eq!(relations(&conn, novel).unwrap().len(), 3);
        // A skill is its own kind, linked to who knows it.
        let fire = entry("焚决", "skill");
        assert_eq!((fire.kind.as_str(), fire.rank), ("skill", None));
        add_relation(&conn, novel, &b.uid, &fire.uid, "Has skill", "vi", "").unwrap();
        assert_eq!(relations(&conn, novel).unwrap()[3].kind, "has_skill");
    }

    #[test]
    fn translates_what_a_language_is_missing() {
        struct Translator(Mutex<Vec<String>>);
        impl Chat for Translator {
            fn chat(&self, system: &str, user: &str) -> Result<String, AiError> {
                assert!(system.contains("English"));
                self.0.lock().unwrap().push(user.to_string());
                Ok(if user.contains("\"relations\"") {
                    r#"{"relations":[{"id":0,"label":"godfather"}]}"#.into()
                } else if user.contains("林动") {
                    r#"{"entries":[{"id":"0","name":"Lin Dong","summary":"The hero."}]}"#.into()
                } else {
                    r#"{"entries":[{"id":0,"name":"Yan City"}]}"#.into()
                })
            }
            fn model(&self) -> &str {
                "t"
            }
        }
        let (conn, novel) = book("tr");
        conn.execute("update novels set original_lang = 'zh'", [])
            .unwrap();
        let lin = save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: "林动".into(),
                names: BTreeMap::from([("vi".into(), "Lâm Động".into())]),
                aliases: vec![],
                summaries: BTreeMap::from([("vi".into(), "Nhân vật chính.".into())]),
                rank: None,
            },
        )
        .unwrap();
        let city = save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "place".into(),
                name: "炎城".into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
            },
        )
        .unwrap();
        add_relation(&conn, novel, &lin.uid, &city.uid, "nghĩa phụ", "vi", "").unwrap();
        add_relation(&conn, novel, &lin.uid, &city.uid, "located_in", "vi", "").unwrap();
        let (e, r) = missing(&conn, novel, "en").unwrap();
        assert_eq!((e.len(), r.len()), (2, 1));
        let llm = Translator(Mutex::default());
        assert_eq!(translate_missing(&conn, &llm, novel, "en").unwrap(), 3);
        let lin = by_uid(&conn, &lin.uid).unwrap().unwrap();
        assert_eq!(lin.names["en"], "Lin Dong");
        assert_eq!(lin.names["vi"], "Lâm Động");
        assert_eq!(lin.summaries["en"], "The hero.");
        let city = by_uid(&conn, &city.uid).unwrap().unwrap();
        assert_eq!(city.names["en"], "Yan City");
        // No description anywhere, so none is asked for.
        assert!(city.summaries.is_empty());
        let rels = relations(&conn, novel).unwrap();
        assert_eq!(
            rels.iter().find(|r| r.kind == "nghĩa phụ").unwrap().labels["en"],
            "godfather"
        );
        // Nothing is missing now; the typed relation never needed words.
        let (e, r) = missing(&conn, novel, "en").unwrap();
        assert_eq!((e.len(), r.len()), (0, 0));
        assert_eq!(translate_missing(&conn, &llm, novel, "en").unwrap(), 0);
        // One request per section: the characters, the places, the relations.
        assert_eq!(llm.0.lock().unwrap().len(), 3);
        // A reply that is no JSON fills nothing in, and says so.
        struct Chatty;
        impl Chat for Chatty {
            fn chat(&self, _: &str, _: &str) -> Result<String, AiError> {
                Ok("Sure! Here are the names.".into())
            }
            fn model(&self) -> &str {
                "t"
            }
        }
        assert!(matches!(
            translate_missing(&conn, &Chatty, novel, "vi"),
            Err(AiError::Other(_))
        ));
    }

    #[test]
    fn translates_a_big_wiki_in_small_batches() {
        // Answers every entry it is sent, but can't take more than 6 at once (the reply
        // is cut short), and counts how many requests run together.
        struct Small {
            asked: Mutex<Vec<usize>>,
            running: std::sync::atomic::AtomicUsize,
            most: std::sync::atomic::AtomicUsize,
        }
        impl Chat for Small {
            fn chat(&self, _: &str, user: &str) -> Result<String, AiError> {
                use std::sync::atomic::Ordering::SeqCst;
                let now = self.running.fetch_add(1, SeqCst) + 1;
                self.most.fetch_max(now, SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                self.running.fetch_sub(1, SeqCst);
                let v: Value = serde_json::from_str(user).unwrap();
                let list = v["entries"].as_array().unwrap();
                self.asked.lock().unwrap().push(list.len());
                if list.len() > 6 {
                    return Ok(r#"{"entries":[{"id":0,"name":"cut"#.into());
                }
                let out: Vec<Value> = list
                    .iter()
                    .map(|e| json!({ "id": e["id"], "name": format!("EN {}", e["original_name"].as_str().unwrap()), "summary": "Done." }))
                    .collect();
                Ok(json!({ "entries": out }).to_string())
            }
            fn model(&self) -> &str {
                "t"
            }
        }
        let (conn, novel) = book("big");
        conn.execute("update novels set original_lang = 'zh'", [])
            .unwrap();
        let long = "长".repeat(900);
        for (i, kind) in ["character", "place", "skill"].iter().enumerate() {
            for n in 0..20 {
                save(
                    &conn,
                    novel,
                    EntryInput {
                        uid: None,
                        kind: kind.to_string(),
                        name: format!("{kind}{i}-{n}"),
                        names: BTreeMap::new(),
                        aliases: vec![],
                        summaries: BTreeMap::from([("raw".into(), long.clone())]),
                        rank: None,
                    },
                )
                .unwrap();
            }
        }
        let llm = Small {
            asked: Mutex::default(),
            running: Default::default(),
            most: Default::default(),
        };
        assert_eq!(translate_missing(&conn, &llm, novel, "en").unwrap(), 60);
        let asked = llm.asked.lock().unwrap().clone();
        // About 5,000 characters a request: 5 long entries, never one big request.
        assert!(asked.iter().all(|n| *n <= 6), "{asked:?}");
        assert_eq!(asked.iter().sum::<usize>(), 60);
        let most = llm.most.load(std::sync::atomic::Ordering::SeqCst);
        assert!(most > 1 && most <= TRANSLATE_PARALLEL, "{most}");
        let (e, _) = missing(&conn, novel, "en").unwrap();
        assert!(e.is_empty());

        // Short entries go 25 to a request; one the model can't take whole is split.
        let (conn, novel) = book("split");
        conn.execute("update novels set original_lang = 'zh'", [])
            .unwrap();
        for n in 0..30 {
            entry_in(&conn, novel, &format!("人{n}"));
        }
        let llm = Small {
            asked: Mutex::default(),
            running: Default::default(),
            most: Default::default(),
        };
        assert_eq!(translate_missing(&conn, &llm, novel, "en").unwrap(), 30);
        let asked = llm.asked.lock().unwrap().clone();
        assert!(asked.contains(&25) && asked.contains(&5), "{asked:?}");
        let lin = entries(&conn, novel).unwrap();
        assert!(lin
            .iter()
            .all(|e| e.names["en"] == format!("EN {}", e.name)));
    }

    fn entry_in(conn: &Connection, novel: i64, name: &str) {
        save(
            conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: name.into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn the_reader_sees_the_wiki_as_of_its_chapter() {
        let (conn, novel) = book("at");
        let ai = |name: &str, kind: &str, summary: &str| AiEntry {
            name: name.into(),
            kind: kind.into(),
            translated: String::new(),
            aliases: vec![],
            summary: summary.into(),
            rank: None,
        };
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            ai("林动", "character", "Thiếu niên."),
        )
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(5),
            ai("林动", "character", "Đệ tử Đạo Tông."),
        )
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(5),
            ai("道宗", "faction", "Tông môn."),
        )
        .unwrap();
        let rel = AiRelation {
            from: "林动".into(),
            to: "道宗".into(),
            relation: "member_of".into(),
            note: String::new(),
        };
        relate_ai(&conn, novel, "vi", Some(5), &rel)
            .unwrap()
            .unwrap();
        // A user's entry without a chapter is always known, with its own words.
        save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "place".into(),
                name: "青阳镇".into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::from([("vi".into(), "Thị trấn.".into())]),
                rank: None,
            },
        )
        .unwrap();

        let early = at_chapter(&conn, novel, 3).unwrap();
        let names: Vec<&str> = early.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["林动", "青阳镇"]);
        assert_eq!(early.entries[0].summaries["vi"], "Thiếu niên.");
        assert_eq!(early.entries[1].summaries["vi"], "Thị trấn.");
        assert!(early.relations.is_empty());

        let later = at_chapter(&conn, novel, 5).unwrap();
        assert_eq!(later.entries.len(), 3);
        let lin = later.entries.iter().find(|e| e.name == "林动").unwrap();
        assert_eq!(lin.summaries["vi"], "Đệ tử Đạo Tông.");
        assert_eq!(later.relations.len(), 1);
    }
}
