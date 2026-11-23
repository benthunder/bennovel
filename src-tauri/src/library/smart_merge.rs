//! Smart merge for a book's wiki: finds entries that are one person, place or thing
//! written two ways ("Seol-Hwi" and "Seol Hwi", "Tuyết Hui" and "Tuyết Huy") from their
//! names, mostly the Vietnamese ones the translation used, and lists them for the user
//! to review (`suggest`). Nothing is merged until the user picks pairs (`apply`).
//!
//! Every merge, smart or by hand (`merge_with_rules`), also writes the kept entry's name
//! into the chapters translated before (`rewrite`) where they wrote the merged one, so
//! they show it without being translated again. The user's own replace list
//! (`rules.rs`) is theirs alone: merging never adds to it.

use super::genre_terms::{embed, fold};
use super::llm::AiError;
use super::rules::Rule;
use super::wiki::{self, Entry, WIKI_LANG};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Two names whose letter trigrams are closer than this may be one name with a typo or
/// another spelling ("Tuyết Hui" / "Tuyết Huy").
const NEAR: f32 = 0.3;
/// Suggestions listed at most.
const MAX: usize = 300;

/// Why two entries look like one.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The same Vietnamese name.
    SameName,
    /// The same original name, or one's original name is the other's Vietnamese name
    /// (left untranslated in one place: "Seol-Hwi" and "Tuyết Huy" from "Seol-Hwi").
    SameRaw,
    /// Names that differ by a letter or a mark. Not picked until the user ticks it.
    NearName,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// The entry that goes away: its names become the kept one's aliases.
    pub from: Entry,
    /// The entry that is kept, with its name.
    pub into: Entry,
    pub reason: Reason,
    /// The names the merge will write over in the translations (find → replace).
    pub rules: Vec<Rule>,
    /// More about why, when there is something to say.
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub from_uid: String,
    pub into_uid: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub merged: usize,
    pub rules: usize,
    /// Entries that took the AI's final name.
    pub renamed: usize,
    /// Entries whose wrong original name was put right.
    pub fixed: usize,
}

/// The name an entry is written with in the translation.
pub(crate) fn shown(e: &Entry) -> &str {
    e.names
        .get(WIKI_LANG)
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| e.name.trim())
}

/// A name for comparing: lower case, hyphens and punctuation as spaces, the marks kept
/// ("Seol-Hwi" = "seol hwi", but "Lý Tú" ≠ "Lý Tử").
pub(crate) fn key(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether an entry's Vietnamese name was translated (not the original kept as it is).
fn translated(e: &Entry) -> bool {
    let n = shown(e);
    key(n) != key(&e.name) || n.chars().any(|c| !c.is_ascii())
}

/// Which of two entries to keep: the user's, then a translated name, then the one met
/// first, then the one with more aliases. True keeps `a`.
pub(crate) fn keep_first(a: &Entry, b: &Entry) -> bool {
    let score = |e: &Entry| {
        (
            e.source == "user",
            translated(e),
            std::cmp::Reverse(e.first_chapter.unwrap_or(i64::MAX)),
            e.aliases.len(),
        )
    };
    score(a) >= score(b)
}

/// Whether `find` can be replaced by `to` as plain text without harming other words:
/// not a short one-word name, and not a part of `to` (it would be replaced inside it).
pub(crate) fn safe_find(find: &str, to: &str) -> bool {
    let long = find.contains(' ') || find.contains('-') || find.chars().count() >= 6;
    find.chars().count() >= 3 && long && find != to && !to.contains(find)
}

/// The names merging `from` into `into` writes over in the translations: the names `from` was written
/// with in the translation → the kept name. A name is left out when replacing it as
/// plain text would harm other words or other entries: a short one-word name, one the
/// kept name holds (it would be replaced inside the kept name), one that is another
/// live entry's name, or a title or nickname rather than the name misspelt (`rule_ok`).
pub fn rules_for(from: &Entry, into: &Entry, others: &[Entry]) -> Vec<Rule> {
    let to = shown(into).to_string();
    let mut finds: Vec<String> = vec![shown(from).to_string(), from.name.trim().to_string()];
    finds.dedup();
    let taken = |f: &str| {
        others.iter().any(|o| {
            o.uid != from.uid && o.uid != into.uid && (shown(o) == f || o.name.trim() == f)
        })
    };
    finds
        .into_iter()
        .filter(|f| safe_find(f, &to) && rule_ok(f, &to) && !taken(f))
        .map(|find| Rule {
            find,
            replace: to.clone(),
        })
        .collect()
}

/// Two entries (by their place in the list) that look like one, and why.
type Edge = (usize, usize, Reason);

/// The pairs of the book's live entries that look like one, before they are put
/// together (`to_one`). Pairs set aside (`skip`) are left out.
fn edges(all: &[Entry], skips: &std::collections::HashSet<String>) -> Vec<Edge> {
    struct Seen {
        name: String,
        raw: String,
        words: Vec<String>,
        vec: Vec<f32>,
    }
    let seen: Vec<Seen> = all
        .iter()
        .map(|e| {
            let n = shown(e);
            Seen {
                name: key(n),
                raw: key(&e.name),
                words: fold(&key(n)).split_whitespace().map(String::from).collect(),
                vec: embed(n),
            }
        })
        .collect();
    let mut out: Vec<Edge> = Vec::new();
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            let (a, b) = (&all[i], &all[j]);
            if a.kind != b.kind {
                continue;
            }
            // Two of the user's own entries are offered together only when they are
            // written the same (the user named both "Lâm Động": one entry, many originals).
            let both_user = a.source == "user" && b.source == "user";
            if a.kind == "level"
                && !a.meta.system.is_empty()
                && !b.meta.system.is_empty()
                && a.meta.system != b.meta.system
            {
                continue;
            }
            let (sa, sb) = (&seen[i], &seen[j]);
            let reason = if !sa.name.is_empty() && sa.name == sb.name {
                Reason::SameName
            } else if !sa.raw.is_empty()
                && (sa.raw == sb.raw || sa.raw == sb.name || sa.name == sb.raw)
            {
                Reason::SameRaw
            } else if sa.words.len() >= 2
                && sa.words.len() == sb.words.len()
                && sa.words.iter().any(|w| sb.words.contains(w))
                && distance(&sa.vec, &sb.vec) < NEAR
                && raws_close(a, b)
            {
                Reason::NearName
            } else {
                continue;
            };
            if both_user && reason == Reason::NearName {
                continue;
            }
            if skips.contains(&skip_key([a.uid.as_str(), b.uid.as_str()])) {
                continue;
            }
            out.push((i, j, reason));
        }
    }
    out
}

