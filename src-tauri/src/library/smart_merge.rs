//! Smart merge for a book's wiki: finds entries that are one person, place or thing
//! written two ways ("Seol-Hwi" and "Seol Hwi", "Tuyết Hui" and "Tuyết Huy") from their
//! names, mostly the Vietnamese ones the translation used, and lists them for the user
//! to review (`suggest`). Nothing is merged until the user picks pairs (`apply`).
//!
//! Every merge, smart or by hand (`merge_with_rules`), also adds a line to the book's
//! replace rules (`rules.rs`): the name the merged entry had → the kept entry's name.
//! The reader applies those rules to the text, so chapters translated before the merge
//! show the kept name without being translated again.

use super::genre_terms::{embed, fold};
use super::llm::AiError;
use super::rules::{self, Rule};
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
    /// The replace rules the merge will add (find → replace).
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

/// The replace rules merging `from` into `into` adds: the names `from` was written
/// with in the translation → the kept name. A name is left out when replacing it as
/// plain text would harm other words or other entries: a short one-word name, one the
/// kept name holds (it would be replaced inside the kept name), or one that is another
/// live entry's name.
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
        .filter(|f| safe_find(f, &to) && !taken(f))
        .map(|find| Rule {
            find,
            replace: to.clone(),
        })
        .collect()
}

/// The pairs of the book's live entries that look like one, for the user to review.
pub fn suggest(conn: &Connection, novel_id: i64) -> rusqlite::Result<Vec<Suggestion>> {
    let all = wiki::entries(conn, novel_id)?;
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
    let mut out: Vec<Suggestion> = Vec::new();
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            let (a, b) = (&all[i], &all[j]);
            if a.kind != b.kind || (a.source == "user" && b.source == "user") {
                continue;
            }
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
            {
                Reason::NearName
            } else {
                continue;
            };
            let (from, into) = if keep_first(a, b) { (b, a) } else { (a, b) };
            out.push(Suggestion {
                rules: rules_for(from, into, &all),
                from: from.clone(),
                into: into.clone(),
                reason,
                note: String::new(),
            });
        }
    }
    out.sort_by(|x, y| x.reason.cmp(&y.reason));
    out.truncate(MAX);
    Ok(out)
}

/// An original name the translation wrote as an entry's name (or close to it).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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
    let mut pairs: Vec<Suggestion> = suggest(conn, novel_id)?
        .into_iter()
        .filter(|s| s.from.uid == uid || s.into.uid == uid)
        .collect();
    for r in &raws {
        let Some(other) = r
            .entry_uid
            .as_deref()
            .and_then(|u| all.iter().find(|e| e.uid == u))
        else {
            continue;
        };
        if other.uid == uid
            || other.kind != me.kind
            || (other.source == "user" && me.source == "user")
            || pairs
                .iter()
                .any(|s| s.from.uid == other.uid || s.into.uid == other.uid)
        {
            continue;
        }
        let (from, into) = if keep_first(me, other) {
            (other, me)
        } else {
            (me, other)
        };
        pairs.push(Suggestion {
            rules: rules_for(from, into, &all),
            from: from.clone(),
            into: into.clone(),
            reason: if r.exact {
                Reason::SameName
            } else {
                Reason::NearName
            },
            note: String::new(),
        });
    }
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

/// Adds `new` to the book's replace rules in the wiki's language: a rule that already
/// finds the same text now replaces it with the new name, and one that replaced with a
/// name now merged away follows to the kept name. Returns the rules added or changed.
pub fn add_rules(conn: &Connection, novel_id: i64, new: &[Rule]) -> rusqlite::Result<usize> {
    if new.is_empty() {
        return Ok(0);
    }
    let mut list = rules::get(conn, novel_id, WIKI_LANG)?.unwrap_or_default();
    let mut changed = 0;
    for r in new {
        for old in list.iter_mut().filter(|o| o.replace == r.find) {
            old.replace = r.replace.clone();
            changed += 1;
        }
        match list.iter_mut().find(|o| o.find == r.find) {
            Some(o) if o.replace == r.replace => {}
            Some(o) => {
                o.replace = r.replace.clone();
                changed += 1;
            }
            None => {
                list.push(r.clone());
                changed += 1;
            }
        }
    }
    // A rule may now replace a name with itself (the chain came back to it).
    list.retain(|o| o.find != o.replace);
    if changed > 0 {
        rules::set(conn, novel_id, WIKI_LANG, &list)?;
    }
    Ok(changed)
}

/// Merges `from_uid` into `into_uid` as the user asked and adds the replace rules for
/// the name that went away. Returns the kept entry and the rules added.
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
    let added = add_rules(conn, novel_id, &new)?;
    Ok((kept, added))
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
    fn merging_adds_replace_rules_that_follow_later_merges() {
        let (conn, novel) = book("smart-merge-rules");
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
        let list = rules::get(&conn, novel, "vi").unwrap().unwrap();
        // "Tuyết Hui" first went to "Seol Hwi", then followed it to "Tuyết Huy".
        assert!(
            list.contains(&Rule {
                find: "Tuyết Hui".into(),
                replace: "Tuyết Huy".into()
            }),
            "{list:?}"
        );
        assert!(
            list.contains(&Rule {
                find: "Seol Hwi".into(),
                replace: "Tuyết Huy".into()
            }),
            "{list:?}"
        );
        assert!(list.iter().all(|r| r.replace == "Tuyết Huy"), "{list:?}");
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
}
