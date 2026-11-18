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
    "equivalent_to",
];
/// The one language the wiki is written in: names, descriptions, relations and notes
/// are Vietnamese, whatever language a chapter is translated into. The original name
/// (`name`, `aliases`) stays as the text writes it, for finding it in the raw text.
pub const WIKI_LANG: &str = "vi";

/// The key of the descriptions written in the book's own language (older wikis).
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
    /// A level's ladder and stage, a character's current level (`Meta`).
    pub meta: Meta,
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

/// Where a level stands in its ladder and where a character stands on one. A level
/// keeps its place in `Entry::rank`; `system` names the ladder (a story can have
/// several: cultivation, alchemy, star ranks), so levels only compare within one. A
/// character's `level` is the name of its current level, and `stage` the step inside a
/// level (early 1, middle 2, late 3, peak 4) for either.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<i64>,
    #[serde(
        default,
        alias = "stage_name",
        skip_serializing_if = "String::is_empty"
    )]
    pub stage_name: String,
    /// A character's profile, in the wiki's language, one or two sentences each: what
    /// it is like, how old it is, where it comes from, what it is good at (beside the
    /// named skills it is linked to by `has_skill`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub personality: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub age: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub abilities: String,
    /// A skill's grade as the story ranks techniques ("Thiên giai", "Huyền cấp thượng
    /// phẩm") and how hard it is to learn or use, in the wiki's language.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub grade: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub difficulty: String,
}

impl Meta {
    /// Only what fits `kind` (a ladder for a level, a current level for a character),
    /// trimmed and short.
    fn for_kind(self, kind: &str) -> Self {
        let t = |s: String| head(s.trim(), 60);
        let long = |s: String| head(s.trim(), 400);
        let stage = self.stage.filter(|s| (1..=20).contains(s));
        match kind {
            "level" => Meta {
                system: t(self.system),
                stage,
                stage_name: t(self.stage_name),
                ..Default::default()
            },
            "character" => Meta {
                level: t(self.level),
                stage,
                stage_name: t(self.stage_name),
                personality: long(self.personality),
                age: t(self.age),
                origin: long(self.origin),
                abilities: long(self.abilities),
                ..Default::default()
            },
            "skill" => Meta {
                grade: t(self.grade),
                difficulty: long(self.difficulty),
                ..Default::default()
            },
            _ => Meta::default(),
        }
    }

    /// `new`'s fields where it has them, else these.
    fn merged(self, new: Meta) -> Self {
        let pick = |a: String, b: String| if b.is_empty() { a } else { b };
        Meta {
            system: pick(self.system, new.system),
            level: pick(self.level, new.level),
            stage: new.stage.or(self.stage),
            stage_name: pick(self.stage_name, new.stage_name),
            personality: pick(self.personality, new.personality),
            age: pick(self.age, new.age),
            origin: pick(self.origin, new.origin),
            abilities: pick(self.abilities, new.abilities),
            grade: pick(self.grade, new.grade),
            difficulty: pick(self.difficulty, new.difficulty),
        }
    }
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
    #[serde(default)]
    pub meta: Meta,
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
     (select c.display_number from chapters c where c.novel_id = e.novel_id and c.number = e.first_chapter),
     e.meta";
/// Columns after `ENTRY_COLS` start here.
const AFTER: usize = 12;

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
        meta: serde_json::from_str(&r.get::<_, String>(11)?).unwrap_or_default(),
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

/// Words a model writes for a relation type in English or Vietnamese instead of its
/// code, so "sư phụ của", "Teacher" or "kẻ thù" land on the type, named the same way in
/// every language, and not on a free relation in a mix of languages.
const REL_WORDS: &[(&str, &[&str])] = &[
    (
        "master_of",
        &[
            "master",
            "teacher",
            "mentor",
            "sư phụ",
            "sư tôn",
            "thầy",
            "thầy giáo",
        ],
    ),
    (
        "disciple_of",
        &[
            "disciple",
            "student",
            "apprentice",
            "pupil",
            "đệ tử",
            "đồ đệ",
            "đồ nhi",
            "học trò",
            "học sinh",
        ],
    ),
    (
        "parent_of",
        &[
            "parent",
            "father",
            "mother",
            "cha",
            "bố",
            "mẹ",
            "phụ thân",
            "mẫu thân",
            "cha mẹ",
        ],
    ),
    (
        "child_of",
        &["child", "son", "daughter", "con", "con trai", "con gái"],
    ),
    (
        "sibling_of",
        &[
            "sibling",
            "brother",
            "sister",
            "anh em",
            "chị em",
            "anh trai",
            "em trai",
            "chị gái",
            "em gái",
            "huynh đệ",
            "tỷ muội",
            "huynh muội",
            "tỷ đệ",
        ],
    ),
    (
        "spouse_of",
        &[
            "spouse",
            "husband",
            "wife",
            "vợ",
            "chồng",
            "vợ chồng",
            "phu quân",
            "thê tử",
            "phu nhân",
        ],
    ),
    (
        "lover_of",
        &[
            "lover",
            "partner",
            "girlfriend",
            "boyfriend",
            "người yêu",
            "tình nhân",
            "người tình",
        ],
    ),
    (
        "friend_of",
        &[
            "friend",
            "ally",
            "bạn",
            "bạn bè",
            "bằng hữu",
            "bạn thân",
            "đồng minh",
        ],
    ),
    (
        "enemy_of",
        &[
            "enemy",
            "rival",
            "foe",
            "nemesis",
            "kẻ thù",
            "kẻ địch",
            "đối thủ",
            "cừu nhân",
            "kẻ thù của",
        ],
    ),
    (
        "member_of",
        &[
            "member",
            "belongs to",
            "thành viên",
            "thuộc",
            "người của",
            "đệ tử của tông",
        ],
    ),
    (
        "leader_of",
        &[
            "leader",
            "head",
            "chief",
            "ruler",
            "lord",
            "thủ lĩnh",
            "tông chủ",
            "chưởng môn",
            "gia chủ",
            "bang chủ",
            "cốc chủ",
            "lãnh đạo",
        ],
    ),
    (
        "located_in",
        &[
            "located in",
            "inside",
            "part of",
            "nằm ở",
            "nằm trong",
            "thuộc vùng",
            "ở trong",
        ],
    ),
    (
        "owns",
        &[
            "owner",
            "owns",
            "has",
            "possesses",
            "sở hữu",
            "chủ nhân",
            "có",
        ],
    ),
    (
        "has_level",
        &["level", "realm", "rank", "cảnh giới", "cấp bậc", "tu vi"],
    ),
    (
        "equivalent_to",
        &[
            "equivalent",
            "equivalent to",
            "equals",
            "tương đương",
            "tương đương với",
            "ngang với",
            "ngang hàng",
        ],
    ),
    (
        "has_skill",
        &[
            "skill",
            "technique",
            "knows",
            "công pháp",
            "võ công",
            "kỹ năng",
            "chiêu thức",
            "bí thuật",
        ],
    ),
];

/// A relation type's code for `kind` ("Master of", "sư phụ của" → master_of), if it is
/// one.
pub fn rel_type(kind: &str) -> Option<&'static str> {
    let k = kind.trim().to_lowercase();
    let code = k.replace([' ', '-'], "_");
    if let Some(t) = REL_TYPES.iter().find(|t| **t == code) {
        return Some(t);
    }
    let words = k
        .trim_end_matches(" of")
        .trim_end_matches(" của")
        .trim_start_matches("is ")
        .trim_start_matches("là ")
        .trim();
    REL_WORDS
        .iter()
        .find(|(_, w)| w.contains(&words))
        .map(|(t, _)| *t)
}

/// How a relation reads in prompts to the model (English words for the types).
fn rel_words(r: &Relation, lang: &str) -> String {
    match rel_type(&r.kind) {
        Some(t) if lang == "vi" => rel_vi(t).to_string(),
        Some(t) => t.replace('_', " "),
        None => r
            .labels
            .get(lang)
            .cloned()
            .unwrap_or_else(|| r.kind.clone()),
    }
}

/// A relation type in the words of a Vietnamese prompt ("A là sư phụ của B").
fn rel_vi(code: &str) -> &'static str {
    match code {
        "master_of" => "là sư phụ của",
        "disciple_of" => "là đệ tử của",
        "parent_of" => "là cha mẹ của",
        "child_of" => "là con của",
        "sibling_of" => "là anh chị em của",
        "spouse_of" => "là vợ / chồng của",
        "lover_of" => "là người yêu của",
        "friend_of" => "là bạn của",
        "enemy_of" => "là kẻ thù của",
        "member_of" => "là thành viên của",
        "leader_of" => "là người đứng đầu",
        "located_in" => "nằm trong",
        "owns" => "sở hữu",
        "has_level" => "đang ở cảnh giới / cấp bậc",
        "has_skill" => "biết",
        "equivalent_to" => "tương đương với",
        _ => "liên quan đến",
    }
}

/// A kind in the words of a Vietnamese prompt.
fn kind_vi(kind: &str) -> &str {
    match kind {
        "character" => "nhân vật",
        "place" => "địa danh",
        "faction" => "thế lực",
        "item" => "vật phẩm",
        "level" => "cảnh giới / cấp bậc",
        "skill" => "công pháp / chiêu thức",
        "term" => "thuật ngữ",
        "other" => "khác",
        k => k,
    }
}

/// An entry in the names list of a prompt: "- 林动 → Lâm Động (nhân vật); tên gọi khác:
/// 动少; cảnh giới: Kim Đan, hậu kỳ", "- 金丹 → Kim Đan (cảnh giới; Luyện khí sĩ, bậc 9)".
fn entry_line(e: &Entry, shown: &str, vi: bool) -> String {
    let m = &e.meta;
    let stage = |vi: bool| -> String {
        match (m.stage_name.is_empty(), m.stage) {
            (false, _) => format!(", {}", m.stage_name),
            (true, Some(n)) if vi => format!(", bậc nhỏ {n}"),
            (true, Some(n)) => format!(", stage {n}"),
            _ => String::new(),
        }
    };
    let mut kind = if vi {
        kind_vi(&e.kind).to_string()
    } else {
        e.kind.clone()
    };
    if e.kind == "level" {
        let mut on = Vec::new();
        if !m.system.is_empty() {
            on.push(m.system.clone());
        }
        if let Some(r) = e.rank {
            on.push(if vi {
                format!("bậc {r}")
            } else {
                format!("rank {r}")
            });
        }
        if !on.is_empty() {
            kind = format!("{kind}; {}{}", on.join(", "), stage(vi));
        }
    }
    let mut s = format!("- {shown} ({kind})");
    if !e.aliases.is_empty() {
        let w = if vi {
            "tên gọi khác"
        } else {
            "also called"
        };
        s.push_str(&format!("; {w}: {}", e.aliases.join(", ")));
    }
    if e.kind == "character" && !m.level.is_empty() {
        let w = if vi { "cảnh giới" } else { "level" };
        s.push_str(&format!("; {w}: {}{}", m.level, stage(vi)));
    }
    s
}

/// What each kind code means, for the JSON and tool parts of a Vietnamese prompt.
const KINDS_VI: &str = "character (nhân vật), place (địa danh), faction (thế lực, tông môn, gia tộc), item (vật phẩm), level (cảnh giới, cấp bậc), skill (công pháp, chiêu thức), term (thuật ngữ), other (khác)";