/// Puts linked entries together, many to one: each set of entries that look like one
/// (through any of `edges`) gets one entry to keep (`keep_first` over them all), and every
/// other entry of the set goes into it. An entry never goes into two entries, and a
/// chain ("Seol Hwi" ~ "Seol-Hwi" ~ "Tuyết Huy") ends in one kept entry instead of
/// pairs pointing at each other. The reason of a member is that of its own link to the
/// kept entry, else the weakest link it has (a near name is not ticked until the user
/// allows it). Two of the user's entries are never merged, so a set holding several
/// keeps the first and leaves the others out; a pair set aside is left out too.
fn to_one(
    all: &[Entry],
    edges: &[Edge],
    skips: &std::collections::HashSet<String>,
) -> Vec<Suggestion> {
    let mut parent: Vec<usize> = (0..all.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for &(a, b, _) in edges {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut sets: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for &(a, b, _) in edges {
        for i in [a, b] {
            let r = root(&mut parent, i);
            let set = sets.entry(r).or_default();
            if !set.contains(&i) {
                set.push(i);
            }
        }
    }
    let mut out = Vec::new();
    for set in sets.values() {
        let keep = set
            .iter()
            .copied()
            .reduce(|k, i| if keep_first(&all[k], &all[i]) { k } else { i })
            .expect("a set has entries");
        let into = &all[keep];
        let mut members: Vec<usize> = set.iter().copied().filter(|&i| i != keep).collect();
        members.sort_by_key(|&i| (all[i].first_chapter.unwrap_or(i64::MAX), i));
        for m in members {
            let from = &all[m];
            if skips.contains(&skip_key([from.uid.as_str(), into.uid.as_str()])) {
                continue;
            }
            let link = |x: usize, y: usize| {
                edges
                    .iter()
                    .find(|(a, b, _)| (*a == x && *b == y) || (*a == y && *b == x))
                    .map(|e| e.2)
            };
            let reason = link(m, keep).unwrap_or_else(|| {
                edges
                    .iter()
                    .filter(|(a, b, _)| *a == m || *b == m)
                    .map(|e| e.2)
                    .max()
                    .unwrap_or(Reason::NearName)
                    .max(Reason::SameRaw)
            });
            // Two of the user's own entries only when written the same.
            if from.source == "user" && into.source == "user" && reason == Reason::NearName {
                continue;
            }
            out.push(Suggestion {
                rules: rules_for(from, into, all),
                from: from.clone(),
                into: into.clone(),
                reason,
                note: String::new(),
            });
        }
    }
    out
}

/// The book's live entries that look like one, for the user to review: many entries
/// into one kept entry each (`to_one`), the surest first.
pub fn suggest(conn: &Connection, novel_id: i64) -> rusqlite::Result<Vec<Suggestion>> {
    let all = wiki::entries(conn, novel_id)?;
    let skips = skipped(conn, novel_id)?;
    let mut out = to_one(&all, &edges(&all, &skips), &skips);
    // The kept entry's members stay together, the surest sets first.
    let best: HashMap<String, Reason> = out.iter().fold(HashMap::new(), |mut m, s| {
        let r = m.entry(s.into.uid.clone()).or_insert(s.reason);
        *r = (*r).min(s.reason);
        m
    });
    out.sort_by(|x, y| {
        (best[&x.into.uid], &x.into.uid, x.reason).cmp(&(best[&y.into.uid], &y.into.uid, y.reason))
    });
    out.truncate(MAX);
    Ok(out)
}

/// The key of a set of entries a suggestion would merge (or rename): their uids,
/// sorted, joined by '+'.
pub fn skip_key<'a>(uids: impl IntoIterator<Item = &'a str>) -> String {
    let mut v: Vec<&str> = uids.into_iter().collect();
    v.sort_unstable();
    v.dedup();
    v.join("+")
}

/// Sets a suggestion aside as already right: the same entries are never suggested
/// together again (nor, for one entry alone, with another name).
pub fn skip(conn: &Connection, novel_id: i64, uids: &[String]) -> rusqlite::Result<()> {
    conn.execute(
        "insert or ignore into wiki_merge_skips (novel_id, uids, created_at) values (?1, ?2, ?3)",
        rusqlite::params![
            novel_id,
            skip_key(uids.iter().map(String::as_str)),
            super::db::now_ms()
        ],
    )?;
    Ok(())
}

/// The keys (`skip_key`) of the book's suggestions set aside.
pub fn skipped(
    conn: &Connection,
    novel_id: i64,
) -> rusqlite::Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare("select uids from wiki_merge_skips where novel_id = ?1")?;
    let rows = stmt.query_map([novel_id], |r| r.get(0))?;
    rows.collect()
}

