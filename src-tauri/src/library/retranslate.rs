//! Translating one name again: a wiki entry, or words the user selected in the reader.
//!
//! The device finds where the name stands in the book's own text (raw segments) and
//! takes the sentence with it and the ones on either side, with the translation of the
//! same paragraph (`contexts`). Those passages, the name as it is translated now and what
//! the wiki says of it go to the user's LLM in one request (`ask`), which answers the
//! original as the text writes it, the name to use and why. Nothing changes until the
//! user keeps the answer, maybe edited (`apply`): the entry takes the name (a new entry
//! when the words had none), the old spellings are written over in the translated
//! chapters, and smart merge looks again at the entry (`smart_merge::for_entry`), since
//! the new name may be another entry's.

use super::llm::{AiError, Chat};
use super::rules::Rule;
use super::smart_merge::{self, key, rule_ok, safe_find, shown, EntryMerge};
use super::wiki::{self, Entry, EntryInput, WIKI_LANG};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Passages sent at most.
pub const CONTEXTS: usize = 6;
/// Sentences taken on each side of the one holding the name.
const AROUND: usize = 1;
/// A passage side (original or translation) is cut to this many letters.
const PASSAGE: usize = 600;
/// Paragraphs read per name while looking for it.
const SCAN: usize = 40;

/// Where the name stands: the sentences around it in the original and in the translation.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    /// The chapter, in reading order from 1.
    pub chapter: i64,
    pub raw: String,
    pub translated: String,
}

/// What to translate again: an entry, or words selected in a chapter.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Target {
    pub uid: Option<String>,
    /// The words selected (as the reader showed them: translated or raw).
    pub text: String,
    /// The chapter they were selected in (0-based reading order).
    pub chapter_index: Option<usize>,
    /// Names the AI gave before for it ("Ask again"): another one is wanted.
    pub avoid: Vec<String>,
}

/// The AI's proposal, for the user to keep, edit or drop.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    /// The entry it is for: the one asked about, or the one the selected words are.
    pub uid: Option<String>,
    /// The original as the text writes it.
    pub raw: String,
    /// The name used now in the translation.
    pub current: String,
    /// The name the AI gives.
    pub name: String,
    /// The kind of entry (for a new one).
    pub kind: String,
    pub why: String,
    pub contexts: Vec<Context>,
}

/// What the user kept.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Keep {
    pub uid: Option<String>,
    pub raw: String,
    pub name: String,
    pub kind: String,
    /// Other spellings to write over with `name` in the translations (the words selected).
    pub old: Vec<String>,
}