/// What each relation code means, for a Vietnamese prompt.
const RELS_VI: &str = "master_of (là sư phụ của), disciple_of (là đệ tử của), parent_of (là cha mẹ của), child_of (là con của), sibling_of (là anh chị em của), spouse_of (là vợ / chồng của), lover_of (là người yêu của), friend_of (là bạn của), enemy_of (là kẻ thù của), member_of (là thành viên của), leader_of (đứng đầu), located_in (nằm trong, cho địa danh hay thế lực nằm trong nơi lớn hơn), owns (sở hữu), has_level (cảnh giới / cấp bậc hiện tại của nhân vật), has_skill (công pháp, chiêu thức nhân vật biết), equivalent_to (cảnh giới tương đương với cảnh giới ở thang cấp bậc khác)";

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
/// and whether it was removed. A live entry wins over a removed one: a name merged into
/// another entry (`merge`) is that entry's alias now.
fn find(
    conn: &Connection,
    novel_id: i64,
    name: &str,
) -> rusqlite::Result<Option<(i64, Entry, bool)>> {
    conn.query_row(
        &format!(
            "select {ENTRY_COLS}, e.id, e.deleted from wiki_entries e where e.novel_id = ?1 and (
               e.name = ?2 or exists (select 1 from json_each(e.aliases) a where a.value = ?2))
             order by e.deleted, e.name = ?2 desc limit 1"
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
    meta: &'a Meta,
    source: &'a str,
}

fn write_entry(conn: &Connection, id: i64, f: &Fields) -> rusqlite::Result<()> {
    conn.execute(
        "update wiki_entries set kind = ?2, name = ?3, names = ?4, aliases = ?5, summaries = ?6,
           rank = ?7, source = ?8, deleted = 0, updated_at = ?9, meta = ?10 where id = ?1",
        params![
            id,
            f.kind,
            f.name,
            to_json(f.names, "{}"),
            to_json(&f.aliases, "[]"),
            to_json(f.summaries, "{}"),
            f.rank,
            f.source,
            super::db::now_ms(),
            to_json(f.meta, "{}")
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
           first_chapter, source, updated_at, meta)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
            super::db::now_ms(),
            to_json(f.meta, "{}")
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
    let meta = input.meta.for_kind(kind);
    let fields = Fields {
        kind,
        name: &name,
        names: &names,
        aliases: &aliases,
        summaries: &summaries,
        rank: input.rank.filter(|_| kind == "level"),
        meta: &meta,
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

/// Makes entry `from` part of entry `into`, as one character written two ways ("bé áo
/// xanh" and her name): `into` takes `from`'s name and aliases as aliases, the names
/// and descriptions it lacks per language, and its relations; `from` is removed. A
/// merge the user asked for makes `into` theirs.
pub fn merge(
    conn: &Connection,
    novel_id: i64,
    from_uid: &str,
    into_uid: &str,
    by_user: bool,
) -> Result<Entry, AiError> {
    if from_uid == into_uid {
        return Err(AiError::Other("wiki-merge-same".into()));
    }
    let (Some(from), Some(into)) = (by_uid(conn, from_uid)?, by_uid(conn, into_uid)?) else {
        return Err(AiError::Other("wiki-entry-missing".into()));
    };
    let id: i64 = conn.query_row(
        "select id from wiki_entries where uid = ?1 and novel_id = ?2",
        params![into_uid, novel_id],
        |r| r.get(0),
    )?;
    let mut aliases = into.aliases.clone();
    aliases.push(from.name.clone());
    aliases.extend(from.aliases.iter().cloned());
    let aliases = clean_list(aliases, &into.name);
    let mut names = into.names.clone();
    for (l, n) in &from.names {
        names.entry(l.clone()).or_insert_with(|| n.clone());
    }
    let mut summaries = into.summaries.clone();
    for (l, t) in &from.summaries {
        summaries.entry(l.clone()).or_insert_with(|| t.clone());
    }
    let source = if by_user {
        "user"
    } else {
        into.source.as_str()
    };
    write_entry(
        conn,
        id,
        &Fields {
            kind: &into.kind,
            name: &into.name,
            names: &names,
            aliases: &aliases,
            summaries: &summaries,
            rank: into.rank.or(from.rank),
            meta: &from.meta.clone().merged(into.meta.clone()),
            source,
        },
    )?;
    conn.execute(
        "update wiki_entries set first_chapter = min(coalesce(first_chapter, ?2), coalesce(?2, first_chapter))
         where id = ?1",
        params![id, from.first_chapter],
    )?;
    // The relations move over, but not onto `into` itself nor onto one it already has.
    let now = super::db::now_ms();
    conn.execute(
        "update wiki_relations set deleted = 1, updated_at = ?3 where deleted = 0 and (
           (from_uid = ?1 and to_uid = ?2) or (from_uid = ?2 and to_uid = ?1)
           or (from_uid = ?1 and exists (select 1 from wiki_relations x where x.from_uid = ?2
                and x.to_uid = wiki_relations.to_uid and x.kind = wiki_relations.kind))
           or (to_uid = ?1 and exists (select 1 from wiki_relations x where x.to_uid = ?2
                and x.from_uid = wiki_relations.from_uid and x.kind = wiki_relations.kind)))",
        params![from_uid, into_uid, now],
    )?;
    conn.execute(
        "update wiki_relations set from_uid = ?2, updated_at = ?3 where from_uid = ?1 and deleted = 0",
        params![from_uid, into_uid, now],
    )?;
    conn.execute(
        "update wiki_relations set to_uid = ?2, updated_at = ?3 where to_uid = ?1 and deleted = 0",
        params![from_uid, into_uid, now],
    )?;
    conn.execute(
        "update or ignore wiki_notes set entry_uid = ?2 where entry_uid = ?1",
        params![from_uid, into_uid],
    )?;
    conn.execute(
        "update wiki_names set entry_uid = ?2 where entry_uid = ?1",
        params![from_uid, into_uid],
    )?;
    remove(conn, from_uid)?;
    Ok(by_uid(conn, into_uid)?.expect("merged entry"))
}

/// Cleans what older versions let the AI write: entries of its own that are common
/// nouns are removed (and so never added again), and one whose name is another entry's
/// alias is merged into it. The user's own entries stay as they are. Returns the
/// changes made.
pub fn tidy(conn: &Connection, novel_id: i64) -> rusqlite::Result<usize> {
    let all = entries(conn, novel_id)?;
    let mut changes = 0;
    for e in all.iter().filter(|e| e.source == "ai") {
        // Judged by the Vietnamese name when there is one (the raw one may be Chinese).
        let translated = e
            .names
            .get("vi")
            .or_else(|| e.names.values().next())
            .map(String::as_str)
            .unwrap_or("");
        let common = !proper_noun(&e.name, translated);
        if common {
            remove(conn, &e.uid)?;
            changes += 1;
            continue;
        }
        let owner = all.iter().find(|o| {
            o.uid != e.uid && o.aliases.iter().any(|a| a == &e.name) && by_uid_live(conn, &o.uid)
        });
        if let Some(o) = owner {
            if merge(conn, novel_id, &e.uid, &o.uid, false).is_ok() {
                changes += 1;
            }
        }
    }
    Ok(changes)
}

fn by_uid_live(conn: &Connection, uid: &str) -> bool {
    conn.query_row(
        "select deleted = 0 from wiki_entries where uid = ?1",
        [uid],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// `tidy` for every book, once, as the library file is brought up to date (db.rs).
pub fn tidy_all(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("select distinct novel_id from wiki_entries")?;
    let books: Vec<i64> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for b in books {
        tidy(conn, b)?;
    }
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
    #[serde(default)]
    meta: Meta,
}

#[derive(Debug, Deserialize)]
struct AiRelation {
    from: String,
    to: String,
    relation: String,
    #[serde(default)]
    note: String,
}

/// Words that follow a name to make a family, sect, place or building of it ("Mặc
/// gia", "Lâm tộc", "Lin family"): the one lower-case word a proper noun may end with.
const NAME_SUFFIXES: &[&str] = &[
    "gia",
    "thị",
    "tộc",
    "phái",
    "môn",
    "tông",
    "giáo",
    "bang",
    "hội",
    "quốc",
    "triều",
    "thành",
    "trấn",
    "thôn",
    "sơn",
    "phong",
    "cốc",
    "động",
    "hồ",
    "giang",
    "hà",
    "hải",
    "châu",
    "đảo",
    "lâu",
    "các",
    "điện",
    "cung",
    "phủ",
    "viện",
    "tự",
    "quán",
    "trang",
    "đường",
    "đài",
    "uyển",
    "family",
    "clan",
    "sect",
    "city",
    "town",
    "village",
    "mountain",
    "peak",
    "valley",
    "lake",
    "river",
    "sea",
    "island",
    "palace",
    "hall",
    "temple",
    "manor",
    "kingdom",
    "empire",
    "dynasty",
    "academy",
    "pavilion",
    "tower",
    "house",
    "realm",
];

/// Whether `name`, in a script with capitals, is written as a proper noun: its first
/// word capitalised and, past one word, another capitalised word too ("Trần Bình An",
/// "núi Lạc Phách" fails the first, "Bé gái váy hồng" or "Tổ trạch" at the start of a
/// sentence fail the second), or a name with one of `NAME_SUFFIXES` ("Mặc gia"). None
/// for a script without capitals (Chinese…).
fn capitalised_name(name: &str) -> Option<bool> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let cap = |w: &str| -> Option<bool> {
        let c = w.chars().find(|c| c.is_alphabetic())?;
        (c.is_uppercase() || c.is_lowercase()).then(|| c.is_uppercase())
    };
    let first = words.iter().find_map(|w| cap(w))?;
    if !first {
        return Some(false);
    }
    let rest: Vec<&str> = words
        .iter()
        .skip(1)
        .copied()
        .filter(|w| cap(w).is_some())
        .collect();
    Some(
        rest.is_empty()
            || rest.iter().any(|w| cap(w) == Some(true))
            || (rest.len() == 1 && NAME_SUFFIXES.contains(&rest[0].to_lowercase().as_str())),
    )
}

/// Whether the AI's `name` (as the original writes it) with its `translated` form reads
/// as a proper noun: two letters at least, and capitalised as a name wherever its script
/// has capitals (`capitalised_name`: "núi", "linh khí", "Bé gái váy hồng" are not). A
/// Chinese name has no capitals, so its translation tells.
fn proper_noun(name: &str, translated: &str) -> bool {
    if name.trim().chars().count() < 2 {
        return false;
    }
    match capitalised_name(name) {
        Some(ok) => ok,
        None => capitalised_name(translated).unwrap_or(true),
    }
}

/// The live entry an AI item stands for when its own name is new: one that has any of
/// its aliases as name or alias ("bé áo xanh" listed with the name of a known girl), or
/// the same name in `lang`.
fn find_same(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    item: &AiEntry,
) -> rusqlite::Result<Option<(i64, Entry, bool)>> {
    for alias in &item.aliases {
        if alias.trim().chars().count() < 2 {
            continue;
        }
        if let Some(found @ (_, _, false)) = find(conn, novel_id, alias)? {
            return Ok(Some(found));
        }
    }
    let translated = item.translated.trim();
    if translated.is_empty() {
        return Ok(None);
    }
    conn.query_row(
        &format!(
            "select {ENTRY_COLS}, e.id, e.deleted from wiki_entries e
             where e.novel_id = ?1 and e.deleted = 0 and json_extract(e.names, '$.' || ?2) = ?3
             order by e.id limit 1"
        ),
        params![novel_id, lang, translated],
        |r| Ok((r.get(AFTER)?, entry_row(r)?, r.get::<_, bool>(AFTER + 1)?)),
    )
    .optional()
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
    mut item: AiEntry,
) -> rusqlite::Result<Option<String>> {
    let name = item.name.trim().to_string();
    if name.is_empty() {
        return Ok(None);
    }
    let translated = item.translated.trim().to_string();
    let known = match find(conn, novel_id, &name)? {
        Some(found) => Some(found),
        None => find_same(conn, novel_id, lang, &item)?,
    };
    match known {
        Some((_, _, true)) => Ok(None),
        Some((id, e, false)) => {
            let new_meta = std::mem::take(&mut item.meta);
            let mut names = e.names.clone();
            let mut aliases = e.aliases.clone();
            aliases.extend(item.aliases);
            if name != e.name {
                aliases.push(name.clone());
            }
            let aliases = clean_list(aliases, &e.name);
            let mine = e.source == "user";
            // Found by another of its names (a nickname): that name's translation is
            // not the entry's.
            let own_name = name == e.name;
            // A name keeps its first translation: the parts and chapters after it write
            // it the same way (the user can still change it on the wiki screen).
            let _ = own_name;
            if !translated.is_empty() && !names.contains_key(lang) {
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
            // The user's own values stay; the AI only fills what is missing.
            let meta = if mine {
                new_meta.merged(e.meta.clone())
            } else {
                e.meta.clone().merged(new_meta)
            }
            .for_kind(kind);
            if names != e.names
                || aliases != e.aliases
                || summaries != e.summaries
                || kind != e.kind
                || rank != e.rank
                || meta != e.meta
            {
                let f = Fields {
                    kind,
                    name: &e.name,
                    names: &names,
                    aliases: &aliases,
                    summaries: &summaries,
                    rank,
                    meta: &meta,
                    source: &e.source,
                };
                write_entry(conn, id, &f)?;
            }
            Ok(Some(e.uid))
        }
        // A common noun ("núi", "the sect", "linh khí") is not kept as an entry.
        None if !proper_noun(&name, &translated) => Ok(None),
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
                meta: &item.meta.clone().for_kind(kind),
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

/* ---------- The vector table of proper nouns ---------- */

/// What the vectors are made with: letter trigrams on the device (`genre_terms::embed`),
/// no embedding model. Rows made another way are made again.
const VEC_MODEL: &str = "trigram-256";
/// Two names closer than this (cosine distance) may be the same entity…
const NEAR: f64 = 0.35;
/// …and closer than this they are, whatever else.
const SAME: f64 = 0.06;

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// What a name is embedded as: the raw name and the translated one (the kind is
/// compared on its own).
fn embed_text(raw: &str, translated: &str) -> String {
    if translated.is_empty() || translated == raw {
        raw.to_string()
    } else {
        format!("{raw} {translated}")
    }
}

/// One proper noun of the vector table.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Name {
    pub raw: String,
    pub translated: String,
    pub kind: String,
    pub entry_uid: String,
}

/// Brings the book's vector table (`wiki_names`) for `lang` in line with its wiki: one
/// row per entry name and alias, with the name the translation uses (an alias takes its
/// entry's) and the kind, and the rows without a vector yet get one, made on the device
/// (`VEC_MODEL`). Returns the rows given a vector. `_llm` is not used any more.
pub fn index_names(
    conn: &Connection,
    _llm: Option<&dyn Chat>,
    novel_id: i64,
    lang: &str,
) -> Result<usize, AiError> {
    let now = super::db::now_ms();
    let mut keep: Vec<(String, String, String, String, String)> = Vec::new();
    for e in entries(conn, novel_id)? {
        let translated = e.names.get(lang).cloned().unwrap_or_default();
        let meta = to_json(&e.meta, "{}");
        for raw in std::iter::once(&e.name).chain(e.aliases.iter()) {
            keep.push((
                raw.clone(),
                translated.clone(),
                e.kind.clone(),
                e.uid.clone(),
                meta.clone(),
            ));
        }
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut up = tx.prepare(
            "insert into wiki_names (novel_id, raw, lang, translated, kind, entry_uid, updated_at, meta)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             on conflict (novel_id, raw, lang) do update set
               translated = excluded.translated, kind = excluded.kind, entry_uid = excluded.entry_uid,
               updated_at = excluded.updated_at, meta = excluded.meta,
               embedding = case when wiki_names.translated = excluded.translated
                 and wiki_names.kind = excluded.kind then wiki_names.embedding end,
               model = case when wiki_names.translated = excluded.translated
                 and wiki_names.kind = excluded.kind then wiki_names.model end
             where wiki_names.translated <> excluded.translated or wiki_names.kind <> excluded.kind
               or wiki_names.entry_uid <> excluded.entry_uid or wiki_names.meta <> excluded.meta",
        )?;
        for (raw, translated, kind, uid, meta) in &keep {
            up.execute(params![
                novel_id, raw, lang, translated, kind, uid, now, meta
            ])?;
        }
    }
    let raws: Vec<&str> = keep.iter().map(|k| k.0.as_str()).collect();
    tx.execute(
        "delete from wiki_names where novel_id = ?1 and lang = ?2
           and raw not in (select value from json_each(?3))",
        params![novel_id, lang, to_json(&raws, "[]")],
    )?;
    tx.commit()?;
    let model = VEC_MODEL;
    let mut stmt = conn.prepare(
        "select id, raw, translated, kind from wiki_names
         where novel_id = ?1 and lang = ?2 and (embedding is null or model is not ?3)",
    )?;
    let todo: Vec<(i64, String)> = stmt
        .query_map(params![novel_id, lang, model], |r| {
            Ok((
                r.get(0)?,
                embed_text(&r.get::<_, String>(1)?, &r.get::<_, String>(2)?),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut done = 0;
    for (id, text) in todo {
        let v = super::genre_terms::embed(&text);
        conn.execute(
            "update wiki_names set embedding = ?2, model = ?3 where id = ?1",
            params![id, to_blob(&v), model],
        )?;
        done += 1;
    }
    Ok(done)
}

/// The rows of the book's vector table for `lang` whose raw name `text` has (as a whole
/// word, names.rs), the most mentioned first.
pub fn names_in(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    text: &str,
) -> rusqlite::Result<Vec<Name>> {
    let mut stmt = conn.prepare(
        "select raw, translated, kind, entry_uid from wiki_names where novel_id = ?1 and lang = ?2",
    )?;
    let all: Vec<Name> = stmt
        .query_map(params![novel_id, lang], |r| {
            Ok(Name {
                raw: r.get(0)?,
                translated: r.get(1)?,
                kind: r.get(2)?,
                entry_uid: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let finder =
        super::names::Finder::new(&all.iter().map(|n| vec![n.raw.as_str()]).collect::<Vec<_>>());
    let mut found: Vec<(usize, Name)> = finder
        .hits(text)
        .into_iter()
        .zip(all)
        .filter(|(hits, _)| *hits > 0)
        .collect();
    found.sort_by_key(|(hits, _)| std::cmp::Reverse(*hits));
    Ok(found.into_iter().map(|(_, n)| n).collect())
}

/// Whether two names may be one entity written two ways: one holds the other, or they
/// share their translation.
fn alike(a: &Name, b: &Name) -> bool {
    let (ra, rb) = (a.raw.to_lowercase(), b.raw.to_lowercase());
    let (ta, tb) = (a.translated.to_lowercase(), b.translated.to_lowercase());
    ra.contains(&rb)
        || rb.contains(&ra)
        || (!ta.is_empty() && ta == tb)
        || (!ta.is_empty() && !tb.is_empty() && (ta.contains(&tb) || tb.contains(&ta)))
}

/// Merges the book's entries whose names land together in the vector table: the same
/// kind, closer than `SAME`, or closer than `NEAR` and `alike`. The entry met first
/// keeps its name; two entries of the user's are never merged. Needs the rows given
/// their vectors (`index_names`); returns the merges made.
pub fn merge_near(conn: &Connection, novel_id: i64, lang: &str) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare(
        "select a.raw, a.translated, a.kind, a.entry_uid, b.raw, b.translated, b.kind, b.entry_uid,
           vec_distance_cosine(a.embedding, b.embedding) as d
         from wiki_names a join wiki_names b
           on b.novel_id = a.novel_id and b.lang = a.lang and b.kind = a.kind
              and b.entry_uid > a.entry_uid and b.model = a.model
              -- Two levels of different ladders are never one.
              and (coalesce(json_extract(a.meta, '$.system'), '') = ''
                   or coalesce(json_extract(b.meta, '$.system'), '') = ''
                   or json_extract(a.meta, '$.system') = json_extract(b.meta, '$.system'))
              and length(b.embedding) = length(a.embedding)
         where a.novel_id = ?1 and a.lang = ?2 and a.embedding is not null
           and vec_distance_cosine(a.embedding, b.embedding) < ?3
         order by d",
    )?;
    let name = |r: &Row, at: usize| -> rusqlite::Result<Name> {
        Ok(Name {
            raw: r.get(at)?,
            translated: r.get(at + 1)?,
            kind: r.get(at + 2)?,
            entry_uid: r.get(at + 3)?,
        })
    };
    let pairs: Vec<(Name, Name, f64)> = stmt
        .query_map(params![novel_id, lang, NEAR], |r| {
            Ok((name(r, 0)?, name(r, 4)?, r.get(8)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut merges = 0;
    for (a, b, d) in pairs {
        if d >= SAME && !alike(&a, &b) {
            continue;
        }
        let (Some(ea), Some(eb)) = (by_uid(conn, &a.entry_uid)?, by_uid(conn, &b.entry_uid)?)
        else {
            continue;
        };
        if !by_uid_live(conn, &ea.uid) || !by_uid_live(conn, &eb.uid) {
            continue;
        }
        if ea.source == "user" && eb.source == "user" {
            continue;
        }
        // The user's entry stays, else the one met first.
        let a_first = match (ea.source.as_str(), eb.source.as_str()) {
            ("user", _) => true,
            (_, "user") => false,
            _ => ea.first_chapter.unwrap_or(i64::MAX) <= eb.first_chapter.unwrap_or(i64::MAX),
        };
        let (from, into) = if a_first { (&eb, &ea) } else { (&ea, &eb) };
        if merge(conn, novel_id, &from.uid, &into.uid, false).is_ok() {
            merges += 1;
        }
    }
    Ok(merges)
}

/// The wiki part of a translation prompt for chapter `text`: the proper nouns of the
/// vector table it names, as the translation must write them (raw → translated, kind),
/// and how those entries relate, with the circumstances. Empty when there are none.
#[cfg(test)]
pub fn context(
    conn: &Connection,
    novel_id: i64,
    text: &str,
    lang: &str,
) -> rusqlite::Result<String> {
    Ok(recall(conn, None, novel_id, text, lang)?.prompt(text))
}

/// A name looked up in the vector table counts when it is closer than this to a row…
const FIND: f64 = 0.25;
/// Phrases of a chapter looked up in the vector table at most.
const FIND_MAX: usize = 64;
/// Entries a chapter's recall keeps (each part then takes up to `CONTEXT_ENTRIES`).
const RECALL_ENTRIES: usize = 300;

/// The proper nouns a chapter names, with what the wiki knows of them, found before it
/// is translated: by the exact names in the vector table, and also by the phrases that
/// look like names and whose vectors (made on the device) land next to a row of it (a name the
/// convert wrote a little differently). Each part of the chapter then gets only the
/// names it has (`prompt`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Recall {
    vi: bool,
    items: Vec<Recalled>,
    /// From uid, to uid, the line.
    rels: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Recalled {
    uid: String,
    /// What finds the entry in a text: its names, aliases and the phrases found near it.
    terms: Vec<String>,
    line: String,
}

/// Words that start with a capital letter, one to four in a row: what may be a name in
/// a text in a cased script ("Vương thành", "Lâm Động ca"). Not for Chinese and the like.
pub(crate) fn name_phrases(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |run: &[&str]| {
        for n in (1..=run.len().min(4)).rev() {
            for w in run.windows(n) {
                let p = w.join(" ");
                if p.chars().count() >= 2 && !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    };
    for sentence in text.split(|c: char| ".,;:!?…\"“”()[]「」—\n".contains(c)) {
        let mut run: Vec<&str> = Vec::new();
        for w in sentence.split_whitespace() {
            let w = w.trim_matches(|c: char| !c.is_alphanumeric());
            if w.chars().next().is_some_and(char::is_uppercase) {
                run.push(w);
            } else {
                add(&run);
                run.clear();
            }
        }
        add(&run);
    }
    out
}

/// Whether `phrase` shares a word with `name`, ignoring case.
fn shares_word(phrase: &str, name: &str) -> bool {
    let words: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
    phrase
        .split_whitespace()
        .any(|w| words.contains(&w.to_lowercase()))
}

/// The rows of the vector table next to the phrases of `text` that may be names and are
/// not already found: (phrase, row). The phrases get their vectors on the device, like
/// the rows (`index_names`).
fn near_names(
    conn: &Connection,
    novel_id: i64,
    lang: &str,
    text: &str,
    found: &[Name],
) -> rusqlite::Result<Vec<(String, Name)>> {
    let model = VEC_MODEL;
    let known = |p: &str| {
        found
            .iter()
            .any(|n| n.raw.contains(p) || p.contains(n.raw.as_str()) || n.translated == p)
    };
    let phrases: Vec<String> = name_phrases(text)
        .into_iter()
        .filter(|p| !known(p))
        .take(FIND_MAX)
        .collect();
    if phrases.is_empty() {
        return Ok(Vec::new());
    }
    let vectors: Vec<Vec<f32>> = phrases.iter().map(|p| super::genre_terms::embed(p)).collect();
    let mut stmt = conn.prepare(
        "select raw, translated, kind, entry_uid, vec_distance_cosine(embedding, ?3) as d
         from wiki_names
         where novel_id = ?1 and lang = ?2 and model = ?4 and embedding is not null
           and length(embedding) = length(?3)
         order by d limit 1",
    )?;
    let mut out = Vec::new();
    for (p, v) in phrases.into_iter().zip(vectors) {
        let hit = stmt
            .query_row(params![novel_id, lang, to_blob(&v), model], |r| {
                Ok((
                    Name {
                        raw: r.get(0)?,
                        translated: r.get(1)?,
                        kind: r.get(2)?,
                        entry_uid: r.get(3)?,
                    },
                    r.get::<_, f64>(4)?,
                ))
            })
            .optional()?;
        if let Some((n, d)) = hit {
            let close = d < NEAR
                || (d < FIND && (shares_word(&p, &n.raw) || shares_word(&p, &n.translated)));
            if close {
                out.push((p, n));
            }
        }
    }
    Ok(out)
}

/// What the wiki knows of the proper nouns `text` names (`Recall`).
pub fn recall(
    conn: &Connection,
    _llm: Option<&dyn Chat>,
    novel_id: i64,
    text: &str,
    lang: &str,
) -> rusqlite::Result<Recall> {
    let vi = lang == "vi";
    let found = names_in(conn, novel_id, lang, text)?;
    let near = near_names(conn, novel_id, lang, text, &found)?;
    let mut order: Vec<String> = Vec::new();
    let mut extra: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in &found {
        if !order.contains(&n.entry_uid) {
            order.push(n.entry_uid.clone());
        }
    }
    for (p, n) in near {
        if !order.contains(&n.entry_uid) {
            order.push(n.entry_uid.clone());
        }
        extra.entry(n.entry_uid).or_default().push(p);
    }
    order.truncate(RECALL_ENTRIES);
    let mut items = Vec::new();
    let mut picked: BTreeMap<String, Entry> = BTreeMap::new();
    for uid in &order {
        let Some(e) = by_uid(conn, uid)? else {
            continue;
        };
        let translated = e.names.get(lang).map(String::as_str).unwrap_or("");
        let shown = if translated.is_empty() {
            e.name.clone()
        } else {
            format!("{} → {translated}", e.name)
        };
        let mut line = String::new();
        line.push_str(&entry_line(&e, &shown, vi));
        line.push('\n');
        let mut terms: Vec<String> = std::iter::once(e.name.clone())
            .chain(e.aliases.iter().cloned())
            .chain((!translated.is_empty()).then(|| translated.to_string()))
            .chain(extra.remove(uid).unwrap_or_default())
            .filter(|t| !t.trim().is_empty())
            .collect();
        terms.dedup();
        items.push(Recalled {
            uid: e.uid.clone(),
            terms,
            line,
        });
        picked.insert(e.uid.clone(), e);
    }
    let rels = relations(conn, novel_id)?
        .into_iter()
        .filter_map(|r| {
            let a = picked.get(&r.from_uid)?;
            let b = picked.get(&r.to_uid)?;
            let name = |e: &Entry| {
                e.names
                    .get(lang)
                    .filter(|n| vi && !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| e.name.clone())
            };
            let mut line = if vi {
                format!("- {} {} {}", name(a), rel_words(&r, lang), name(b))
            } else {
                format!("- {} — {} — {}", a.name, rel_words(&r, lang), b.name)
            };
            if !r.note.trim().is_empty() {
                line.push_str(&format!(" ({})", head(r.note.trim(), 160)));
            }
            line.push('\n');
            Some((r.from_uid.clone(), r.to_uid.clone(), line))
        })
        .collect();
    Ok(Recall { vi, items, rels })
}

impl Recall {
    /// This recall with what `base` found that it did not: a part's own recall, made
    /// just before it is sent so it has the names earlier parts put in the wiki, with
    /// the near names the chapter's recall found by embedding.
    pub fn with(mut self, base: &Recall) -> Recall {
        for i in &base.items {
            if !self.items.iter().any(|m| m.uid == i.uid) {
                self.items.push(i.clone());
            }
        }
        for r in &base.rels {
            if !self.rels.iter().any(|m| m.0 == r.0 && m.1 == r.1) {
                self.rels.push(r.clone());
            }
        }
        self
    }

    /// The names part of the prompt for one part of the chapter: only the entries the
    /// part names, the most mentioned first, and the relationships between them.
    pub fn prompt(&self, text: &str) -> String {
        if self.items.is_empty() {
            return String::new();
        }
        let finder = super::names::Finder::new(
            &self
                .items
                .iter()
                .map(|i| i.terms.clone())
                .collect::<Vec<_>>(),
        );
        let mut hits: Vec<(usize, &Recalled)> = finder
            .hits(text)
            .into_iter()
            .zip(&self.items)
            .filter(|(h, _)| *h > 0)
            .collect();
        if hits.is_empty() {
            return String::new();
        }
        hits.sort_by_key(|(h, _)| std::cmp::Reverse(*h));
        hits.truncate(CONTEXT_ENTRIES);
        let mut s = String::from(if self.vi {
            "\nDANH SÁCH TÊN RIÊNG trong đoạn này (tên gốc → tên viết trong bản tiếng Việt, loại). Luôn viết đúng như vậy; tên gọi khác của một mục chỉ người hay vật đó, giữ tên gọi khác khi văn bản dùng nó như biệt danh:\n"
        } else {
            "\nProper nouns in this text (original → as this translation writes it, kind); always write them this way, and an alias the same as its entry's name unless the text uses it as a nickname:\n"
        });
        for (_, i) in &hits {
            s.push_str(&i.line);
        }
        let here = |uid: &str| hits.iter().any(|(_, i)| i.uid == uid);
        let rels: Vec<&str> = self
            .rels
            .iter()
            .filter(|(a, b, _)| here(a) && here(b))
            .map(|(_, _, l)| l.as_str())
            .take(CONTEXT_ENTRIES)
            .collect();
        if !rels.is_empty() {
            s.push_str(if self.vi {
                "Quan hệ:\n"
            } else {
                "Relationships:\n"
            });
            s.extend(rels);
        }
        s
    }
}

/* ---------- Learning from a chapter ---------- */

/// What `Meta` holds, for the JSON part of a prompt.
const META_VI: &str = "{\"system\": tên thang cấp bậc (chỉ cho level, ví dụ hệ tu luyện), \"level\": cảnh giới hiện tại (chỉ cho character), \"stage\": bậc nhỏ trong cảnh giới (sơ kỳ 1, trung kỳ 2, hậu kỳ 3, đại viên mãn 4), \"stage_name\": tên bậc nhỏ, \"personality\": tính cách (chỉ cho character), \"age\": tuổi tác (chỉ cho character, ví dụ 15 tuổi, thiếu niên), \"origin\": xuất thân (chỉ cho character: gia tộc, quê quán, thân phận), \"abilities\": kỹ năng, sở trường (chỉ cho character), \"grade\": đẳng cấp của công pháp như truyện xếp hạng (chỉ cho skill, ví dụ Thiên giai, Huyền cấp thượng phẩm), \"difficulty\": độ khó tu luyện hay sử dụng (chỉ cho skill)}, mỗi mục hồ sơ một hai câu tiếng Việt, gồm cả điều đã biết trước đó; bỏ trống khi không rõ";
const META_EN: &str = "{\"system\": the ladder's name (level only, e.g. the cultivation system), \"level\": current level (character only), \"stage\": step inside the level (early 1, middle 2, late 3, peak 4), \"stage_name\": that step's name, \"personality\": personality (character only), \"age\": age (character only, e.g. 15, a teenager), \"origin\": background (character only: family, home, standing), \"abilities\": skills and strengths (character only), \"grade\": the technique's grade as the story ranks them (skill only), \"difficulty\": how hard it is to learn or use (skill only)}, each profile field one or two sentences, earlier facts included; left out when unknown";

fn meta_schema(vi: bool) -> Value {
    let t = |v: &str, e: &str| if vi { v.to_string() } else { e.to_string() };
    json!({
        "type": "object",
        "description": t("Chỉ cho level và character; bỏ trống khi không rõ.", "Only for a level or a character; left out when unknown."),
        "properties": {
            "system": { "type": "string", "description": t("Level: tên thang cấp bậc mà cảnh giới thuộc về (một truyện có thể có nhiều thang).", "Level: the ladder it belongs to (a story can have several).") },
            "level": { "type": "string", "description": t("Character: tên cảnh giới hiện tại, đúng như văn bản gốc viết.", "Character: its current level, as the original writes it.") },
            "stage": { "type": "integer", "description": t("Bậc nhỏ trong cảnh giới: sơ kỳ 1, trung kỳ 2, hậu kỳ 3, đại viên mãn 4.", "Step inside the level: early 1, middle 2, late 3, peak 4.") },
            "stage_name": { "type": "string", "description": t("Tên bậc nhỏ như văn bản viết.", "That step's name as the text writes it.") },
            "personality": { "type": "string", "description": t("Character: tính cách, một hai câu tiếng Việt.", "Character: its personality, one or two sentences.") },
            "age": { "type": "string", "description": t("Character: tuổi tác, ngắn gọn (15 tuổi, thiếu niên, lão giả trăm tuổi).", "Character: its age, short (15, a teenager, a hundred-year-old elder).") },
            "origin": { "type": "string", "description": t("Character: xuất thân (gia tộc, quê quán, thân phận), một hai câu tiếng Việt.", "Character: its background (family, home, standing), one or two sentences.") },
            "abilities": { "type": "string", "description": t("Character: kỹ năng, sở trường, năng lực đặc biệt, một hai câu tiếng Việt; công pháp có tên thì ghi thêm thành mục skill và quan hệ has_skill.", "Character: its skills, strengths and special powers, one or two sentences; a named technique is also its own skill entry with a has_skill relation.") },
            "grade": { "type": "string", "description": t("Skill: đẳng cấp của công pháp như truyện xếp hạng (Thiên giai, Địa giai, Huyền cấp thượng phẩm…), ngắn gọn.", "Skill: its grade as the story ranks techniques, short.") },
            "difficulty": { "type": "string", "description": t("Skill: độ khó tu luyện hay sử dụng, điều kiện, cái giá phải trả; một câu tiếng Việt.", "Skill: how hard it is to learn or use, what it needs or costs; one sentence.") }
        }
    })
}

/// The tools a translation or rewrite request offers: `nop_ban_viet` for the text,
/// `ghi_wiki` for the facts of the part (the shape `extract_prompt` asks for as JSON,
/// read the same way) and `tra_ten` to look up a name the prompt does not list.
pub fn part_tools(lang: &str) -> Value {
    let vi = lang == "vi";
    let l = super::translate::lang_name(lang);
    let t = |v: &str, e: String| if vi { v.to_string() } else { e };
    let s = |d: String| json!({ "type": "string", "description": d });
    let entry = json!({
        "type": "object",
        "properties": {
            "name": s(t("Tên đúng như văn bản gốc viết.", "The name exactly as the original writes it.".into())),
            "kind": { "type": "string", "enum": KINDS },
            "translated": s(t("Tên dùng trong bản tiếng Việt.", format!("The name as the {l} text writes it."))),
            "aliases": { "type": "array", "items": { "type": "string" }, "description": t("Biệt danh, danh hiệu, cách gọi khác của cùng người hay vật này trong văn bản gốc.", "Nicknames, titles and other ways the original refers to this same one.".into()) },
            "summary": s(t("Tối đa ba câu tiếng Việt.", format!("At most three sentences in {l}."))),
            "rank": { "type": "integer", "description": t("Chỉ cho level: thứ bậc trong thang, 1 là thấp nhất.", "Level only: its place in its ladder, 1 for the lowest.".into()) },
            "meta": meta_schema(vi)
        },
        "required": ["name", "kind"]
    });
    let relation = json!({
        "type": "object",
        "properties": {
            "from": s(t("Tên gốc.", "Original name.".into())),
            "to": s(t("Tên gốc.", "Original name.".into())),
            "relation": s(if vi { format!("Một trong {RELS_VI}; không loại nào hợp thì vài chữ tiếng Việt.") } else { format!("One of {}; else a few words in {l}.", REL_TYPES.join(", ")) }),
            "note": s(t("Hoàn cảnh, một câu tiếng Việt.", format!("The circumstances, one sentence in {l}.")))
        },
        "required": ["from", "to", "relation"]
    });
    let person = json!({
        "type": "object",
        "properties": {
            "name": s(t("Tên gốc của nhân vật.", "Original name of the character.".into())),
            "gender": { "type": "string", "enum": ["male", "female", "unknown"] },
            "age": { "type": "string", "enum": ["child", "teen", "young adult", "adult", "middle-aged", "elder"] }
        },
        "required": ["name"]
    });
    let address = json!({
        "type": "object",
        "properties": {
            "from": s(t("Tên gốc người nói.", "Original name of the speaker.".into())),
            "to": s(t("Tên gốc người nghe.", "Original name of the one spoken to.".into())),
            "self_raw": s(t("Người nói tự xưng thế nào trong văn bản gốc.", "How the speaker calls themself in the original.".into())),
            "calls_raw": s(t("Người nói gọi người nghe thế nào trong văn bản gốc.", "How the speaker calls the other in the original.".into())),
            "self": s(t("Tự xưng trong bản tiếng Việt.", format!("The same in the {l} text."))),
            "calls": s(t("Cách gọi trong bản tiếng Việt.", format!("The same in the {l} text."))),
            "tone": { "type": "string", "enum": ["respectful", "familiar", "intimate", "cold", "hostile", "playful"] }
        },
        "required": ["from", "to"]
    });
    json!([
        {
            "type": "function",
            "function": {
                "name": PART_TEXT,
                "description": t("Nộp văn bản đã viết: mỗi đoạn vào là đúng một phần tử của doan, cùng thứ tự.", "Hand in the written text: one item of doan per paragraph given, in the same order.".into()),
                "parameters": {
                    "type": "object",
                    "properties": { "doan": { "type": "array", "items": { "type": "string" } } },
                    "required": ["doan"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": PART_FACTS,
                "description": t("Ghi các danh từ riêng, quan hệ, nhân vật và cách xưng hô của đoạn này. Không ghi danh từ chung.", "Record the proper nouns, relationships, characters and forms of address of this part. No common nouns.".into()),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entries": { "type": "array", "items": entry },
                        "relations": { "type": "array", "items": relation },
                        "people": { "type": "array", "items": person },
                        "address": { "type": "array", "items": address }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": PART_LOOKUP,
                "description": t("Tra một tên, biệt danh hay cảnh giới trong wiki của truyện khi nó không có trong danh sách tên riêng.", "Look up a name, nickname or level in the book's wiki when the names list does not have it.".into()),
                "parameters": {
                    "type": "object",
                    "properties": { "ten": { "type": "string" } },
                    "required": ["ten"]
                }
            }
        }
    ])
}

pub const PART_TEXT: &str = "nop_ban_viet";
pub const PART_FACTS: &str = "ghi_wiki";
pub const PART_LOOKUP: &str = "tra_ten";

/// What a translation or rewrite prompt adds when the request offers `part_tools`:
/// the wiki's rules and how to answer through the tools.
pub fn tool_prompt(lang: &str, genre: &str) -> String {
    if lang == "vi" {
        return format!(
            "\n\nCÁCH TRẢ LỜI: chỉ trả lời bằng tool. Nộp văn bản đã viết qua {PART_TEXT}: mỗi đoạn vào là đúng một phần tử của doan, cùng thứ tự, không thêm gì khác. Cùng lúc đó ghi dữ kiện của đoạn qua {PART_FACTS} (không có gì đáng ghi thì bỏ qua). Nếu đoạn có một tên không nằm trong danh sách tên riêng mà bạn cần biết là ai để viết đúng, gọi {PART_LOOKUP} trước rồi mới nộp. Không bao giờ trả lời bằng lời, không hỏi lại, không xin văn bản khác: luôn nộp kết quả qua {PART_TEXT}.\n\nGHI WIKI. {}",
            learn_rules(lang, genre)
        );
    }
    format!(
        "\n\nHOW TO ANSWER: answer only through the tools. Hand in the text through {PART_TEXT}: one item of doan per paragraph given, in the same order, nothing else. At the same time record the part's facts through {PART_FACTS} (skip it when there is nothing worth keeping). When the part has a name the names list does not have and you need to know who it is to write it right, call {PART_LOOKUP} first, then hand in. Never answer in words, never ask anything back, never ask for another text: always hand in the result through {PART_TEXT}.\n\nKEEP THE WIKI. {}",
        learn_rules(lang, genre)
    )
}

/// The answer to `tra_ten`: the entries the wiki has under that name (by name, alias
/// or translation; else the nearest name in the vector table), with their relations.
pub fn lookup(
    conn: &Connection,
    _llm: Option<&dyn Chat>,
    novel_id: i64,
    lang: &str,
    query: &str,
) -> rusqlite::Result<Value> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(json!([]));
    }
    let mut found: Vec<Entry> = search(conn, novel_id, q, 3).unwrap_or_default();
    if found.is_empty() {
        let near = near_names(conn, novel_id, lang, q, &[])?;
        for (_, n) in near {
            if let Some(e) = by_uid(conn, &n.entry_uid)? {
                if !found.iter().any(|f| f.uid == e.uid) {
                    found.push(e);
                }
            }
        }
    }
    let rels = relations(conn, novel_id)?;
    let name_of = |uid: &str| -> rusqlite::Result<String> {
        Ok(by_uid(conn, uid)?
            .map(|e| e.names.get(lang).cloned().unwrap_or(e.name))
            .unwrap_or_default())
    };
    let mut out = Vec::new();
    for e in found {
        let mut lines = Vec::new();
        for r in rels
            .iter()
            .filter(|r| r.from_uid == e.uid || r.to_uid == e.uid)
            .take(12)
        {
            let mut l = format!(
                "{} {} {}",
                name_of(&r.from_uid)?,
                rel_words(r, lang),
                name_of(&r.to_uid)?
            );
            if !r.note.trim().is_empty() {
                l.push_str(&format!(" ({})", head(r.note.trim(), 160)));
            }
            lines.push(l);
        }
        out.push(json!({
            "name": e.name, "translated": e.names.get(lang), "kind": e.kind,
            "aliases": e.aliases, "summary": e.summary_in(lang), "rank": e.rank,
            "meta": e.meta, "relations": lines,
        }));
    }
    Ok(json!(out))
}

fn tools(lang: &str) -> Value {
    let vi = lang == "vi";
    let lang = super::translate::lang_name(lang);
    let t = |v: &str, e: String| if vi { v.to_string() } else { e };
    json!([
        {
            "type": "function",
            "function": {
                "name": "wiki_search",
                "description": t("Tìm mục trong wiki của truyện theo tên, tên gọi khác hoặc mô tả.", "Find entries of this novel's wiki by name, alias or description.".to_string()),
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
                "description": t("Thêm một mục, hoặc bổ sung thông tin cho mục đã có (khớp theo tên gốc hoặc tên gọi khác).", "Add an entry, or add new facts to an existing one (matched by its original name or an alias).".to_string()),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": t("Tên đúng như văn bản gốc viết.", "The name exactly as written in the original text.".to_string()) },
                        "kind": { "type": "string", "enum": KINDS },
                        "translated": { "type": "string", "description": t("Tên dùng trong bản tiếng Việt.", format!("The name as used in the {lang} translation.")) },
                        "aliases": { "type": "array", "items": { "type": "string" }, "description": t("Mọi cách khác văn bản gốc dùng để chỉ cùng người hay vật này: biệt danh, danh hiệu, cách gọi, mô tả thay cho tên.", "Every other way the original text refers to this same one: nicknames, titles, epithets, descriptions that stand for it (\"the girl in green\").".to_string()) },
                        "summary": { "type": "string", "description": t("Bằng tiếng Việt, tối đa ba câu: người hay vật này là gì, theo những gì đã biết. Viết lại cả tóm tắt, gồm cả thông tin cũ.", format!("In {lang}, at most three sentences: who or what it is, as known so far. Write the whole summary, old facts included.")) },
                        "rank": { "type": "integer", "description": t("Chỉ cho loại level: thứ bậc trong thang cảnh giới hay cấp bậc, 1 là thấp nhất.", "Only for kind level: its place in its ladder of ranks or cultivation realms, 1 for the lowest.".to_string()) },
                        "meta": meta_schema(vi)
                    },
                    "required": ["name", "kind"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "wiki_relate",
                "description": t("Ghi quan hệ giữa hai mục, theo tên gốc.", "Record a relationship between two entries, by their original names.".to_string()),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "string" },
                        "to": { "type": "string" },
                        "relation": { "type": "string", "description": if vi { format!("`from` là gì của `to`: một trong {RELS_VI}. Chỉ khi không loại nào hợp, viết vài chữ tiếng Việt.") } else { format!("What `from` is to `to`: one of {} (located_in for a place or faction inside a bigger place, has_level for a character's current rank or realm, has_skill for a technique or ability a character knows, equivalent_to for a level matching a level of another ladder). Only when none fits, a few words in {lang}.", REL_TYPES.join(", ")) } },
                        "note": { "type": "string", "description": t("Bằng tiếng Việt, một câu: hoàn cảnh (vì sao có quan hệ này, hai bên gọi nhau thế nào).", format!("In {lang}: the circumstances (how it came about, what one calls the other), one sentence.")) }
                    },
                    "required": ["from", "to", "relation"]
                }
            }
        }
    ])
}

fn learn_rules(lang: &str, genre: &str) -> String {
    if lang == "vi" {
        let names = if genre == "nonfiction" {
            " Đây là sách tài liệu thực tế: tên dịch luôn là tên gốc tiếng Anh, không dịch."
        } else {
            ""
        };
        return format!(
            "Bạn ghi wiki cho một truyện đang được chuyển sang tiếng Việt. Từ văn bản dưới đây, chỉ ghi các danh từ riêng có vai trò trong truyện: tên riêng của nhân vật, địa danh, thế lực hay tông môn, vật phẩm, cảnh giới hay cấp bậc (kèm số bậc, tên thang cấp bậc và bậc nhỏ trong meta) và công pháp hay chiêu thức có tên. Không ghi danh từ chung hay từ mô tả, dù quan trọng với cảnh truyện: một nơi hay một vật chỉ được ghi khi nó có tên riêng. Tên riêng thường viết hoa; cụm không viết hoa hoặc chỉ là mô tả (người, đồ vật, nơi chốn chung chung) thì không phải tên riêng. Ghi mỗi tên trọn vẹn, đúng như văn bản gốc viết, không cắt một phần của từ hay tên dài hơn. Biệt danh, danh hiệu, cách gọi hay mô tả dùng để chỉ một nhân vật hay nơi đã có tên thì không phải mục mới: ghi vào tên gọi khác (aliases) của mục đó. Với nhân vật, ghi cảnh giới hiện tại vào meta (level, stage) và hồ sơ vào meta: personality (tính cách), age (tuổi tác), origin (xuất thân), abilities (kỹ năng, sở trường), mỗi mục một hai câu, viết lại cả điều đã biết; summary chỉ là một hai câu giới thiệu nhân vật là ai, không lặp lại hồ sơ. Với công pháp, chiêu thức (skill): summary mô tả nó ra sao, làm được gì; ghi đẳng cấp vào meta.grade và độ khó vào meta.difficulty. Với cảnh giới: khi truyện cho biết một cảnh giới tương đương với cảnh giới ở thang cấp bậc khác (ví dụ tu tiên và luyện thể), ghi quan hệ equivalent_to giữa hai cảnh giới đó. Với thế lực: ghi leader_of từ mỗi người đứng đầu đến thế lực, kể cả người tiền nhiệm và người kế nhiệm khi truyện tiết lộ hay đổi người; note ghi người đó là đời thứ mấy (nếu truyện cho biết) và nắm quyền khi nào. Ghi các quan hệ: giữa nhân vật với nhau, với thế lực, cảnh giới hiện tại, công pháp nhân vật biết, nơi nào nằm trong nơi nào; mỗi quan hệ kèm ghi chú bằng tiếng Việt về hoàn cảnh (vì sao có quan hệ đó, hai bên gọi nhau thế nào). Viết mọi tên dịch, tóm tắt, quan hệ tự do và ghi chú chỉ bằng tiếng Việt, không trộn ngôn ngữ. Ghi hết những gì văn bản cho biết về một mục, kể cả tình tiết về sau (không cần tránh tiết lộ nội dung), và liên kết nó với các mục liên quan; không đoán.{names}"
        );
    }
    let l = super::translate::lang_name(lang);
    let names = if genre == "nonfiction" {
        " This book is non-fiction: the translated name is always the original English name, never translated.".to_string()
    } else {
        String::new()
    };
    format!(
        "You keep the wiki of a novel being translated into {l}. From the chapter below, record only proper nouns that matter to the story: the names of specific characters, places, factions or sects, items, levels (ranks, cultivation realms, each with its rank number, its ladder and stage in meta) and skills (named techniques, martial arts, spells). Never record a common noun or a generic word, however important to the scene (mountain, river, sword, city, sect, spirit energy, núi, sông, kiếm, thành, linh khí, tông môn): a place or thing is kept only under its own name. Write each name whole and exactly as the original text writes it, never a piece of a longer word or name (not a surname cut out of another word). A nickname, title, epithet or description that stands for a character or place already named (\"the girl in green\", \"bé áo xanh\", \"the old man\", \"Sư phụ\" for one person) is not a new entry: put it in that entry's aliases. For a character, put its current level in meta (level, stage) and its profile in meta: personality, age, origin (background), abilities (skills and strengths), one or two sentences each, earlier facts included; its summary then only says in a sentence or two who it is, without repeating the profile. For a skill, the summary says what it is and does; put its grade in meta.grade and how hard it is in meta.difficulty. For levels, when the story says a level of one ladder matches a level of another, record equivalent_to between them. For a faction, record leader_of from each of its leaders, predecessors and successors included when the story reveals or changes them; the note says which leader in line they are (when the story says) and when they led. Record the relationships: between characters, with factions, a character's current level, the skills a character has, which place lies in which (the map), each with a note in {l} on the circumstances (how it came about, what one calls the other). Write every translated name, summary, free relation and note in {l} only, never mixing languages. Record all the chapter shows about an entry, later events included (spoilers are fine), and link it to the others it involves; do not guess.{names}"
    )
}

fn learn_prompt(lang: &str, genre: &str) -> String {
    if lang == "vi" {
        return format!(
            "{} Dùng các công cụ: wiki_search để xem wiki đã biết gì (tìm cả biệt danh trước khi thêm mục mới), wiki_upsert để thêm hoặc cập nhật một mục, wiki_relate để ghi quan hệ (thêm cả hai mục trước). Khi wiki đã cập nhật xong, trả lời DONE.",
            learn_rules(lang, genre)
        );
    }
    format!(
        "{} Use the tools: wiki_search to check what is known (search a nickname too before adding it as new), wiki_upsert to add or update an entry, wiki_relate for relationships (add both entries first). When the wiki is up to date, reply with DONE.",
        learn_rules(lang, genre)
    )
}

/// The JSON object the AI answers with when it does not use the tools.
fn json_shape(lang: &str) -> String {
    if lang == "vi" {
        return format!(
            "{{\"entries\": [{{\"name\": tên gốc, \"kind\": một trong {KINDS_VI}, \"translated\": tên trong bản tiếng Việt, \"aliases\": [biệt danh, danh hiệu và các cách văn bản gốc gọi mục này], \"summary\": tối đa ba câu tiếng Việt, \"rank\": số bậc, chỉ cho level, \"meta\": {META_VI}}}], \"relations\": [{{\"from\": tên gốc, \"to\": tên gốc, \"relation\": một trong {RELS_VI} (nếu không có loại nào hợp thì vài chữ tiếng Việt), \"note\": hoàn cảnh, bằng tiếng Việt}}]}}"
        );
    }
    format!(
        "{{\"entries\": [{{\"name\": original name, \"kind\": one of {}, \"translated\": name in the translation, \"aliases\": [nicknames, titles and other ways the original refers to it], \"summary\": at most three sentences in {l}, \"rank\": number for a level only, \"meta\": {META_EN}}}], \"relations\": [{{\"from\": original name, \"to\": original name, \"relation\": one of {} (else a few words in {l}), \"note\": the circumstances in {l}}}]}}",
        KINDS.join(", "),
        REL_TYPES.join(", "),
        l = super::translate::lang_name(lang),
    )
}

fn json_prompt(lang: &str, genre: &str) -> String {
    if lang == "vi" {
        return format!(
            "{} Chỉ trả về đúng một đối tượng JSON, không gì khác (giữ nguyên tên khóa và mã tiếng Anh): {}",
            learn_rules(lang, genre),
            json_shape(lang)
        );
    }
    format!(
        "{} Reply with only one JSON object and nothing else: {}",
        learn_rules(lang, genre),
        json_shape(lang)
    )
}

/// What a translation prompt adds so the same request also feeds the wiki: after the
/// translated text, `mark` on its own line and the facts of that part as JSON.
pub fn extract_prompt(lang: &str, mark: &str, genre: &str) -> String {
    if lang == "vi" {
        return format!(
            "\n\nGHI WIKI. {} Sau phần văn bản đã viết, viết một dòng chỉ có {mark}, rồi đúng một đối tượng JSON, không gì khác (giữ nguyên tên khóa và mã tiếng Anh): {}, thêm hai danh sách: {}. Nếu văn bản không có gì đáng ghi, viết {{\"entries\": [], \"relations\": []}}.",
            learn_rules(lang, genre),
            json_shape(lang),
            super::cast::shape(lang)
        );
    }
    format!(
        "\n\nAlso keep the novel's wiki. {} After the translated text, write a line with only {mark}, then one JSON object and nothing else: {}, with two more lists in it: {}. When the text shows nothing worth keeping, write {{\"entries\": [], \"relations\": []}}.",
        learn_rules(lang, genre),
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
        s.push_str(if lang == "vi" {
            "Wiki đã có:\n"
        } else {
            "Already in the wiki:\n"
        });
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
    let vi = lang == "vi";
    s.push_str(if vi {
        "Chương gốc:\n"
    } else {
        "Original chapter:\n"
    });
    s.push_str(&head(original, ORIGINAL_CHARS));
    if !translated.is_empty() {
        s.push_str(if vi {
            "\n\nBản tiếng Việt:\n"
        } else {
            "\n\nIts translation:\n"
        });
        s.push_str(&head(translated, TRANSLATED_CHARS));
    }
    Ok(s)
}

/// Whether the API or model does not take tools, so the JSON way is used instead.
pub(crate) fn tools_refused(e: &AiError) -> bool {
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
    let genre = super::translate::prefs(conn, novel_id)?.genre;
    let mut messages = vec![
        json!({ "role": "system", "content": learn_prompt(lang, &genre) }),
        json!({ "role": "user", "content": input }),
    ];
    let mut changes = 0;
    for round in 0..MAX_ROUNDS {
        let msg = match llm.chat_tools(&messages, &tools) {
            Ok(m) => m,
            Err(e) if round == 0 && tools_refused(&e) => {
                let reply = AiError::keep_cut(llm.chat(&json_prompt(lang, &genre), &input))?;
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

/// Puts what one part of a chapter found (its `ghi_wiki` call, or the facts after its
/// text) in the wiki right away, so the parts and chapters after it find those names
/// and write them the same way. A name already in the wiki keeps its translation.
/// `read_lang` is the language the chapter is translated into, whose names the parts
/// are looked up by. Nothing when the book's wiki does not learn.
pub fn learn_part(
    conn: &Connection,
    novel_id: i64,
    chapter_id: i64,
    read_lang: &str,
    facts: &str,
) -> Result<usize, AiError> {
    if !auto(conn, novel_id)? {
        return Ok(0);
    }
    let Some(answer) = parse_object::<AiAnswer>(facts) else {
        return Ok(0);
    };
    let chapter: Option<i64> = conn
        .query_row(
            "select number from chapters where id = ?1",
            [chapter_id],
            |r| r.get(0),
        )
        .optional()?;
    let mut changes = 0;
    for item in answer.entries {
        if upsert_ai(conn, novel_id, WIKI_LANG, chapter, item)?.is_some() {
            changes += 1;
        }
    }
    for item in &answer.relations {
        if relate_ai(conn, novel_id, WIKI_LANG, chapter, item)?.is_ok() {
            changes += 1;
        }
    }
    if changes > 0 {
        index_names(conn, None, novel_id, WIKI_LANG)?;
        if read_lang != WIKI_LANG {
            index_names(conn, None, novel_id, read_lang)?;
        }
    }
    Ok(changes)
}

/// The names a chapter's parts wrote differently from the wiki: (as written, as the
/// wiki keeps it), for the entries the parts named by their own name. Two parts that
/// ran at the same time may each have translated a new name their own way; the first
/// one kept wins and the chapter is made to agree with it. A pair where one name holds
/// the other ("Lâm" and "Lâm Động") is left alone.
pub fn settled_names(
    conn: &Connection,
    novel_id: i64,
    found: &[String],
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    for answer in found.iter().filter_map(|f| parse_object::<AiAnswer>(f)) {
        for item in answer.entries {
            let (name, written) = (item.name.trim(), item.translated.trim());
            if name.is_empty() || written.chars().count() < 2 {
                continue;
            }
            let Some((_, e, false)) = find(conn, novel_id, name)? else {
                continue;
            };
            if e.name != name {
                continue;
            }
            let Some(kept) = e.names.get(WIKI_LANG).map(|k| k.trim()) else {
                continue;
            };
            if kept.is_empty() || kept.contains(written) || written.contains(kept) {
                continue;
            }
            if !out.iter().any(|(w, _)| w == written) {
                out.push((written.to_string(), kept.to_string()));
            }
        }
    }
    Ok(out)
}

/// `text` with every `from` that stands as whole words replaced by `to`.
pub fn replace_name(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() || !text.contains(from) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let before = rest[..at]
            .chars()
            .next_back()
            .or_else(|| out.chars().next_back());
        let after = rest[at + from.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        out.push_str(&rest[..at]);
        if word(before) || word(after) {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = &rest[at + from.len()..];
    }
    out.push_str(rest);
    out
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
    }
    .and_then(|n| {
        // The vector table follows, and names it finds written two ways are merged.
        if index_names(conn, Some(llm), novel_id, lang)? > 0 {
            merge_near(conn, novel_id, lang)?;
        }
        Ok(n)
    });
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

/// Drops every language but Vietnamese from a book's wiki where the Vietnamese words are
/// there (see migration 22); returns how many rows changed.
pub fn only_vi(conn: &Connection, novel_id: i64) -> rusqlite::Result<usize> {
    let mut n = 0;
    for (table, col) in [
        ("wiki_entries", "summaries"),
        ("wiki_entries", "names"),
        ("wiki_relations", "labels"),
    ] {
        n += conn.execute(
            &format!(
                "update {table} set {col} = json_object('vi', json_extract({col}, '$.vi'))
                 where novel_id = ?1 and json_valid({col}) and json_extract({col}, '$.vi') is not null
                   and (select count(*) from json_each({table}.{col})) > 1"
            ),
            [novel_id],
        )?;
    }
    Ok(n)
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
    let error = match AiError::keep_cut(llm.chat(system, &user.to_string())) {
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
        "delete from wiki_books; delete from wiki_chapters; delete from wiki_names;
         delete from ai_book_prefs;
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
        // Words for a relation type land on the type.
        assert_eq!(w.relations[0].kind, "disciple_of");
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
                meta: Default::default(),
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
                meta: Default::default(),
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
                meta: Default::default(),
            },
        )
        .unwrap();
        add_relation(
            &conn,
            novel,
            &b.uid,
            &a.uid,
            "kẻ thù của",
            "vi",
            "Tranh ngôi gia chủ.",
        )
        .unwrap();
        index_names(&conn, None, novel, "vi").unwrap();
        let ctx = context(&conn, novel, "动少 看着 林琅天。", "vi").unwrap();
        // Names only, no descriptions: raw → translated (kind).
        assert!(ctx.contains("- 林动 → Lâm Động (nhân vật); tên gọi khác: 动少\n"));
        assert!(!ctx.contains("Nhân vật chính"));
        assert!(ctx.contains("林琅天 là kẻ thù của Lâm Động (Tranh ngôi gia chủ.)"));
        assert!(!ctx.contains("炎城"));
        assert_eq!(context(&conn, novel, "nothing here", "vi").unwrap(), "");
        // The vector table holds every name and alias with the translation and kind.
        let names = names_in(&conn, novel, "vi", "动少 林动").unwrap();
        assert_eq!(names.len(), 2);
        assert!(names
            .iter()
            .all(|n| n.translated == "Lâm Động" && n.kind == "character"));
    }

    fn ai(name: &str, kind: &str, translated: &str, aliases: &[&str]) -> AiEntry {
        AiEntry {
            name: name.into(),
            kind: kind.into(),
            translated: translated.into(),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
            summary: String::new(),
            rank: None,
            meta: Default::default(),
        }
    }

    #[test]
    fn characters_keep_a_profile_that_later_chapters_fill_in() {
        let (conn, novel) = book("profile");
        let lam = AiEntry {
            meta: serde_json::from_value(json!({
                "personality": "Kiên nhẫn, trọng tình nghĩa.", "age": "15 tuổi",
                "origin": "Con trai Lâm Khiếu, Lâm gia ở Thanh Dương trấn.", "system": "x"
            }))
            .unwrap(),
            ..ai("林动", "character", "Lâm Động", &[])
        };
        upsert_ai(&conn, novel, "vi", Some(1), lam).unwrap();
        // A place keeps no profile.
        let town = AiEntry {
            meta: serde_json::from_value(json!({ "personality": "?", "age": "cổ" })).unwrap(),
            ..ai("青阳镇", "place", "Thanh Dương trấn", &[])
        };
        upsert_ai(&conn, novel, "vi", Some(1), town).unwrap();
        // A later chapter adds what he is good at and his new age; the rest stays.
        let later = AiEntry {
            meta: serde_json::from_value(
                json!({ "age": "16 tuổi", "abilities": "Khắc phù, luyện thể." }),
            )
            .unwrap(),
            ..ai("林动", "character", "Lâm Động", &[])
        };
        upsert_ai(&conn, novel, "vi", Some(2), later).unwrap();
        let w = load(&conn, novel).unwrap();
        let lam = w.entries.iter().find(|e| e.name == "林动").unwrap();
        assert_eq!(lam.meta.personality, "Kiên nhẫn, trọng tình nghĩa.");
        assert_eq!(lam.meta.age, "16 tuổi");
        assert_eq!(
            lam.meta.origin,
            "Con trai Lâm Khiếu, Lâm gia ở Thanh Dương trấn."
        );
        assert_eq!(lam.meta.abilities, "Khắc phù, luyện thể.");
        assert_eq!(lam.meta.system, "");
        let town = w.entries.iter().find(|e| e.name == "青阳镇").unwrap();
        assert_eq!(town.meta, Meta::default());
        // The app sends it back as it got it.
        let v = serde_json::to_value(&lam.meta).unwrap();
        assert_eq!(v["personality"], "Kiên nhẫn, trọng tình nghĩa.");
        assert!(v.get("system").is_none());
        // A skill keeps its grade and how hard it is, nothing of a character's.
        let fire = AiEntry {
            meta: serde_json::from_value(json!({
                "grade": "Thiên giai", "difficulty": "Phải có Thôn Phệ Tổ Phù mới luyện được.", "age": "x"
            }))
            .unwrap(),
            ..ai("吞噬祖符", "skill", "Thôn Phệ Tổ Phù", &[])
        };
        upsert_ai(&conn, novel, "vi", Some(3), fire).unwrap();
        let w = load(&conn, novel).unwrap();
        let fire = w.entries.iter().find(|e| e.name == "吞噬祖符").unwrap();
        assert_eq!(fire.meta.grade, "Thiên giai");
        assert_eq!(
            fire.meta.difficulty,
            "Phải có Thôn Phệ Tổ Phù mới luyện được."
        );
        assert_eq!(fire.meta.age, "");
    }

    #[test]
    fn levels_keep_their_ladder_and_characters_their_level() {
        let (conn, novel) = book("meta");
        let level = |name: &str, tr: &str, system: &str, rank: i64| AiEntry {
            rank: Some(rank),
            meta: serde_json::from_value(json!({ "system": system, "stage": 9, "level": "x" }))
                .unwrap(),
            ..ai(name, "level", tr, &[])
        };
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            level("Kim Đan", "Kim Đan", "Luyện khí sĩ", 9),
        )
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            level("Long Môn", "Long Môn", "Luyện khí sĩ", 8),
        )
        .unwrap();
        let ninh = AiEntry {
            meta: serde_json::from_value(
                json!({ "level": "Kim Đan", "stage_name": "trung kỳ", "system": "y" }),
            )
            .unwrap(),
            ..ai("Ninh Diêu", "character", "Ninh Diêu", &[])
        };
        upsert_ai(&conn, novel, "vi", Some(1), ninh).unwrap();
        let w = load(&conn, novel).unwrap();
        let get = |n: &str| w.entries.iter().find(|e| e.name == n).unwrap().clone();
        // Only what fits the kind: a ladder for a level, a current level for a character.
        let kd = get("Kim Đan");
        assert_eq!(
            (
                kd.meta.system.as_str(),
                kd.meta.level.as_str(),
                kd.meta.stage
            ),
            ("Luyện khí sĩ", "", Some(9))
        );
        let nd = get("Ninh Diêu");
        assert_eq!(
            (
                nd.meta.level.as_str(),
                nd.meta.system.as_str(),
                nd.meta.stage_name.as_str()
            ),
            ("Kim Đan", "", "trung kỳ")
        );
        // A later chapter moves her up; what it does not say stays.
        let later = AiEntry {
            meta: serde_json::from_value(json!({ "level": "Nguyên Anh" })).unwrap(),
            ..ai("Ninh Diêu", "character", "Ninh Diêu", &[])
        };
        upsert_ai(&conn, novel, "vi", Some(2), later).unwrap();
        let nd = by_uid(&conn, &nd.uid).unwrap().unwrap();
        assert_eq!(
            (nd.meta.level.as_str(), nd.meta.stage_name.as_str()),
            ("Nguyên Anh", "trung kỳ")
        );
        // The prompt says where each stands.
        index_names(&conn, None, novel, "vi").unwrap();
        let ctx = context(&conn, novel, "Ninh Diêu đã vượt Kim Đan.", "vi").unwrap();
        assert!(ctx
            .contains("- Kim Đan → Kim Đan (cảnh giới / cấp bậc; Luyện khí sĩ, bậc 9, bậc nhỏ 9)"));
        assert!(ctx.contains("- Ninh Diêu → Ninh Diêu (nhân vật); cảnh giới: Nguyên Anh, trung kỳ"));
        // The vector table holds it too, and tra_ten answers with it.
        let meta: String = conn
            .query_row(
                "select meta from wiki_names where raw = 'Kim Đan'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(meta.contains("Luyện khí sĩ"));
        let found = lookup(&conn, None, novel, "vi", "Ninh Diêu").unwrap();
        assert_eq!(found[0]["meta"]["level"], "Nguyên Anh");
        assert_eq!(lookup(&conn, None, novel, "vi", "  ").unwrap(), json!([]));
    }

    #[test]
    fn levels_of_two_ladders_are_not_merged() {
        let (conn, novel) = book("ladders");
        let level = |name: &str, system: &str| AiEntry {
            meta: serde_json::from_value(json!({ "system": system })).unwrap(),
            ..ai(name, "level", name, &[])
        };
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            level("Nhất Phẩm", "Luyện đan sư"),
        )
        .unwrap();
        // Names that land together and hold one another: the one of the same ladder
        // merges, the other ladder's stays apart.
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(2),
            level("Nhất Phẩm Võ Giả", "Võ giả"),
        )
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(3),
            level("Nhất Phẩm Đan", "Luyện đan sư"),
        )
        .unwrap();
        let llm = Embedder(Mutex::new(0));
        index_names(&conn, Some(&llm), novel, "vi").unwrap();
        assert_eq!(merge_near(&conn, novel, "vi").unwrap(), 1);
        let w = load(&conn, novel).unwrap();
        assert_eq!(w.entries.len(), 2);
        assert!(w
            .entries
            .iter()
            .any(|e| e.name == "Nhất Phẩm" && e.meta.system == "Luyện đan sư"));
    }

    #[test]
    fn only_proper_nouns_become_entries() {
        let (conn, novel) = book("proper");
        // Common nouns, in a spaced script or by their translation, are left out.
        for item in [
            ai("núi", "place", "núi", &[]),
            ai("sông", "place", "", &[]),
            ai("灵气", "term", "linh khí", &[]),
            ai("Bé gái váy hồng", "character", "Bé gái váy hồng", &[]),
            ai("Tổ trạch", "place", "Tổ trạch", &[]),
            ai("lầu trúc", "place", "lầu trúc", &[]),
            ai("núi Lạc Phách", "place", "", &[]),
            ai("X", "character", "X", &[]),
        ] {
            assert_eq!(upsert_ai(&conn, novel, "vi", Some(1), item).unwrap(), None);
        }
        // Names are kept.
        for item in [
            ai("Thanh Dương trấn", "place", "Thanh Dương trấn", &[]),
            ai("林动", "character", "Lâm Động", &[]),
            ai("道宗", "faction", "", &[]),
            ai("Mặc gia", "faction", "Mặc gia", &[]),
            ai("Ngõ Nê Bình", "place", "Ngõ Nê Bình", &[]),
            ai("林家", "faction", "Lin family", &[]),
        ] {
            assert!(upsert_ai(&conn, novel, "vi", Some(1), item)
                .unwrap()
                .is_some());
        }
        assert_eq!(load(&conn, novel).unwrap().entries.len(), 6);
    }

    #[test]
    fn a_nickname_joins_its_character() {
        let (conn, novel) = book("nick");
        let girl = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            ai("Thanh Đàn", "character", "Thanh Đàn", &[]),
        )
        .unwrap()
        .unwrap();
        // A later chapter brings her nickname with her name as an alias: the same entry.
        let again = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(2),
            ai("Bé Áo Xanh", "character", "Bé Áo Xanh", &["Thanh Đàn"]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(again, girl);
        // The same translated name is the same entry too.
        let third = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(3),
            ai("青檀", "character", "Thanh Đàn", &[]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(third, girl);
        let w = load(&conn, novel).unwrap();
        assert_eq!(w.entries.len(), 1);
        assert_eq!(w.entries[0].aliases, vec!["Bé Áo Xanh", "青檀"]);
    }

    #[test]
    fn two_entries_merge_into_one() {
        let (conn, novel) = book("merge");
        let a = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            ai("Lâm Động", "character", "Lâm Động", &[]),
        )
        .unwrap()
        .unwrap();
        let b = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(3),
            ai("Thiếu Gia", "character", "Thiếu Gia", &[]),
        )
        .unwrap()
        .unwrap();
        let sect = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            ai("Đạo Tông", "faction", "Đạo Tông", &[]),
        )
        .unwrap()
        .unwrap();
        add_relation(&conn, novel, &b, &sect, "member_of", "vi", "").unwrap();
        add_relation(&conn, novel, &a, &b, "friend_of", "vi", "").unwrap();
        let merged = merge(&conn, novel, &b, &a, true).unwrap();
        assert_eq!(merged.aliases, vec!["Thiếu Gia"]);
        assert_eq!(merged.source, "user");
        let w = load(&conn, novel).unwrap();
        assert_eq!(w.entries.len(), 2);
        // The relation moved over; the one between the two is gone.
        assert_eq!(w.relations.len(), 1);
        assert_eq!(
            (
                w.relations[0].from_uid.as_str(),
                w.relations[0].kind.as_str()
            ),
            (a.as_str(), "member_of")
        );
        // The AI naming the merged name again lands on the kept entry.
        let back = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(5),
            ai("Thiếu Gia", "character", "", &[]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(back, a);
        assert!(merge(&conn, novel, &a, &a, true).is_err());
    }

    #[test]
    fn tidy_removes_common_nouns_and_merges_aliases() {
        let (conn, novel) = book("tidy");
        // As an older version wrote them, straight into the table.
        for (uid, name, names, aliases, source) in [
            ("a", "núi", r#"{"vi":"núi"}"#, "[]", "ai"),
            (
                "b",
                "Lâm Động",
                r#"{"vi":"Lâm Động"}"#,
                r#"["Động Thiếu"]"#,
                "ai",
            ),
            ("c", "Động Thiếu", r#"{"vi":"Động Thiếu"}"#, "[]", "ai"),
            ("d", "sông", "{}", "[]", "user"),
        ] {
            conn.execute(
                "insert into wiki_entries (uid, novel_id, kind, name, names, aliases, source, updated_at)
                 values (?1, ?2, 'character', ?3, ?4, ?5, ?6, 1)",
                params![uid, novel, name, names, aliases, source],
            )
            .unwrap();
        }
        assert_eq!(tidy(&conn, novel).unwrap(), 2);
        let names: Vec<String> = load(&conn, novel)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["Lâm Động", "sông"]);
    }

    #[test]
    fn relation_words_land_on_types() {
        assert_eq!(rel_type("Master of"), Some("master_of"));
        assert_eq!(rel_type("sư phụ của"), Some("master_of"));
        assert_eq!(rel_type("Kẻ thù"), Some("enemy_of"));
        assert_eq!(rel_type("là đệ tử của"), Some("disciple_of"));
        assert_eq!(rel_type("ân nhân cứu mạng"), None);
    }

    /// A model for the calls that take one; the vectors are made on the device.
    #[allow(dead_code)]
    struct Embedder(Mutex<usize>);

    impl Chat for Embedder {
        fn chat(&self, _: &str, _: &str) -> Result<String, AiError> {
            Ok(String::new())
        }
        fn model(&self) -> &str {
            "m"
        }
    }

    #[test]
    fn each_part_gets_the_names_it_has_found_through_the_vector_table() {
        let (conn, novel) = book("find");
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            ai("Vương thành", "place", "Vương thành", &[]),
        )
        .unwrap()
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            ai("Lâm Động", "character", "Lâm Động", &["Động thiếu"]),
        )
        .unwrap()
        .unwrap();
        let llm = Embedder(Mutex::new(0));
        index_names(&conn, Some(&llm), novel, "vi").unwrap();
        // The convert wrote the city with a capital T: no exact match, but the vector
        // table finds it.
        let text = "Lâm Động đi vào Vương Thành.\n\nĐộng thiếu ngồi xuống.";
        // The vectors are made on the device: no model is needed to find it.
        let r = recall(&conn, None, novel, text, "vi").unwrap();
        let all = r.prompt(text);
        assert!(all.contains("- Vương thành → Vương thành (địa danh)"));
        assert!(all.contains("- Lâm Động → Lâm Động (nhân vật); tên gọi khác: Động thiếu"));
        // A part only gets the names it has, an alias finding its entry.
        let second = r.prompt("Động thiếu ngồi xuống.");
        assert!(second.contains("Lâm Động") && !second.contains("Vương thành"));
        assert_eq!(r.prompt("Không ai cả."), "");
        assert_eq!(
            name_phrases("Lâm Động đi vào Vương Thành. Hắn cười."),
            vec![
                "Lâm Động",
                "Lâm",
                "Động",
                "Vương Thành",
                "Vương",
                "Thành",
                "Hắn"
            ]
        );
    }

    #[test]
    fn names_written_two_ways_merge_through_the_vector_table() {
        let (conn, novel) = book("vec");
        let one: String = conn
            .query_row("select vec_version()", [], |r| r.get(0))
            .unwrap();
        assert!(one.starts_with('v'));
        let a = upsert_ai(
            &conn,
            novel,
            "vi",
            Some(1),
            ai("林动", "character", "Lâm Động", &[]),
        )
        .unwrap()
        .unwrap();
        // The same man under a longer form of his name, met later.
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(4),
            ai("林动哥", "character", "Lâm Động ca", &[]),
        )
        .unwrap()
        .unwrap();
        // Another man and a place that start alike are not merged.
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(2),
            ai("林琅天", "character", "Lâm Lang Thiên", &[]),
        )
        .unwrap()
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "vi",
            Some(2),
            ai("林家", "faction", "Lâm Gia", &[]),
        )
        .unwrap()
        .unwrap();
        let llm = Embedder(Mutex::new(0));
        assert_eq!(index_names(&conn, Some(&llm), novel, "vi").unwrap(), 4);
        // Embedded once: nothing more to do.
        assert_eq!(index_names(&conn, Some(&llm), novel, "vi").unwrap(), 0);
        assert_eq!(merge_near(&conn, novel, "vi").unwrap(), 1);
        let w = load(&conn, novel).unwrap();
        assert_eq!(w.entries.len(), 3);
        let lin = w.entries.iter().find(|e| e.uid == a).unwrap();
        assert_eq!(lin.aliases, vec!["林动哥"]);
        // The vector table follows the merge.
        index_names(&conn, Some(&llm), novel, "vi").unwrap();
        let rows: i64 = conn
            .query_row(
                "select count(*) from wiki_names where entry_uid = ?1",
                [&a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 2);
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
                meta: Default::default(),
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
            meta: Default::default(),
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
                meta: Default::default(),
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
                meta: Default::default(),
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
                meta: Default::default(),
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
                    meta: Default::default(),
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
        // Levels of two ladders that match.
        assert_eq!(rel_type("tương đương với"), Some("equivalent_to"));
        assert_eq!(rel_type("Equivalent to"), Some("equivalent_to"));
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
                meta: Default::default(),
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
                meta: Default::default(),
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
                        meta: Default::default(),
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
                meta: Default::default(),
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
            meta: Default::default(),
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
                meta: Default::default(),
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

    #[test]
    fn a_name_keeps_its_first_translation_and_the_chapter_follows_it() {
        let (conn, novel) = book("keepfirstname");
        set_auto(&conn, novel, true).unwrap();
        let chapter: i64 = conn
            .query_row(
                "select id from chapters where novel_id = ?1 limit 1",
                [novel],
                |r| r.get(0),
            )
            .unwrap();
        let first =
            r#"{"entries": [{"name": "地魔", "kind": "character", "translated": "Địa Ma"}]}"#;
        let other =
            r#"{"entries": [{"name": "地魔", "kind": "character", "translated": "Quỷ Đất"}]}"#;
        assert_eq!(learn_part(&conn, novel, chapter, "vi", first).unwrap(), 1);
        // The next part finds it in the wiki before it is sent.
        let prompt = recall(&conn, None, novel, "地魔 gầm lên.", "vi")
            .unwrap()
            .prompt("地魔 gầm lên.");
        assert!(prompt.contains("Địa Ma"), "{prompt}");
        // A part that ran at the same time wrote it its own way: the wiki keeps the first.
        learn_part(&conn, novel, chapter, "vi", other).unwrap();
        let e = &entries(&conn, novel).unwrap()[0];
        assert_eq!(e.names["vi"], "Địa Ma");
        let pairs = settled_names(&conn, novel, &[other.to_string()]).unwrap();
        assert_eq!(pairs, vec![("Quỷ Đất".to_string(), "Địa Ma".to_string())]);
        assert_eq!(
            replace_name(
                "Quỷ đất gầm lên. Quỷ đấtx chưa! (Quỷ đất)",
                "Quỷ đất",
                "Địa ma"
            ),
            "Địa ma gầm lên. Quỷ đấtx chưa! (Địa ma)"
        );
        // With the wiki off nothing goes in.
        set_auto(&conn, novel, false).unwrap();
        let more =
            r#"{"entries": [{"name": "天魔", "kind": "character", "translated": "Thiên ma"}]}"#;
        assert_eq!(learn_part(&conn, novel, chapter, "vi", more).unwrap(), 0);
    }

    #[test]
    fn the_wiki_keeps_only_its_vietnamese_words() {
        let (conn, novel) = book("wikionlyvi");
        upsert_ai(
            &conn,
            novel,
            "en",
            None,
            ai("林动", "character", "Lin Dong", &[]),
        )
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "vi",
            None,
            ai("林动", "character", "Lâm Động", &[]),
        )
        .unwrap();
        upsert_ai(
            &conn,
            novel,
            "en",
            None,
            ai("青檀", "character", "Qing Tan", &[]),
        )
        .unwrap();
        only_vi(&conn, novel).unwrap();
        let all = entries(&conn, novel).unwrap();
        let lin = all.iter().find(|e| e.name == "林动").unwrap();
        assert_eq!(lin.names.keys().collect::<Vec<_>>(), vec!["vi"]);
        // Not in Vietnamese yet: kept until it is translated.
        let qing = all.iter().find(|e| e.name == "青檀").unwrap();
        assert_eq!(qing.names["en"], "Qing Tan");
    }
}