/// An original name the translation wrote as an entry's name (or close to it).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RawName {
    /// As the original writes it.
    pub raw: String,
    /// As the translation wrote it.
    pub translated: String,
    /// Chapters where the translation wrote it so, by number, the first ones.
    pub chapters: Vec<i64>,
    /// The entry whose name or alias it is, when one has it.
    pub entry_uid: Option<String>,
    /// The translation wrote exactly the entry's name (else a near spelling of it).
    pub exact: bool,
}

/// Smart merge for one entry: the original names translated as its name, and the
/// entries that look like it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryMerge {
    pub raws: Vec<RawName>,
    pub pairs: Vec<Suggestion>,
}

/// Chapters listed per original name at most.
const CHAPTERS_SHOWN: usize = 5;

/// Whether the translated name `b` is close to `a` (as `suggest` judges a near name).
pub(crate) fn near(a: &str, b: &str) -> bool {
    let (ka, kb) = (key(a), key(b));
    if ka == kb {
        return true;
    }
    let wa: Vec<String> = fold(&ka).split_whitespace().map(String::from).collect();
    let wb: Vec<String> = fold(&kb).split_whitespace().map(String::from).collect();
    wa.len() >= 2
        && wa.len() == wb.len()
        && wa.iter().any(|w| wb.contains(w))
        && distance(&embed(a), &embed(b)) < NEAR
}

/// Whether `find` may become `to` as a replace rule: the same name written another way
/// ("Cổ Đại Viêm" → "Cố Đại Diễm", "Dạ Tô Quang" → "Dữ Tú Quang"), or an original name
/// the translation left as it was ("Gu Dae-yeom"), or any name while the kept one is not
/// translated yet (a later merge carries the rule on to its translation). A title or a nickname ("Hắc Ma Chiến
/// Quân" → "Cố Đại Diễm") is another word for the person, not a misspelling: replacing
/// it in the text would put the person's name where the story says the title.
pub(crate) fn rule_ok(find: &str, to: &str) -> bool {
    super::detect_lang(find) != "vi" || super::detect_lang(to) != "vi" || spelled_alike(find, to)
}

