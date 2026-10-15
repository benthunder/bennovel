//! The wiki of an imported book: its characters, places, factions, items and terms,
//! and the relations between them. Only the wiki is embedded, never the chapters.
//!
//! - The AI fills it as it translates (`learn`, the last node of the graph in
//!   translate.rs): the model gets tools (`wiki_search`, `wiki_upsert`, `wiki_relate`)
//!   in the OpenAI tool-calling format and runs them in a loop; an API or model
//!   without tools gets the same job as one JSON answer instead.
//! - Before a chapter is translated, the entries it mentions (by name or alias) and
//!   the ones nearest to it in meaning go in the prompt (`context`), so names and
//!   relations stay the same through the book.
//! - Search by meaning uses sqlite-vec (`vec_distance_cosine`) over one vector per
//!   entry, made by the embedding model set with the LLM; without one, search goes by
//!   name only. The vectors stay on the device; entries and relations are synced.
//! - An entry the user edited keeps their words: the AI may only add to it. One the
//!   user removed stays removed.

use super::llm::{AiError, Chat};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const KINDS: &[&str] = &["character", "place", "faction", "item", "term", "other"];
/// Tool-call rounds allowed for one chapter.
const MAX_ROUNDS: usize = 12;
/// Entries put in a translation prompt at most.
const CONTEXT_ENTRIES: usize = 40;
/// Entries found by meaning added to those a chapter names.
const NEIGHBOURS: usize = 8;
/// Cosine distance under which an entry counts as related to a chapter.
const NEAR: f64 = 0.55;
/// Text of a chapter the AI reads to learn from it, in characters.
const ORIGINAL_CHARS: usize = 12_000;
const TRANSLATED_CHARS: usize = 6_000;
/// Text of a chapter its neighbours are found from.
const QUERY_CHARS: usize = 2_000;
/// Entries embedded per request.
const EMBED_BATCH: usize = 32;

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
    pub summary: String,
    pub first_chapter: Option<i64>,
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
    pub kind: String,
    pub note: String,
    pub first_chapter: Option<i64>,
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
    pub summary: String,
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
    /// An embedding model is set, so search goes by meaning too.
    pub by_meaning: bool,
}

const ENTRY_COLS: &str =
    "e.uid, e.kind, e.name, e.names, e.aliases, e.summary, e.first_chapter, e.source, e.updated_at";