/// After keeping it: the entry, the paragraphs changed, and what smart merge finds now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub entry: Entry,
    pub changed: usize,
    pub merge: EntryMerge,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Answer {
    raw: String,
    name: String,
    kind: String,
    why: String,
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

/// The sentences of a text segment (JSON list), or none.
fn sentences(content: &str) -> Vec<String> {
    serde_json::from_str(content).unwrap_or_default()
}

/// The sentences `at - AROUND ..= at + AROUND`, joined.
fn window(list: &[String], at: usize) -> String {
    let from = at.saturating_sub(AROUND);
    let to = (at + AROUND + 1).min(list.len());
    list[from..to].join(" ")
}

/// The translation of a chapter's paragraph at `position` in the wiki's language (any
/// style), as sentences.
fn translated_at(
    conn: &Connection,
    chapter_id: i64,
    position: i64,
) -> rusqlite::Result<Vec<String>> {
    let found: Option<String> = conn
        .query_row(
            "select content from segments where chapter_id = ?1 and position = ?2 and lang = ?3
               and style <> '' and type = 'text' order by style limit 1",
            params![chapter_id, position, WIKI_LANG],
            |r| r.get(0),
        )
        .ok();
    Ok(found.map(|c| sentences(&c)).unwrap_or_default())
}

/// The original paragraph at `position`, as sentences.
fn raw_at(conn: &Connection, chapter_id: i64, position: i64) -> rusqlite::Result<Vec<String>> {
    let found: Option<String> = conn
        .query_row(
            "select content from segments where chapter_id = ?1 and position = ?2 and style = ''
               and type = 'text' limit 1",
            params![chapter_id, position],
            |r| r.get(0),
        )
        .ok();
    Ok(found.map(|c| sentences(&c)).unwrap_or_default())
}

/// The passage around sentence `at` of one side, and the same place on the other side:
/// the same sentence when both have as many, else the whole paragraph.
fn pair(found: &[String], at: usize, other: &[String]) -> (String, String) {
    let mine = window(found, at);
    let theirs = if other.len() == found.len() {
        window(other, at)
    } else {
        other.join(" ")
    };
    (cut(&mine, PASSAGE), cut(&theirs, PASSAGE))
}

/// Where `names` stand in the book: in the original (`raw` true) or the translation,
/// the first chapter `first` (a chapter id) before the others; at most `max` passages,
/// one per paragraph.
pub fn contexts(
    conn: &Connection,
    novel_id: i64,
    names: &[String],
    raw: bool,
    first: Option<i64>,
    max: usize,
) -> rusqlite::Result<Vec<Context>> {
    let mut out: Vec<Context> = Vec::new();
    let mut seen: Vec<(i64, i64)> = Vec::new();
    let sql = if raw {
        "select s.chapter_id, s.position, s.content,
           (select count(*) from chapters x where x.novel_id = c.novel_id and x.number <= c.number)
         from segments s join chapters c on c.id = s.chapter_id
         where c.novel_id = ?1 and s.style = '' and s.type = 'text' and instr(s.content, ?2) > 0
         order by s.chapter_id <> coalesce(?3, -1), c.number, s.position limit ?4"
    } else {
        "select s.chapter_id, s.position, s.content,
           (select count(*) from chapters x where x.novel_id = c.novel_id and x.number <= c.number)
         from segments s join chapters c on c.id = s.chapter_id
         where c.novel_id = ?1 and s.lang = 'vi' and s.style <> '' and s.type = 'text'
           and instr(s.content, ?2) > 0
         order by s.chapter_id <> coalesce(?3, -1), c.number, s.position limit ?4"
    };
    let mut stmt = conn.prepare(sql)?;
    for name in names {
        let name = name.trim();
        if name.chars().count() < 2 {
            continue;
        }
        let rows: Vec<(i64, i64, String, i64)> = stmt
            .query_map(params![novel_id, name, first, SCAN as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (chapter_id, position, content, number) in rows {
            if out.len() >= max {
                return Ok(out);
            }
            if seen.contains(&(chapter_id, position)) {
                continue;
            }
            let list = sentences(&content);
            let Some(at) = list.iter().position(|s| s.contains(name)) else {
                continue;
            };
            seen.push((chapter_id, position));
            let (raw_text, translated) = if raw {
                let other = translated_at(conn, chapter_id, position)?;
                pair(&list, at, &other)
            } else {
                let other = raw_at(conn, chapter_id, position)?;
                let (mine, theirs) = pair(&list, at, &other);
                (theirs, mine)
            };
            out.push(Context {
                chapter: number,
                raw: raw_text,
                translated,
            });
        }
    }
    Ok(out)
}

/// The entry the selected words are: its translated name, original name or an alias.
pub fn entry_for(all: &[Entry], text: &str) -> Option<Entry> {
    let k = key(text);
    if k.is_empty() {
        return None;
    }
    all.iter()
        .find(|e| key(shown(e)) == k)
        .or_else(|| {
            all.iter()
                .find(|e| key(&e.name) == k || e.aliases.iter().any(|a| key(a) == k))
        })
        .cloned()
}

fn prompt(book: &str) -> String {
    format!(
        "You translate {book} web novels into Vietnamese. The reader pressed \"Translate \
again\" on ONE name or term: they find its current Vietnamese wrong, or want another \
rendering. You get the name as the translation writes it now, what the wiki knows, how \
this translation already renders other names (follow that style), and passages: the \
original sentences around it and their current Vietnamese translation. The current name \
and the wiki are only for reference, never something to keep.\n\
- Find the name in the original passages and give it exactly as the original writes it \
(\"raw\"). If the user gave Vietnamese words, find the original they translate.\n\
- Give a Vietnamese rendering for it (\"name\") that is DIFFERENT from the current name \
and from every name listed as rejected.\n\
- A name of a person, sect or place from a Korean, Chinese or Japanese story is given in \
its Sino-Vietnamese (Hán-Việt) reading, also when the book is an English translation \
that romanizes it: Seol-Hwi → Tuyết Huy, Ah Ryeong → A Linh, Hye Un → Huệ Vân, Lin Dong \
→ Lâm Động. Never answer with the Latin romanization (\"Chung Song\", \"Ah Ryeong\") or \
the original words unchanged.\n\
- A title, technique, item or place that has meaning gets a natural Hán-Việt or \
Vietnamese translation.\n\
- \"kind\": one of character, place, faction, item, skill, level, term.\n\
- \"why\": one short sentence in Vietnamese (which characters or meaning you read).\n\
Reply with JSON only: {{\"raw\": \"...\", \"name\": \"...\", \"kind\": \"...\", \"why\": \"...\"}}"
    )
}

/// How the book's translation renders other names (original → Vietnamese), for the AI
/// to follow: entries of the same kind first, only those it renders differently.
fn style_examples(all: &[Entry], kind: &str, skip: Option<&str>, n: usize) -> Vec<String> {
    let mut pairs: Vec<(&Entry, &str)> = all
        .iter()
        .filter(|e| Some(e.uid.as_str()) != skip)
        .filter_map(|e| {
            let vi = e.names.get(WIKI_LANG)?.trim();
            (!vi.is_empty() && vi != e.name.trim()).then_some((e, vi))
        })
        .collect();
    pairs.sort_by_key(|(e, _)| e.kind != kind);
    pairs
        .into_iter()
        .take(n)
        .map(|(e, vi)| format!("{} → {vi}", e.name.trim()))
        .collect()
}

/// Whether `name` is one the AI must not give again: the current one, a rejected one,
/// or the original written unchanged.
fn not_new(name: &str, current: &str, raw: &str, avoid: &[String]) -> bool {
    let k = key(name);
    k.is_empty()
        || k == key(current)
        || (!raw.trim().is_empty() && k == key(raw))
        || avoid.iter().any(|a| key(a) == k)
}

/// What to send for `target`: the entry (if any), the name as translated now, the
/// passages, and the names they were looked for by.
fn gather(
    conn: &Connection,
    novel_id: i64,
    target: &Target,
) -> Result<(Option<Entry>, String, Vec<Context>), AiError> {
    let all = wiki::entries(conn, novel_id)?;
    let entry = match target.uid.as_deref() {
        Some(uid) => Some(
            all.iter()
                .find(|e| e.uid == uid)
                .cloned()
                .ok_or_else(|| AiError::Other("wiki-entry-missing".into()))?,
        ),
        None => entry_for(&all, &target.text),
    };
    let first = match target.chapter_index {
        Some(i) => super::translate::chapter_at(conn, novel_id, i)?,
        None => None,
    };
    let text = target.text.trim().to_string();
    let (current, mut found) = match &entry {
        Some(e) => {
            let raws: Vec<String> = std::iter::once(e.name.clone())
                .chain(e.aliases.iter().cloned())
                .collect();
            let mut found = contexts(conn, novel_id, &raws, true, first, CONTEXTS)?;
            if found.is_empty() {
                found = contexts(
                    conn,
                    novel_id,
                    &[shown(e).to_string()],
                    false,
                    first,
                    CONTEXTS,
                )?;
            }
            (shown(e).to_string(), found)
        }
        None => {
            if text.is_empty() {
                return Err(AiError::Other("retranslate-empty".into()));
            }
            // The words may be the translation's or the original's.
            let mut found = contexts(
                conn,
                novel_id,
                std::slice::from_ref(&text),
                false,
                first,
                CONTEXTS,
            )?;
            if found.is_empty() {
                found = contexts(
                    conn,
                    novel_id,
                    std::slice::from_ref(&text),
                    true,
                    first,
                    CONTEXTS,
                )?;
            }
            (text.clone(), found)
        }
    };
    found.truncate(CONTEXTS);
    Ok((entry, current, found))
}

/// The request for `target`, as sent (for tests and the log).
fn request(
    entry: Option<&Entry>,
    current: &str,
    found: &[Context],
    avoid: &[String],
    examples: &[String],
) -> String {
    let mut lines = vec![format!("Name as translated now (give another): {current}")];
    if !avoid.is_empty() {
        lines.push(format!(
            "Rejected already (give another): {}",
            avoid.join(", ")
        ));
    }
    if let Some(e) = entry {
        lines.push(format!("Original in the wiki: {}", e.name));
        if !e.aliases.is_empty() {
            lines.push(format!("Other names: {}", e.aliases.join(", ")));
        }
        lines.push(format!("Kind: {}", e.kind));
        if let Some(s) = e
            .summaries
            .get(WIKI_LANG)
            .or_else(|| e.summaries.values().next())
        {
            lines.push(format!("Wiki: {}", cut(s, 300)));
        }
    }
    if !examples.is_empty() {
        lines.push(format!(
            "How this translation renders other names: {}",
            examples.join("; ")
        ));
    }
    if found.is_empty() {
        lines.push("No passage found.".into());
    }
    for (i, c) in found.iter().enumerate() {
        lines.push(format!(
            "[{}] Chapter {}\nOriginal: {}\nVietnamese now: {}",
            i + 1,
            c.chapter,
            c.raw,
            c.translated
        ));
    }
    lines.join("\n")
}

/// Asks the LLM for a better translation of `target`; nothing is changed.
pub fn ask(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    target: &Target,
) -> Result<Proposal, AiError> {
    let (entry, current, found) = gather(conn, novel_id, target)?;
    let book = wiki::book_lang(conn, novel_id).unwrap_or_default();
    let book = super::translate::lang_name(&book).to_string();
    let all = wiki::entries(conn, novel_id)?;
    let kind = entry
        .as_ref()
        .map(|e| e.kind.as_str())
        .unwrap_or("character");
    let examples = style_examples(&all, kind, entry.as_ref().map(|e| e.uid.as_str()), 12);
    let req = request(entry.as_ref(), &current, &found, &target.avoid, &examples);
    let parse = |reply: &str| -> Answer {
        let (s, e) = (reply.find('{'), reply.rfind('}'));
        match (s, e) {
            (Some(s), Some(e)) if s < e => serde_json::from_str(&reply[s..=e]).unwrap_or_default(),
            _ => Answer::default(),
        }
    };
    let mut a = parse(&llm.chat(&prompt(&book), &req)?);
    let raw_known = entry
        .as_ref()
        .map(|e| e.name.clone())
        .unwrap_or_else(|| a.raw.clone());
    // The same name back (or the original unchanged): asked once more, firmly.
    if not_new(&a.name, &current, &raw_known, &target.avoid) {
        let again = format!(
            "{req}\n\nYour answer \"{}\" is the current, a rejected or the untranslated name. \
Give a different Vietnamese rendering (Hán-Việt for a Korean, Chinese or Japanese name).",
            a.name.trim()
        );
        let b = parse(&llm.chat(&prompt(&book), &again)?);
        if !b.name.trim().is_empty() {
            a = b;
        }
    }
    let name = a.name.trim().to_string();
    if name.is_empty() {
        return Err(AiError::Other("retranslate-no-answer".into()));
    }
    let raw = match &entry {
        // The entry's original stays unless the AI found it is another one.
        Some(e) if a.raw.trim().is_empty() => e.name.clone(),
        _ => a.raw.trim().to_string(),
    };
    Ok(Proposal {
        uid: entry.as_ref().map(|e| e.uid.clone()),
        raw,
        current,
        name,
        kind: entry
            .as_ref()
            .map(|e| e.kind.clone())
            .unwrap_or_else(|| a.kind.trim().to_string()),
        why: a.why.trim().to_string(),
        contexts: found,
    })
}

/// Keeps a translation the user chose: the entry takes the name (a new entry of the
/// user's when the words had none), the old spellings are written over in the
/// translated chapters, and smart merge looks at the entry again.
pub fn apply(conn: &Connection, novel_id: i64, keep: Keep) -> Result<Kept, AiError> {
    let name = keep.name.trim().to_string();
    if name.is_empty() {
        return Err(AiError::Other("wiki-name-empty".into()));
    }
    let all = wiki::entries(conn, novel_id)?;
    let raw = keep.raw.trim().to_string();
    let existing = match keep.uid.as_deref() {
        Some(uid) => all.iter().find(|e| e.uid == uid).cloned(),
        None => all
            .iter()
            .find(|e| {
                !raw.is_empty()
                    && (e.name.trim() == raw || e.aliases.iter().any(|a| a.trim() == raw))
            })
            .cloned(),
    };
    let (entry, mut changed) = match existing {
        Some(e) => {
            let mut names = e.names.clone();
            names.insert(WIKI_LANG.to_string(), name.clone());
            let mut aliases = e.aliases.clone();
            // The original the AI found, when the entry did not have it.
            let taken = all.iter().any(|o| {
                o.uid != e.uid
                    && (o.name.trim() == raw || o.aliases.iter().any(|a| a.trim() == raw))
            });
            if !raw.is_empty() && raw != e.name.trim() && !aliases.contains(&raw) && !taken {
                aliases.push(raw.clone());
            }
            smart_merge::save_entry(
                conn,
                novel_id,
                EntryInput {
                    uid: Some(e.uid.clone()),
                    kind: e.kind.clone(),
                    name: e.name.clone(),
                    names,
                    aliases,
                    summaries: e.summaries.clone(),
                    rank: e.rank,
                    meta: e.meta.clone(),
                },
            )?
        }
        None => {
            let mut names = BTreeMap::new();
            names.insert(WIKI_LANG.to_string(), name.clone());
            let kind = if keep.kind.trim().is_empty() {
                "term".to_string()
            } else {
                keep.kind.trim().to_string()
            };
            let saved = wiki::save(
                conn,
                novel_id,
                EntryInput {
                    uid: None,
                    kind,
                    name: if raw.is_empty() {
                        name.clone()
                    } else {
                        raw.clone()
                    },
                    names,
                    aliases: vec![],
                    summaries: BTreeMap::new(),
                    rank: None,
                    meta: Default::default(),
                },
            )?;
            (saved, 0)
        }
    };
    // The words selected, when they were another spelling of the name.
    let others = wiki::entries(conn, novel_id)?;
    let rules: Vec<Rule> = keep
        .old
        .iter()
        .map(|o| o.trim().to_string())
        .filter(|o| {
            !o.is_empty()
                && *o != raw
                && safe_find(o, &name)
                && rule_ok(o, &name)
                && !others.iter().any(|x| x.uid != entry.uid && shown(x) == o)
        })
        .map(|find| Rule {
            find,
            replace: name.clone(),
        })
        .collect();
    changed += smart_merge::rewrite(conn, novel_id, &rules)?;
    wiki::index_names(conn, None, novel_id, WIKI_LANG)?;
    let merge = smart_merge::for_entry(conn, novel_id, &entry.uid)?;
    Ok(Kept {
        entry,
        changed,
        merge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::book;
    use std::sync::Mutex;

    struct Judge {
        reply: String,
        sent: Mutex<Vec<String>>,
    }
    impl Chat for Judge {
        fn chat(&self, _system: &str, user: &str) -> Result<String, AiError> {
            self.sent.lock().unwrap().push(user.into());
            Ok(self.reply.clone())
        }
        fn model(&self) -> &str {
            "judge"
        }
    }

    fn add(conn: &Connection, novel: i64, raw: &str, vi: &str) -> String {
        let mut names = BTreeMap::new();
        names.insert("vi".to_string(), vi.to_string());
        let e = wiki::save(
            conn,
            novel,
            EntryInput {
                uid: None,
                kind: "character".into(),
                name: raw.into(),
                names,
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank: None,
                meta: Default::default(),
            },
        )
        .unwrap();
        conn.execute(
            "update wiki_entries set source = 'ai' where uid = ?1",
            [&e.uid],
        )
        .unwrap();
        e.uid
    }

    fn text(conn: &Connection) {
        conn.execute_batch(
            r#"insert into chapter_translations (chapter_id, lang, style, source, state, title, content)
                 values (10, 'vi', 'natural', 'ai', 'published', 'Chương 1', '');
               insert into segments (chapter_id, lang, style, position, type, content) values
                 (10, 'en', '', 5, 'text', '["The night was cold.","Seol-Hwi drew his sword.","Nobody moved."]'),
                 (10, 'vi', 'natural', 5, 'text', '["Đêm lạnh.","Tuyết Hui rút kiếm.","Không ai động đậy."]');"#,
        )
        .unwrap();
    }

    #[test]
    fn an_entry_is_sent_with_the_sentences_around_it_and_kept_name_goes_into_the_text() {
        let (conn, novel) = book("retranslate-entry");
        text(&conn);
        let a = add(&conn, novel, "Seol-Hwi", "Tuyết Hui");
        let b = add(&conn, novel, "Seol Hwi", "Tuyết Huy");
        let judge = Judge {
            reply: r#"{"raw":"Seol-Hwi","name":"Tuyết Huy","kind":"character","why":"Phiên âm chuẩn."}"#.into(),
            sent: Default::default(),
        };
        let p = ask(
            &conn,
            &judge,
            novel,
            &Target {
                uid: Some(a.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (p.current.as_str(), p.name.as_str()),
            ("Tuyết Hui", "Tuyết Huy")
        );
        assert_eq!(p.contexts.len(), 1);
        assert_eq!(
            p.contexts[0].raw,
            "The night was cold. Seol-Hwi drew his sword. Nobody moved."
        );
        assert_eq!(
            p.contexts[0].translated,
            "Đêm lạnh. Tuyết Hui rút kiếm. Không ai động đậy."
        );
        assert!(judge.sent.lock().unwrap()[0].contains("Original: The night was cold."));
        let kept = apply(
            &conn,
            novel,
            Keep {
                uid: Some(a.clone()),
                raw: p.raw,
                name: p.name,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(shown(&kept.entry), "Tuyết Huy");
        assert_eq!(kept.changed, 1);
        // Back to step 1: the new name is another entry's, so they are offered as one.
        assert_eq!(kept.merge.pairs.len(), 1);
        let pair = &kept.merge.pairs[0];
        assert!([a.as_str(), b.as_str()].contains(&pair.from.uid.as_str()));
        assert!([a.as_str(), b.as_str()].contains(&pair.into.uid.as_str()));
    }

    /// Answers in turn, the last one again after that.
    struct Turns {
        replies: Vec<&'static str>,
        sent: Mutex<Vec<String>>,
    }
    impl Chat for Turns {
        fn chat(&self, _system: &str, user: &str) -> Result<String, AiError> {
            let mut sent = self.sent.lock().unwrap();
            sent.push(user.into());
            Ok(self.replies[(sent.len() - 1).min(self.replies.len() - 1)].into())
        }
        fn model(&self) -> &str {
            "turns"
        }
    }

    #[test]
    fn asking_again_never_gives_back_the_current_or_a_rejected_name() {
        let (conn, novel) = book("retranslate-other");
        text(&conn);
        let a = add(&conn, novel, "Ah Ryeong", "Ah Ryeong");
        add(&conn, novel, "Hye Un", "Huệ Vân");
        let judge = Turns {
            replies: vec![
                r#"{"raw":"Ah Ryeong","name":"Ah Ryeong","kind":"character","why":"Giữ nguyên."}"#,
                r#"{"raw":"Ah Ryeong","name":"A Linh","kind":"character","why":"Hán-Việt."}"#,
            ],
            sent: Default::default(),
        };
        let p = ask(
            &conn,
            &judge,
            novel,
            &Target {
                uid: Some(a.clone()),
                avoid: vec!["A Lăng".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.name, "A Linh");
        let sent = judge.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[0].contains("Rejected already (give another): A Lăng"));
        // The translation's style for other names goes along.
        assert!(sent[0].contains("Hye Un → Huệ Vân"));
        assert!(sent[1].contains("Give a different Vietnamese rendering"));
    }

    #[test]
    fn selected_words_without_an_entry_become_one() {
        let (conn, novel) = book("retranslate-text");
        text(&conn);
        let judge = Judge {
            reply: r#"{"raw":"Seol-Hwi","name":"Tuyết Huy","kind":"character","why":""}"#.into(),
            sent: Default::default(),
        };
        let p = ask(
            &conn,
            &judge,
            novel,
            &Target {
                text: "Tuyết Hui".into(),
                chapter_index: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.uid, None);
        assert_eq!(p.contexts.len(), 1);
        assert!(p.contexts[0].raw.contains("Seol-Hwi drew his sword."));
        let kept = apply(
            &conn,
            novel,
            Keep {
                uid: None,
                raw: p.raw,
                name: p.name,
                kind: p.kind,
                old: vec!["Tuyết Hui".into()],
            },
        )
        .unwrap();
        assert_eq!(kept.entry.name, "Seol-Hwi");
        assert_eq!(kept.entry.kind, "character");
        assert_eq!(kept.changed, 1);
        let vi: String = conn
            .query_row(
                "select content from segments where chapter_id = 10 and lang = 'vi'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            vi,
            r#"["Đêm lạnh.","Tuyết Huy rút kiếm.","Không ai động đậy."]"#
        );
    }
}