/// Whether two names are one written another way: the same number of words with half
/// of them the same, or few letters apart (marks, hyphens and case aside).
pub(crate) fn spelled_alike(a: &str, b: &str) -> bool {
    let (fa, fb) = (fold(&key(a)), fold(&key(b)));
    if fa == fb {
        return true;
    }
    let wa: Vec<&str> = fa.split_whitespace().collect();
    let wb: Vec<&str> = fb.split_whitespace().collect();
    if wa.len() == wb.len() && wa.iter().zip(&wb).filter(|(x, y)| x == y).count() * 2 >= wa.len() {
        return true;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (
        fa.chars().filter(|c| !c.is_whitespace()).collect(),
        fb.chars().filter(|c| !c.is_whitespace()).collect(),
    );
    let longest = ca.len().max(cb.len());
    longest > 0 && edits(&ca, &cb) * 3 <= longest
}

/// The letters to change, add or remove to turn `a` into `b`.
fn edits(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// Whether two original names are one written another way, more strictly than
/// `spelled_alike`: more than half of the words the same, or few letters apart
/// ("Heuk Bi" and "Ack Bi" are two people, though both end in "Bi").
pub(crate) fn raw_close(a: &str, b: &str) -> bool {
    let (fa, fb) = (fold(&key(a)), fold(&key(b)));
    if fa == fb {
        return true;
    }
    let wa: Vec<&str> = fa.split_whitespace().collect();
    let wb: Vec<&str> = fb.split_whitespace().collect();
    if wa.len() == wb.len() && wa.iter().zip(&wb).filter(|(x, y)| x == y).count() * 2 > wa.len() {
        return true;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (
        fa.chars().filter(|c| !c.is_whitespace()).collect(),
        fb.chars().filter(|c| !c.is_whitespace()).collect(),
    );
    let longest = ca.len().max(cb.len());
    longest > 0 && edits(&ca, &cb) * 4 <= longest
}

/// The original names of an entry: its name and aliases.
fn raws_of(e: &Entry) -> impl Iterator<Item = &str> {
    std::iter::once(e.name.trim()).chain(e.aliases.iter().map(|a| a.trim()))
}

/// Whether two entries with near translated names are also near in the original:
/// a near translation of two different originals is two people (or places).
/// Names in other scripts (Han and a romanised one, or two Han names) can't be told
/// apart by their letters, so they don't count against the pair.
fn raws_close(a: &Entry, b: &Entry) -> bool {
    raws_of(a).any(|x| raws_of(b).any(|y| raw_close(x, y) || !latin_pair(x, y)))
}

/// `raws_close` for auto merge (wiki.rs `merge_near`).
pub(crate) fn raws_close_entries(a: &Entry, b: &Entry) -> bool {
    raws_close(a, b)
}

/// Whether two names are both in Latin letters (so `raw_close` can judge them).
fn latin_pair(a: &str, b: &str) -> bool {
    let latin = |s: &str| matches!(super::detect_lang(s), "en" | "vi");
    latin(a) && latin(b)
}

/// What smart merge finds for entry `uid`: the original names the translations (and
/// the wiki) wrote as its name or close to it, and the pairs with this entry, from
/// `suggest` and from those names when another entry of the same kind has them.
pub fn for_entry(conn: &Connection, novel_id: i64, uid: &str) -> rusqlite::Result<EntryMerge> {
    let all = wiki::entries(conn, novel_id)?;
    let Some(me) = all.iter().find(|e| e.uid == uid) else {
        return Ok(EntryMerge {
            raws: vec![],
            pairs: vec![],
        });
    };
    let name = shown(me).to_string();
    let owner = |raw: &str| {
        all.iter()
            .find(|e| e.name.trim() == raw || e.aliases.iter().any(|a| a.trim() == raw))
            .map(|e| e.uid.clone())
    };
    let mut raws: Vec<RawName> = Vec::new();
    let mut add = |raw: &str, tr: &str, chapter: Option<i64>| {
        if !near(&name, tr) {
            return;
        }
        let exact = key(&name) == key(tr);
        // Only near in the translation: the original must be near one of this entry's too.
        if !exact && !raws_of(me).any(|r| raw_close(r, raw) || !latin_pair(r, raw)) {
            return;
        }
        match raws.iter_mut().find(|r| r.raw == raw && r.translated == tr) {
            Some(r) => {
                if let Some(c) = chapter {
                    if !r.chapters.contains(&c) && r.chapters.len() < CHAPTERS_SHOWN {
                        r.chapters.push(c);
                    }
                }
            }
            None => raws.push(RawName {
                raw: raw.to_string(),
                translated: tr.to_string(),
                chapters: chapter.into_iter().collect(),
                entry_uid: owner(raw),
                exact,
            }),
        }
    };
    for (raw, tr, chapter) in wiki::found_names(conn, novel_id, WIKI_LANG)? {
        add(&raw, &tr, Some(chapter));
    }
    for e in &all {
        let tr = shown(e).to_string();
        for raw in std::iter::once(&e.name).chain(e.aliases.iter()) {
            add(raw.trim(), &tr, None);
        }
    }
    raws.sort_by_key(|r| (!r.exact, r.entry_uid.as_deref() != Some(uid)));
    let skips = skipped(conn, novel_id)?;
    let mut links = edges(&all, &skips);
    let at = |u: &str| all.iter().position(|e| e.uid == u);
    let mine = at(uid).expect("the entry is in the list");
    // The entries that own an original name translated as this one's name look like it too.
    for r in &raws {
        let Some(o) = r.entry_uid.as_deref().and_then(at) else {
            continue;
        };
        let other = &all[o];
        if o == mine
            || other.kind != me.kind
            || (other.source == "user" && me.source == "user" && !r.exact)
            || skips.contains(&skip_key([uid, other.uid.as_str()]))
            || links
                .iter()
                .any(|(a, b, _)| (*a == mine && *b == o) || (*a == o && *b == mine))
        {
            continue;
        }
        let reason = if r.exact {
            Reason::SameName
        } else {
            Reason::NearName
        };
        links.push((mine.min(o), mine.max(o), reason));
    }
    // The set this entry is in, many into one: it may go into another entry with the rest.
    let pairs: Vec<Suggestion> = to_one(&all, &links, &skips);
    let kept = pairs
        .iter()
        .find(|s| s.from.uid == uid || s.into.uid == uid)
        .map(|s| s.into.uid.clone());
    let pairs = pairs
        .into_iter()
        .filter(|s| Some(&s.into.uid) == kept.as_ref())
        .collect();
    Ok(EntryMerge { raws, pairs })
}

pub(crate) fn distance(a: &[f32], b: &[f32]) -> f32 {
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

/// Writes the kept name into the book's Vietnamese translations where the merged names
/// were written (`new`: find → replace, longest first): every translated paragraph and
/// chapter title, as whole words. The user's replace list is not touched. Returns the
/// paragraphs and titles changed.
pub fn rewrite(conn: &Connection, novel_id: i64, new: &[Rule]) -> rusqlite::Result<usize> {
    let mut new: Vec<&Rule> = new
        .iter()
        .filter(|r| !r.find.trim().is_empty() && r.find != r.replace)
        .collect();
    if new.is_empty() {
        return Ok(0);
    }
    new.sort_by_key(|r| std::cmp::Reverse(r.find.chars().count()));
    let fix = |text: &str| -> Option<String> {
        let mut out = text.to_string();
        let mut changed = false;
        for r in &new {
            if let Some(t) = replace_words(&out, &r.find, &r.replace) {
                out = t;
                changed = true;
            }
        }
        changed.then_some(out)
    };
    let mut stmt = conn.prepare(
        "select s.id, s.chapter_id, s.style, s.content from segments s
           join chapters c on c.id = s.chapter_id
         where c.novel_id = ?1 and s.lang = ?2 and s.style <> '' and s.type = 'text'",
    )?;
    let rows: Vec<(i64, i64, String, String)> = stmt
        .query_map(rusqlite::params![novel_id, WIKI_LANG], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = 0;
    let mut touched: Vec<(i64, String)> = Vec::new();
    for (id, chapter, style, content) in rows {
        let Ok(sentences) = serde_json::from_str::<Vec<String>>(&content) else {
            continue;
        };
        let mut any = false;
        let fixed: Vec<String> = sentences
            .iter()
            .map(|t| match fix(t) {
                Some(f) => {
                    any = true;
                    f
                }
                None => t.clone(),
            })
            .collect();
        if any {
            conn.execute(
                "update segments set content = ?2 where id = ?1",
                rusqlite::params![id, serde_json::to_string(&fixed).unwrap_or(content)],
            )?;
            changed += 1;
            if !touched.contains(&(chapter, style.clone())) {
                touched.push((chapter, style));
            }
        }
    }
    let mut stmt = conn.prepare(
        "select t.id, t.title from chapter_translations t join chapters c on c.id = t.chapter_id
         where c.novel_id = ?1 and t.lang = ?2",
    )?;
    let titles: Vec<(i64, String)> = stmt
        .query_map(rusqlite::params![novel_id, WIKI_LANG], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, title) in titles {
        if let Some(t) = fix(&title) {
            conn.execute(
                "update chapter_translations set title = ?2,
                   updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
                rusqlite::params![id, t],
            )?;
            changed += 1;
        }
    }
    // A changed translation is the newer one when the library syncs.
    for (chapter, style) in touched {
        conn.execute(
            "update chapter_translations set updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where chapter_id = ?1 and lang = ?2 and coalesce(style, '') = ?3",
            rusqlite::params![chapter, WIKI_LANG, style],
        )?;
    }
    Ok(changed)
}

/// `text` with every `find` that stands as whole words (not inside a longer word)
/// written `to`; None when there is none.
pub(crate) fn replace_words(text: &str, find: &str, to: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut any = false;
    for (i, _) in text.match_indices(find) {
        if i < last {
            continue;
        }
        let before = text[..i].chars().next_back();
        let after = text[i + find.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        if word(before) || word(after) {
            continue;
        }
        out.push_str(&text[last..i]);
        out.push_str(to);
        last = i + find.len();
        any = true;
    }
    if !any {
        return None;
    }
    out.push_str(&text[last..]);
    Some(out)
}

/// Saves an entry the user edited; when its Vietnamese name changed, the old name is
/// written over with the new one in the translated chapters too (`rewrite`), and the
/// names table the translation reads follows. Returns the entry and the paragraphs changed.
pub fn save_entry(
    conn: &Connection,
    novel_id: i64,
    input: wiki::EntryInput,
) -> Result<(Entry, usize), AiError> {
    let before = match input.uid.as_deref() {
        Some(uid) => wiki::entries(conn, novel_id)?
            .into_iter()
            .find(|e| e.uid == uid),
        None => None,
    };
    let saved = wiki::save(conn, novel_id, input)?;
    let mut changed = 0;
    if let Some(old) = before {
        let (from, to) = (
            shown(&old).trim().to_string(),
            shown(&saved).trim().to_string(),
        );
        if !from.is_empty()
            && !to.is_empty()
            && from != to
            && from.chars().count() >= 2
            && !to.contains(&from)
        {
            changed = rewrite(
                conn,
                novel_id,
                &[Rule {
                    find: from,
                    replace: to,
                }],
            )?;
        }
    }
    wiki::index_names(conn, None, novel_id, WIKI_LANG)?;
    Ok((saved, changed))
}

/// Merges `from_uid` into `into_uid` as the user asked and writes the kept name over
/// the name that went away in the translations. Returns the kept entry and the
/// paragraphs changed.
pub fn merge_with_rules(
    conn: &Connection,
    novel_id: i64,
    from_uid: &str,
    into_uid: &str,
) -> Result<(Entry, usize), AiError> {
    let all = wiki::entries(conn, novel_id)?;
    let (Some(from), Some(into)) = (
        all.iter().find(|e| e.uid == from_uid),
        all.iter().find(|e| e.uid == into_uid),
    ) else {
        return Err(AiError::Other("wiki-entry-missing".into()));
    };
    let new = rules_for(from, into, &all);
    let kept = wiki::merge(conn, novel_id, from_uid, into_uid, true)?;
    let added = rewrite(conn, novel_id, &new)?;
    Ok((kept, added))
}

/// What "allow merge" on one entry did: the entry kept and what changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Allowed {
    pub entry: Entry,
    pub applied: Applied,
    /// Original names taken in as the kept entry's aliases.
    pub adopted: usize,
}

/// The user allowed the merge found for an entry (`for_entry`), many into one: every
/// entry of `from_uids` goes into `into_uid`, and the original names `raws` the chapters
/// translated as its name (with no entry of their own) become its aliases, their other
/// spellings written over with its name in the translations. Nothing else is merged.
pub fn allow(
    conn: &Connection,
    novel_id: i64,
    into_uid: &str,
    from_uids: &[String],
    raws: &[RawName],
) -> Result<Allowed, AiError> {
    let picks: Vec<Pick> = from_uids
        .iter()
        .filter(|u| u.as_str() != into_uid)
        .map(|u| Pick {
            from_uid: u.clone(),
            into_uid: into_uid.to_string(),
        })
        .collect();
    let mut applied = apply(conn, novel_id, &picks)?;
    let all = wiki::entries(conn, novel_id)?;
    let Some(into) = all.iter().find(|e| e.uid == into_uid).cloned() else {
        return Err(AiError::Other("wiki-entry-missing".into()));
    };
    let to = shown(&into).to_string();
    let taken = |name: &str| {
        all.iter().any(|o| {
            o.uid != into.uid
                && (o.name.trim() == name || o.aliases.iter().any(|a| a.trim() == name))
        })
    };
    let mut aliases = into.aliases.clone();
    let mut rules: Vec<Rule> = Vec::new();
    let mut adopted = 0;
    for r in raws {
        let raw = r.raw.trim();
        if raw.is_empty() || taken(raw) {
            continue;
        }
        if raw != into.name.trim() && !aliases.iter().any(|a| a.trim() == raw) {
            aliases.push(raw.to_string());
            adopted += 1;
        }
        let tr = r.translated.trim();
        let other_name = all.iter().any(|o| o.uid != into.uid && shown(o) == tr);
        if !other_name
            && safe_find(tr, &to)
            && rule_ok(tr, &to)
            && !rules.iter().any(|x| x.find == tr)
        {
            rules.push(Rule {
                find: tr.to_string(),
                replace: to.clone(),
            });
        }
    }
    let entry = if aliases != into.aliases {
        wiki::save(
            conn,
            novel_id,
            wiki::EntryInput {
                uid: Some(into.uid.clone()),
                kind: into.kind.clone(),
                name: into.name.clone(),
                names: into.names.clone(),
                aliases,
                summaries: into.summaries.clone(),
                rank: into.rank,
                meta: into.meta.clone(),
            },
        )?
    } else {
        into
    };
    applied.rules += rewrite(conn, novel_id, &rules)?;
    wiki::index_names(conn, None, novel_id, WIKI_LANG)?;
    Ok(Allowed {
        entry,
        applied,
        adopted,
    })
}

/// Merges the pairs the user picked from `suggest`, in order. A pair whose entry was
/// already merged in this run follows it to where it went.
pub fn apply(conn: &Connection, novel_id: i64, picks: &[Pick]) -> Result<Applied, AiError> {
    apply_with(conn, novel_id, picks, &mut HashMap::new())
}

/// `apply` that records in `moved` where each merged entry went.
pub fn apply_with(
    conn: &Connection,
    novel_id: i64,
    picks: &[Pick],
    moved: &mut HashMap<String, String>,
) -> Result<Applied, AiError> {
    let follow = |moved: &HashMap<String, String>, uid: &str| {
        let mut u = uid.to_string();
        let mut hops = 0;
        while let Some(n) = moved.get(&u) {
            u = n.clone();
            hops += 1;
            if hops > 64 {
                break;
            }
        }
        u
    };
    let mut done = Applied::default();
    for p in picks {
        let (from, into) = (follow(moved, &p.from_uid), follow(moved, &p.into_uid));
        if from == into || !wiki::by_uid_live(conn, &from) || !wiki::by_uid_live(conn, &into) {
            continue;
        }
        let (_, added) = merge_with_rules(conn, novel_id, &from, &into)?;
        moved.insert(from, into);
        done.merged += 1;
        done.rules += added;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::book;
    use crate::library::wiki::EntryInput;
    use std::collections::BTreeMap;

    fn add(conn: &Connection, novel: i64, kind: &str, raw: &str, vi: &str) -> String {
        let mut names = BTreeMap::new();
        names.insert("vi".to_string(), vi.to_string());
        let e = wiki::save(
            conn,
            novel,
            EntryInput {
                uid: None,
                kind: kind.into(),
                name: raw.into(),
                names,
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
                meta: Default::default(),
            },
        )
        .unwrap();
        // As if the AI had written it, so two of them can be merged.
        conn.execute(
            "update wiki_entries set source = 'ai' where uid = ?1",
            [&e.uid],
        )
        .unwrap();
        e.uid
    }

    #[test]
    fn finds_names_written_two_ways_and_keeps_the_translated_one() {
        let (conn, novel) = book("smart-merge");
        let a = add(&conn, novel, "character", "Seol-Hwi", "Tuyết Huy");
        // Saving would find "Seol Hwi" as `a` already; the AI may still have written it
        // as its own entry before.
        let b = add(&conn, novel, "character", "tmp", "tmp");
        conn.execute(
            "update wiki_entries set name = 'Seol Hwi', names = '{\"vi\":\"Seol-Hwi\"}' where uid = ?1",
            [&b],
        )
        .unwrap();
        let c = add(&conn, novel, "character", "Seolhwi", "Tuyết Hui");
        // Another person, and a place of the same name: not one.
        add(&conn, novel, "character", "Gu Dae-yeom", "Cừu Đại Diễm");
        add(&conn, novel, "place", "Seolhwi Peak", "Tuyết Huy");
        let s = suggest(&conn, novel).unwrap();
        let pairs: Vec<(&str, &str, Reason)> = s
            .iter()
            .map(|x| (x.from.uid.as_str(), x.into.uid.as_str(), x.reason))
            .collect();
        assert!(
            pairs.contains(&(b.as_str(), a.as_str(), Reason::SameRaw)),
            "{pairs:?}"
        );
        assert!(
            pairs.contains(&(c.as_str(), a.as_str(), Reason::NearName)),
            "{pairs:?}"
        );
        assert_eq!(pairs.len(), 2, "{pairs:?}");
        let rules = &s.iter().find(|x| x.from.uid == b).unwrap().rules;
        assert_eq!(
            rules,
            &vec![
                Rule {
                    find: "Seol-Hwi".into(),
                    replace: "Tuyết Huy".into()
                },
                Rule {
                    find: "Seol Hwi".into(),
                    replace: "Tuyết Huy".into()
                },
            ]
        );
    }

    #[test]
    fn a_chain_of_names_goes_into_one_kept_entry() {
        let (conn, novel) = book("smart-merge-chain");
        let a = add(&conn, novel, "character", "X1", "Tuyết Huy");
        // The same Vietnamese name as `a`.
        let b = add(&conn, novel, "character", "Seol Hwi", "Tuyết Huy");
        // The same original as `b`, nothing like `a`'s.
        let c = add(&conn, novel, "character", "Seol-Hwi", "Seol Hwi");
        let s = suggest(&conn, novel).unwrap();
        assert_eq!(s.len(), 2, "{s:?}");
        // One kept entry for the three: a translated name (`a` or `b`, never `c`).
        let kept = s[0].into.uid.clone();
        assert!(s.iter().all(|x| x.into.uid == kept), "many into one");
        assert!(kept == a || kept == b);
        // Each entry goes into one entry only.
        let froms: std::collections::HashSet<&str> =
            s.iter().map(|x| x.from.uid.as_str()).collect();
        assert_eq!(froms.len(), 2);
        assert!(froms.contains(c.as_str()));
        // The entry's own check shows the whole set going into the same entry.
        let m = for_entry(&conn, novel, &c).unwrap();
        assert!(m.pairs.iter().all(|p| p.into.uid == kept), "{:?}", m.pairs);
        assert_eq!(m.pairs.len(), 2);
        let from: Vec<String> = [&a, &b, &c]
            .into_iter()
            .filter(|u| **u != kept)
            .cloned()
            .collect();
        // Allowed: both go into the kept one.
        let done = allow(&conn, novel, &kept, &from, &[]).unwrap();
        assert_eq!(done.applied.merged, 2);
        assert_eq!(wiki::entries(&conn, novel).unwrap().len(), 1);
    }

    #[test]
    fn a_name_the_user_gives_another_entry_joins_the_set_already_there() {
        // a-a and a-b are one entry "Lâm Động" (merged by the user, so theirs).
        let (conn, novel) = book("smart-merge-user-same");
        let a = add(&conn, novel, "character", "a-a", "Lâm Động");
        let ab = add(&conn, novel, "character", "a-b", "Lâm Động");
        allow(&conn, novel, &a, &[ab], &[]).unwrap();
        // The user then names another entry "Lâm Động" too.
        let b = add(&conn, novel, "character", "b-b", "Lâm Đống");
        let e = wiki::entries(&conn, novel)
            .unwrap()
            .into_iter()
            .find(|e| e.uid == b)
            .unwrap();
        let mut names = e.names.clone();
        names.insert("vi".into(), "Lâm Động".into());
        save_entry(
            &conn,
            novel,
            wiki::EntryInput {
                uid: Some(b.clone()),
                kind: e.kind,
                name: e.name,
                names,
                aliases: e.aliases,
                summaries: e.summaries,
                rank: e.rank,
                meta: e.meta,
            },
        )
        .unwrap();
        // Both are the user's now; still offered as one: b-b into the entry already there.
        let m = for_entry(&conn, novel, &b).unwrap();
        assert_eq!(m.pairs.len(), 1, "{:?}", m.pairs);
        assert_eq!(
            (m.pairs[0].from.uid.as_str(), m.pairs[0].into.uid.as_str()),
            (b.as_str(), a.as_str())
        );
        allow(&conn, novel, &a, &[b], &[]).unwrap();
        let all = wiki::entries(&conn, novel).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].aliases, vec!["a-b".to_string(), "b-b".to_string()]);
        // Two of the user's with only near names are still not offered.
        let c = add(&conn, novel, "character", "c-c", "Lâm Đông");
        conn.execute(
            "update wiki_entries set source = 'user' where uid = ?1",
            [&c],
        )
        .unwrap();
        assert!(for_entry(&conn, novel, &c).unwrap().pairs.is_empty());
    }

    #[test]
    fn allowing_takes_loose_original_names_in_as_aliases() {
        let (conn, novel) = book("smart-merge-allow-raws");
        let a = add(&conn, novel, "character", "Seol-Hwi", "Tuyết Huy");
        let done = allow(
            &conn,
            novel,
            &a,
            &[],
            &[RawName {
                raw: "Seolhwi".into(),
                translated: "Tuyết Hui".into(),
                ..Default::default()
            }],
        )
        .unwrap();
        assert_eq!(done.adopted, 1);
        assert_eq!(done.entry.aliases, vec!["Seolhwi".to_string()]);
    }

    #[test]
    fn merging_writes_the_kept_name_into_the_translations_and_not_the_replace_list() {
        let (conn, novel) = book("smart-merge-rules");
        conn.execute_batch(
            r#"insert into chapter_translations (chapter_id, lang, style, source, state, title, content, updated_at)
                 values (10, 'vi', 'natural', 'ai', 'published', 'Chương 1: Tuyết Hui', '', '2020-01-01T00:00:00.000Z');
               insert into segments (chapter_id, lang, style, position, type, content) values
                 (10, 'vi', 'natural', 0, 'text', '["Tuyết Hui cười.","Seol Hwi đi, Tuyết Huiên ở lại."]'),
                 (10, 'en', '', 0, 'text', '["Seol Hwi smiled."]');"#,
        )
        .unwrap();
        let a = add(&conn, novel, "character", "Seol-Hwi", "Tuyết Huy");
        let b = add(&conn, novel, "character", "Seolhwi", "Tuyết Hui");
        let c = add(&conn, novel, "character", "Seol Hwi", "Seol Hwi");
        let done = apply(
            &conn,
            novel,
            &[
                Pick {
                    from_uid: b.clone(),
                    into_uid: c.clone(),
                },
                Pick {
                    from_uid: c.clone(),
                    into_uid: a.clone(),
                },
                // Already gone into `a` through `c`.
                Pick {
                    from_uid: b.clone(),
                    into_uid: a.clone(),
                },
            ],
        )
        .unwrap();
        assert_eq!(done.merged, 2);
        // "Tuyết Hui" first became "Seol Hwi", then followed it to "Tuyết Huy"; a longer
        // word holding the name and the original text stay as they were.
        let text = |lang: &str| -> String {
            conn.query_row(
                "select content from segments where chapter_id = 10 and lang = ?1",
                [lang],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            text("vi"),
            r#"["Tuyết Huy cười.","Tuyết Huy đi, Tuyết Huiên ở lại."]"#
        );
        assert_eq!(text("en"), r#"["Seol Hwi smiled."]"#);
        let (title, at): (String, String) = conn
            .query_row("select title, updated_at from chapter_translations where chapter_id = 10 and lang = 'vi'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(title, "Chương 1: Tuyết Huy");
        assert!(at.as_str() > "2020-01-01T00:00:00.000Z");
        assert!(done.rules > 0);
        assert_eq!(super::super::rules::get(&conn, novel, "vi").unwrap(), None);
        let w = wiki::load(&conn, novel).unwrap();
        assert_eq!(w.entries.len(), 1);
    }

    #[test]
    fn a_name_inside_the_kept_one_or_too_short_gets_no_rule() {
        let (conn, novel) = book("smart-merge-short");
        let a = add(&conn, novel, "character", "Lin Dong", "Lâm Động");
        let b = add(&conn, novel, "character", "Dong", "Động");
        let c = add(&conn, novel, "character", "Xiao", "Tiểu");
        let all = wiki::entries(&conn, novel).unwrap();
        let get = |u: &str| all.iter().find(|e| e.uid == u).unwrap();
        assert!(rules_for(get(&b), get(&a), &all).is_empty());
        assert!(rules_for(get(&c), get(&a), &all).is_empty());
    }

    #[test]
    fn an_entry_lists_the_original_names_translated_as_its_name() {
        let (conn, novel) = book("smart-merge-entry");
        let a = add(&conn, novel, "character", "Seol-Hwi", "Tuyết Huy");
        let b = add(&conn, novel, "character", "雪辉", "Tuyết Hui");
        add(&conn, novel, "character", "Gu Dae-yeom", "Cừu Đại Diễm");
        // Chapter 2 wrote "Seol-Hwi" for a name nobody has yet, and another name as Tuyết Huy.
        conn.execute(
            "insert into chapter_wiki_facts (chapter_id, lang, facts, created_at) values (11, 'vi', ?1, 0)",
            [serde_json::to_string(&vec![
                r#"{"entries":[{"name":"Seol Hwi","translated":"Tuyết Huy"},{"name":"Gu Dae-yeom","translated":"Cừu Đại Diễm"}]}"#,
            ])
            .unwrap()],
        )
        .unwrap();
        let m = for_entry(&conn, novel, &a).unwrap();
        let raws: Vec<(&str, &str, bool)> = m
            .raws
            .iter()
            .map(|r| (r.raw.as_str(), r.translated.as_str(), r.exact))
            .collect();
        assert_eq!(
            raws,
            vec![
                ("Seol-Hwi", "Tuyết Huy", true),
                ("Seol Hwi", "Tuyết Huy", true),
                ("雪辉", "Tuyết Hui", false)
            ],
        );
        assert_eq!(m.raws[1].chapters, vec![2]);
        assert_eq!(m.raws[1].entry_uid, None);
        assert_eq!(m.raws[2].entry_uid.as_deref(), Some(b.as_str()));
        assert_eq!(m.pairs.len(), 1);
        assert_eq!(
            (m.pairs[0].from.uid.as_str(), m.pairs[0].into.uid.as_str()),
            (b.as_str(), a.as_str())
        );
    }

    #[test]
    fn a_title_or_nickname_gets_no_rule_but_a_misspelling_does() {
        assert!(rule_ok("Cổ Đại Viêm", "Cố Đại Diễm"));
        assert!(rule_ok("Dạ Tô Quang", "Dữ Tú Quang"));
        assert!(rule_ok("Gu Dae-yeom", "Cố Đại Diễm"));
        assert!(!rule_ok("Hắc Ma Chiến Quân", "Cố Đại Diễm"));
        assert!(!rule_ok("Hắc Ma Chiến Tướng", "Cố Đại Diễm"));
        assert!(!rule_ok("Thiếu chủ", "Tuyết Huy"));
    }

    #[test]
    fn a_pair_set_aside_is_not_suggested_again() {
        let (conn, novel) = book("smart-merge-skip");
        let a = add(&conn, novel, "character", "Seol-Hwi", "Tuyết Huy");
        let b = add(&conn, novel, "character", "Seol Hwi", "Tuyết Huy");
        assert_eq!(suggest(&conn, novel).unwrap().len(), 1);
        skip(&conn, novel, &[b.clone(), a.clone()]).unwrap();
        skip(&conn, novel, &[a.clone(), b.clone()]).unwrap();
        assert!(suggest(&conn, novel).unwrap().is_empty());
        assert!(for_entry(&conn, novel, &a).unwrap().pairs.is_empty());
        assert_eq!(skipped(&conn, novel).unwrap().len(), 1);
    }

    #[test]
    fn near_translations_of_two_different_originals_are_not_a_pair() {
        let (conn, novel) = book("smart-merge-near-raw");
        let a = add(&conn, novel, "character", "Ack Bi", "Ác Bi");
        add(&conn, novel, "character", "Heuk Bi", "Hắc Bỉ");
        assert!(suggest(&conn, novel).unwrap().is_empty());
        let m = for_entry(&conn, novel, &a).unwrap();
        assert!(m.pairs.is_empty(), "{:?}", m.pairs);
        assert!(m.raws.iter().all(|r| r.raw != "Heuk Bi"), "{:?}", m.raws);
        // The same original spelled another way is still one.
        assert!(raw_close("Seol-Hwi", "Seol Hwi"));
        assert!(raw_close("Gu Dae-yeom", "Gu Dae-yeon"));
        assert!(!raw_close("Heuk Bi", "Ack Bi"));
    }

    #[test]
    fn an_edited_name_is_written_into_the_translations() {
        let (conn, novel) = book("smart-merge-edit");
        let a = add(&conn, novel, "character", "Ack Bi", "Ác Bi");
        conn.execute_batch(
            r#"insert into chapter_translations (chapter_id, lang, style, source, state, title, content)
                 values (10, 'vi', 'natural', 'ai', 'published', 'Chương 1', '');
               insert into segments (chapter_id, lang, style, position, type, content) values
                 (10, 'vi', 'natural', 0, 'text', '["Ác Bi cười, Ác Binh đứng sau."]');"#,
        )
        .unwrap();
        let e = wiki::entries(&conn, novel)
            .unwrap()
            .into_iter()
            .find(|e| e.uid == a)
            .unwrap();
        let mut names = e.names.clone();
        names.insert("vi".into(), "Ách Bi".into());
        let (saved, n) = save_entry(
            &conn,
            novel,
            wiki::EntryInput {
                uid: Some(a.clone()),
                kind: e.kind.clone(),
                name: e.name.clone(),
                names,
                aliases: e.aliases.clone(),
                summaries: e.summaries.clone(),
                rank: e.rank,
                meta: e.meta.clone(),
            },
        )
        .unwrap();
        assert_eq!(shown(&saved), "Ách Bi");
        assert_eq!(n, 1);
        let text: String = conn
            .query_row(
                "select content from segments where chapter_id = 10 and lang = 'vi'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(text, r#"["Ách Bi cười, Ác Binh đứng sau."]"#);
    }
}