fn entry_row(r: &Row) -> rusqlite::Result<Entry> {
    let names: String = r.get(3)?;
    let aliases: String = r.get(4)?;
    Ok(Entry {
        uid: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        names: serde_json::from_str(&names).unwrap_or_default(),
        aliases: serde_json::from_str(&aliases).unwrap_or_default(),
        summary: r.get(5)?,
        first_chapter: r.get(6)?,
        source: r.get(7)?,
        updated_at: r.get(8)?,
    })
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
        "select r.uid, r.from_uid, r.to_uid, r.kind, r.note, r.first_chapter from wiki_relations r
         join wiki_entries a on a.uid = r.from_uid join wiki_entries b on b.uid = r.to_uid
         where r.novel_id = ?1 and r.deleted = 0 and a.deleted = 0 and b.deleted = 0
         order by r.id",
    )?;
    let rows = stmt.query_map([novel_id], |r| {
        Ok(Relation {
            uid: r.get(0)?,
            from_uid: r.get(1)?,
            to_uid: r.get(2)?,
            kind: r.get(3)?,
            note: r.get(4)?,
            first_chapter: r.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn load(conn: &Connection, novel_id: i64, by_meaning: bool) -> rusqlite::Result<Wiki> {
    let (auto, last_error) = book_state(conn, novel_id)?;
    Ok(Wiki {
        entries: entries(conn, novel_id)?,
        relations: relations(conn, novel_id)?,
        auto,
        last_error,
        by_meaning,
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
        |r| Ok((r.get(9)?, entry_row(r)?, r.get::<_, bool>(10)?)),
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

#[allow(clippy::too_many_arguments)]
fn write_entry(
    conn: &Connection,
    id: i64,
    kind: &str,
    name: &str,
    names: &BTreeMap<String, String>,
    aliases: &[String],
    summary: &str,
    source: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "update wiki_entries set kind = ?2, name = ?3, names = ?4, aliases = ?5, summary = ?6,
           source = ?7, deleted = 0, updated_at = ?8 where id = ?1",
        params![
            id,
            kind,
            name,
            serde_json::to_string(names).unwrap_or_else(|_| "{}".into()),
            serde_json::to_string(aliases).unwrap_or_else(|_| "[]".into()),
            summary,
            source,
            super::db::now_ms()
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_entry(
    conn: &Connection,
    novel_id: i64,
    kind: &str,
    name: &str,
    names: &BTreeMap<String, String>,
    aliases: &[String],
    summary: &str,
    first_chapter: Option<i64>,
    source: &str,
) -> rusqlite::Result<String> {
    let uid = new_uid(conn)?;
    conn.execute(
        "insert into wiki_entries (uid, novel_id, kind, name, names, aliases, summary, first_chapter, source, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            uid,
            novel_id,
            kind,
            name,
            serde_json::to_string(names).unwrap_or_else(|_| "{}".into()),
            serde_json::to_string(aliases).unwrap_or_else(|_| "[]".into()),
            summary,
            first_chapter,
            source,
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
    let summary = input.summary.trim();
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
            write_entry(conn, id, kind, &name, &names, &aliases, summary, "user")?;
            conn.query_row("select uid from wiki_entries where id = ?1", [id], |r| {
                r.get(0)
            })?
        }
        None => insert_entry(
            conn, novel_id, kind, &name, &names, &aliases, summary, None, "user",
        )?,
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
/// user removed it before and the AI asks again.
#[allow(clippy::too_many_arguments)]
fn relate(
    conn: &Connection,
    novel_id: i64,
    from_uid: &str,
    to_uid: &str,
    kind: &str,
    note: &str,
    first_chapter: Option<i64>,
    by_user: bool,
) -> rusqlite::Result<Option<String>> {
    let kind = kind.trim();
    let note = note.trim();
    let existing: Option<(String, bool)> = conn
        .query_row(
            "select uid, deleted from wiki_relations where from_uid = ?1 and to_uid = ?2 and kind = ?3",
            params![from_uid, to_uid, kind],
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
            conn.execute(
                "insert into wiki_relations (uid, novel_id, from_uid, to_uid, kind, note, first_chapter, updated_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![uid, novel_id, from_uid, to_uid, kind, note, first_chapter, now],
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
    Ok(relate(conn, novel_id, from_uid, to_uid, kind, note, None, true)?.expect("by user"))
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
/// alone. Returns the entry's uid when there is one to relate.
fn upsert_ai(
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
            let (kind, summary) = if mine {
                let summary = if e.summary.is_empty() {
                    item.summary.trim()
                } else {
                    &e.summary
                };
                (e.kind.as_str(), summary.to_string())
            } else {
                let kind = if item.kind.trim().is_empty() {
                    e.kind.as_str()
                } else {
                    kind_of(&item.kind)
                };
                let summary = if item.summary.trim().is_empty() {
                    e.summary.clone()
                } else {
                    item.summary.trim().to_string()
                };
                (kind, summary)
            };
            if names != e.names || aliases != e.aliases || summary != e.summary || kind != e.kind {
                write_entry(
                    conn, id, kind, &e.name, &names, &aliases, &summary, &e.source,
                )?;
            }
            Ok(Some(e.uid))
        }
        None => {
            let mut names = BTreeMap::new();
            if !translated.is_empty() {
                names.insert(lang.to_string(), translated);
            }
            let aliases = clean_list(item.aliases, &name);
            Ok(Some(insert_entry(
                conn,
                novel_id,
                kind_of(&item.kind),
                &name,
                &names,
                &aliases,
                item.summary.trim(),
                chapter,
                "ai",
            )?))
        }
    }
}

fn relate_ai(
    conn: &Connection,
    novel_id: i64,
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
        &item.note,
        chapter,
        false,
    )?
    .ok_or("the user removed this relation"))
}

/* ---------- Search ---------- */

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// What an entry's vector is made from.
fn embed_text(e: &Entry) -> String {
    let mut s = format!("{} ({})", e.name, e.kind);
    for n in e.names.values() {
        s.push_str(&format!(" / {n}"));
    }
    if !e.aliases.is_empty() {
        s.push_str(&format!(". Also: {}", e.aliases.join(", ")));
    }
    if !e.summary.is_empty() {
        s.push_str(&format!(". {}", e.summary));
    }
    s
}

/// Embeds the entries whose vector is missing or out of date; returns how many.
pub fn reindex(conn: &Connection, llm: &dyn Chat, novel_id: i64) -> Result<usize, AiError> {
    let Some(model) = llm.embed_model().map(String::from) else {
        return Ok(0);
    };
    let mut stmt = conn.prepare(&format!(
        "select {ENTRY_COLS}, e.id, v.model, v.text from wiki_entries e
         left join wiki_vectors v on v.entry_id = e.id
         where e.novel_id = ?1 and e.deleted = 0"
    ))?;
    let stale: Vec<(i64, String)> = stmt
        .query_map([novel_id], |r| {
            let e = entry_row(r)?;
            let id: i64 = r.get(9)?;
            let m: Option<String> = r.get(10)?;
            let t: Option<String> = r.get(11)?;
            Ok((id, embed_text(&e), m, t))
        })?
        .filter_map(|row| match row {
            Ok((id, text, m, t)) => (m.as_deref() != Some(model.as_str())
                || t.as_deref() != Some(text.as_str()))
            .then_some(Ok((id, text))),
            Err(e) => Some(Err(e)),
        })
        .collect::<rusqlite::Result<_>>()?;
    for batch in stale.chunks(EMBED_BATCH) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        let vectors = llm.embed(&texts)?;
        for ((id, text), v) in batch.iter().zip(vectors) {
            conn.execute(
                "insert into wiki_vectors (entry_id, model, text, embedding) values (?1, ?2, ?3, ?4)
                 on conflict (entry_id) do update set model = excluded.model, text = excluded.text,
                   embedding = excluded.embedding",
                params![id, model, text, to_blob(&v)],
            )?;
        }
    }
    Ok(stale.len())
}

/// The entries nearest in meaning to `query`, with their cosine distance.
fn nearest(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    query: &str,
    limit: usize,
) -> Result<Vec<(Entry, f64)>, AiError> {
    let Some(model) = llm.embed_model().map(String::from) else {
        return Ok(Vec::new());
    };
    let has: bool = conn.query_row(
        "select exists (select 1 from wiki_vectors v join wiki_entries e on e.id = v.entry_id
           where e.novel_id = ?1 and v.model = ?2)",
        params![novel_id, model],
        |r| r.get(0),
    )?;
    if !has {
        return Ok(Vec::new());
    }
    let q = to_blob(&llm.embed(&[query.to_string()])?.pop().unwrap_or_default());
    let mut stmt = conn.prepare(&format!(
        "select {ENTRY_COLS}, vec_distance_cosine(v.embedding, ?3) as d
         from wiki_vectors v join wiki_entries e on e.id = v.entry_id
         where e.novel_id = ?1 and e.deleted = 0 and v.model = ?2 and length(v.embedding) = length(?3)
         order by d limit ?4"
    ))?;
    let rows = stmt.query_map(params![novel_id, model, q, limit as i64], |r| {
        Ok((entry_row(r)?, r.get(9)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Entries matching `query` by name, alias or summary, then (with an embedding model)
/// the nearest in meaning.
pub fn search(
    conn: &Connection,
    llm: Option<&dyn Chat>,
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
           or instr(lower(e.names), ?2) > 0 or instr(lower(e.summary), ?2) > 0)
         order by lower(e.name) = ?2 desc, instr(lower(e.name), ?2) = 0, e.name limit ?3"
    ))?;
    let mut out: Vec<Entry> = stmt
        .query_map(params![novel_id, q, limit as i64], entry_row)?
        .collect::<rusqlite::Result<_>>()?;
    if let Some(llm) = llm {
        if out.len() < limit {
            // A failed embedding leaves the search by name.
            if let Ok(near) = nearest(conn, llm, novel_id, query, limit) {
                for (e, _) in near {
                    if out.len() >= limit {
                        break;
                    }
                    if !out.iter().any(|x| x.uid == e.uid) {
                        out.push(e);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Entries named in `text` (by name or alias), the most mentioned first.
fn mentioned(conn: &Connection, novel_id: i64, text: &str) -> rusqlite::Result<Vec<Entry>> {
    let mut found: Vec<(usize, Entry)> = entries(conn, novel_id)?
        .into_iter()
        .filter_map(|e| {
            let hits: usize = std::iter::once(&e.name)
                .chain(e.aliases.iter())
                // One letter matches everywhere.
                .filter(|n| n.chars().count() >= 2)
                .map(|n| text.matches(n.as_str()).count())
                .sum();
            (hits > 0).then_some((hits, e))
        })
        .collect();
    found.sort_by_key(|(hits, _)| std::cmp::Reverse(*hits));
    Ok(found.into_iter().map(|(_, e)| e).collect())
}

fn head(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// The wiki part of a translation prompt for chapter `text`: the entries it names and
/// those nearest to it, with their relations. Empty when the wiki has nothing for it.
pub fn context(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    text: &str,
    lang: &str,
) -> rusqlite::Result<String> {
    let mut picked = mentioned(conn, novel_id, text)?;
    picked.truncate(CONTEXT_ENTRIES);
    if picked.len() < CONTEXT_ENTRIES {
        // Finding neighbours is a help, not a must: a failed request is skipped.
        if let Ok(near) = nearest(conn, llm, novel_id, &head(text, QUERY_CHARS), NEIGHBOURS) {
            for (e, d) in near {
                if d < NEAR
                    && picked.len() < CONTEXT_ENTRIES
                    && !picked.iter().any(|x| x.uid == e.uid)
                {
                    picked.push(e);
                }
            }
        }
    }
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
        if !e.summary.is_empty() {
            s.push_str(&format!(": {}", head(&e.summary, 200)));
        }
        s.push('\n');
    }
    let names: BTreeMap<&str, &Entry> = picked.iter().map(|e| (e.uid.as_str(), e)).collect();
    let rels: Vec<String> = relations(conn, novel_id)?
        .into_iter()
        .filter_map(|r| {
            let a = names.get(r.from_uid.as_str())?;
            let b = names.get(r.to_uid.as_str())?;
            Some(format!("- {} — {} — {}\n", a.name, r.kind, b.name))
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
                        "summary": { "type": "string", "description": format!("In {lang}, at most three sentences: who or what it is, as known so far. Write the whole summary, old facts included.") }
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
                        "relation": { "type": "string", "description": format!("Short, in {lang}: what `from` is to `to` (for example: master of, sworn brother of, member of, located in).") },
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
        "You keep the wiki of a novel being translated into {}. From the chapter below, record the characters, places, factions or sects, important items and special terms (cultivation ranks, techniques…) that matter to the story, and the relationships between characters and with factions. Skip passers-by and common words. Use the name exactly as written in the original text, and the name the translation uses. Only add what the chapter shows; do not guess.",
        super::translate::lang_name(lang)
    )
}

fn learn_prompt(lang: &str) -> String {
    format!(
        "{} Use the tools: wiki_search to check what is known, wiki_upsert to add or update an entry, wiki_relate for relationships (add both entries first). When the wiki is up to date, reply with DONE.",
        learn_rules(lang)
    )
}

fn json_prompt(lang: &str) -> String {
    format!(
        "{} Reply with only one JSON object and nothing else: {{\"entries\": [{{\"name\": original name, \"kind\": one of {}, \"translated\": name in the translation, \"aliases\": [other names], \"summary\": at most three sentences in {}}}], \"relations\": [{{\"from\": original name, \"to\": original name, \"relation\": short, in {}, \"note\": \"\"}}]}}",
        learn_rules(lang),
        KINDS.join(", "),
        super::translate::lang_name(lang),
        super::translate::lang_name(lang),
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
                head(&e.summary, 160)
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
    llm: &dyn Chat,
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
            match search(conn, Some(llm), novel_id, q, 8) {
                Ok(found) => json!(found
                    .iter()
                    .map(|e| json!({
                        "name": e.name, "kind": e.kind, "translated": e.names.get(lang),
                        "aliases": e.aliases, "summary": e.summary,
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
            Ok(item) => match relate_ai(conn, novel_id, chapter, &item)? {
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
    let (Some(a), Some(b)) = (reply.find('{'), reply.rfind('}')) else {
        return Ok(0);
    };
    let Ok(answer) = serde_json::from_str::<AiAnswer>(&reply[a..=b]) else {
        return Ok(0);
    };
    let mut changes = 0;
    for item in answer.entries {
        if upsert_ai(conn, novel_id, lang, chapter, item)?.is_some() {
            changes += 1;
        }
    }
    for item in &answer.relations {
        if relate_ai(conn, novel_id, chapter, item)?.is_ok() {
            changes += 1;
        }
    }
    Ok(changes)
}

/// Reads one translated chapter into the book's wiki (once per chapter) and embeds
/// what changed; returns the changes made.
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
            let answer = run_tool(conn, llm, novel_id, lang, chapter, name, args, &mut changes)?;
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
    if changes > 0 {
        reindex(conn, llm, novel_id)?;
    }
    Ok(changes)
}

/// `learn` as the translation graph runs it: a failure is kept for the wiki screen
/// and never fails the translation.
pub fn learn_quietly(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    chapter_id: i64,
    lang: &str,
    original: &str,
    translated: &str,
) -> rusqlite::Result<()> {
    if !auto(conn, novel_id)? {
        return Ok(());
    }
    match learn(conn, llm, novel_id, chapter_id, lang, original, translated) {
        Ok(_) => set_error(conn, novel_id, None),
        Err(e) => set_error(conn, novel_id, Some(&e.to_string())),
    }
}

/// Takes what stays on this device out of a copy about to be synced.
pub fn strip(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "delete from wiki_vectors; delete from wiki_books; delete from wiki_chapters;",
    )
}

/// Brings the other device's wiki entries and relations for the books both have: per
/// uid, the newer one wins (removals included). An entry whose name is already taken
/// here by another entry is left out.
pub fn merge_attached(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "
        insert into main.wiki_entries (uid, novel_id, kind, name, names, aliases, summary,
            first_chapter, source, deleted, updated_at)
          select e.uid, m.id, e.kind, e.name, e.names, e.aliases, e.summary,
            e.first_chapter, e.source, e.deleted, e.updated_at
          from r.wiki_entries e join r.novels rn on rn.id = e.novel_id
          join main.novels m on m.book_key = rn.book_key
          where not exists (select 1 from main.wiki_entries x
                            where x.novel_id = m.id and x.name = e.name and x.uid <> e.uid)
          on conflict (uid) do update set kind = excluded.kind, name = excluded.name,
            names = excluded.names, aliases = excluded.aliases, summary = excluded.summary,
            source = excluded.source, deleted = excluded.deleted, updated_at = excluded.updated_at
          where excluded.updated_at > wiki_entries.updated_at;

        insert into main.wiki_relations (uid, novel_id, from_uid, to_uid, kind, note,
            first_chapter, deleted, updated_at)
          select w.uid, m.id, w.from_uid, w.to_uid, w.kind, w.note, w.first_chapter,
            w.deleted, w.updated_at
          from r.wiki_relations w join r.novels rn on rn.id = w.novel_id
          join main.novels m on m.book_key = rn.book_key
          where exists (select 1 from main.wiki_entries a where a.uid = w.from_uid)
            and exists (select 1 from main.wiki_entries b where b.uid = w.to_uid)
            and not exists (select 1 from main.wiki_relations x
                            where x.from_uid = w.from_uid and x.to_uid = w.to_uid
                              and x.kind = w.kind and x.uid <> w.uid)
          on conflict (uid) do update set kind = excluded.kind, note = excluded.note,
            deleted = excluded.deleted, updated_at = excluded.updated_at
          where excluded.updated_at > wiki_relations.updated_at;
        ",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::book;
    use std::sync::Mutex;

    /// A model that calls the wiki tools from a script, then says DONE; and embeds
    /// texts by which of a few words they hold.
    struct Agent {
        script: Mutex<Vec<Value>>,
        seen: Mutex<Vec<Vec<Value>>>,
        embeds: bool,
    }

    fn call(name: &str, args: Value) -> Value {
        json!({ "id": format!("c-{name}"), "type": "function",
                "function": { "name": name, "arguments": args.to_string() } })
    }

    impl Agent {
        fn new(rounds: Vec<Vec<Value>>, embeds: bool) -> Self {
            let mut script: Vec<Value> = rounds
                .into_iter()
                .map(|calls| json!({ "role": "assistant", "content": null, "tool_calls": calls }))
                .collect();
            script.push(json!({ "role": "assistant", "content": "DONE" }));
            script.reverse();
            Self {
                script: Mutex::new(script),
                seen: Mutex::default(),
                embeds,
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
        fn embed_model(&self) -> Option<&str> {
            self.embeds.then_some("emb")
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    ["sword", "sect", "master"]
                        .iter()
                        .map(|w| if t.contains(w) { 1.0 } else { 0.05 })
                        .collect()
                })
                .collect())
        }
    }

    #[test]
    fn sqlite_vec_is_built_in() {
        let (conn, _) = book("vec");
        let v: String = conn
            .query_row("select vec_version()", [], |r| r.get(0))
            .unwrap();
        assert!(v.starts_with('v'));
    }

    #[test]
    fn learns_a_chapter_through_tool_calls() {
        let (conn, novel) = book("learn");
        let llm = Agent::new(
            vec![
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
            ],
            true,
        );
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
        let w = load(&conn, novel, true).unwrap();
        assert_eq!(w.entries.len(), 2);
        let lin = w.entries.iter().find(|e| e.name == "林动").unwrap();
        assert_eq!(lin.names.get("vi").map(String::as_str), Some("Lâm Động"));
        assert_eq!(lin.first_chapter, Some(1));
        assert_eq!(w.relations.len(), 1);
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
        // Each entry got its vector, and search by meaning finds the sect.
        let vectors: i64 = conn
            .query_row("select count(*) from wiki_vectors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vectors, 2);
        let found = search(&conn, Some(&llm), novel, "which sect?", 1).unwrap();
        assert_eq!(found[0].name, "道宗");
        // A chapter is read once.
        assert_eq!(learn(&conn, &llm, novel, 10, "vi", "林动", "").unwrap(), 0);
    }

    #[test]
    fn the_prompt_gets_the_entries_a_chapter_names() {
        let (conn, novel) = book("ctx");
        let llm = Agent::new(vec![], false);
        let a = save(
            &conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: "林动".into(),
                names: BTreeMap::from([("vi".into(), "Lâm Động".into())]),
                aliases: vec!["动少".into()],
                summary: "Nhân vật chính.".into(),
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
                summary: String::new(),
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
                summary: String::new(),
            },
        )
        .unwrap();
        add_relation(&conn, novel, &b.uid, &a.uid, "kẻ thù của", "").unwrap();
        let ctx = context(&conn, &llm, novel, "动少 看着 林琅天。", "vi").unwrap();
        assert!(ctx.contains("林动 → Lâm Động (character); also called 动少: Nhân vật chính."));
        assert!(ctx.contains("林琅天 — kẻ thù của — 林动"));
        assert!(!ctx.contains("炎城"));
        assert_eq!(
            context(&conn, &llm, novel, "nothing here", "vi").unwrap(),
            ""
        );
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
                summary: "Viết bởi tôi.".into(),
            },
        )
        .unwrap();
        let ai = |name: &str, translated: &str| AiEntry {
            name: name.into(),
            kind: "item".into(),
            translated: translated.into(),
            aliases: vec!["小动".into()],
            summary: "AI summary".into(),
        };
        upsert_ai(&conn, novel, "vi", Some(2), ai("林动", "Lâm Đông")).unwrap();
        upsert_ai(&conn, novel, "en", Some(2), ai("林动", "Lin Dong")).unwrap();
        let now = by_uid(&conn, &e.uid).unwrap().unwrap();
        assert_eq!(now.summary, "Viết bởi tôi.");
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
                summary: String::new(),
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
                summary: "Thành.".into(),
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
                summary: String::new(),
            },
        )
        .unwrap();
        add_relation(&a, novel, &f.uid, &e.uid, "sống ở", "").unwrap();
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
}
