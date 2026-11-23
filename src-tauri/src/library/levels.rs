//! The order of a book's levels (cảnh giới): each ladder (`Meta::system`: cultivation,
//! alchemy…) with its levels lowest first (`Entry::rank`).
//!
//! The AI writes a rank and a ladder per chapter as it translates, so a long book ends
//! with levels in no ladder, ranks that clash or none at all. Two ways put them right:
//! the user moves a level up or down (`set_ladder` with the ladder's new order), or asks
//! the AI to order every level at once (`ask` then `set_ladder` per ladder it answers).
//! Either way the levels become the user's: the AI's later chapters no longer move them.

use super::llm::{AiError, Chat};
use super::wiki::{self, Entry, WIKI_LANG};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Levels sent to the AI at most.
const MAX: usize = 400;
/// A description is cut to this many letters.
const SUMMARY: usize = 120;

/// One ladder as the AI or the user ordered it: its levels lowest first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ladder {
    pub system: String,
    pub uids: Vec<String>,
}

/// Makes `uids` (lowest first) the ladder `system`: each takes its place as its rank
/// (from 1) and the ladder's name, and is the user's from now on. Returns the levels set.
pub fn set_ladder(
    conn: &Connection,
    novel_id: i64,
    system: &str,
    uids: &[String],
) -> rusqlite::Result<usize> {
    let system = system.trim();
    let now = super::db::now_ms();
    let mut done = 0;
    for (i, uid) in uids.iter().enumerate() {
        done += conn.execute(
            "update wiki_entries set rank = ?3,
               meta = case when ?4 = '' then json_remove(coalesce(nullif(meta, ''), '{}'), '$.system')
                      else json_set(coalesce(nullif(meta, ''), '{}'), '$.system', ?4) end,
               source = 'user', updated_at = ?5
             where uid = ?1 and novel_id = ?2 and kind = 'level' and deleted = 0",
            params![uid, novel_id, (i + 1) as i64, system, now],
        )?;
    }
    Ok(done)
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

fn line(n: usize, e: &Entry) -> String {
    let shown = e.names.get(WIKI_LANG).map(String::as_str).unwrap_or("");
    let mut parts = vec![format!("[{n}] {}", e.name)];
    if !shown.is_empty() && shown != e.name {
        parts.push(format!("= {shown}"));
    }
    if !e.meta.system.is_empty() {
        parts.push(format!("ladder: {}", e.meta.system));
    }
    if let Some(r) = e.rank {
        parts.push(format!("rank: {r}"));
    }
    if let Some(c) = e.first_chapter {
        parts.push(format!("first in chapter {c}"));
    }
    if let Some(s) = e
        .summaries
        .get(WIKI_LANG)
        .or_else(|| e.summaries.values().next())
    {
        parts.push(format!("— {}", cut(s, SUMMARY)));
    }
    parts.join(" | ")
}

const PROMPT: &str = "You order the levels (cảnh giới, realms, ranks) of a web novel. You get \
every level the wiki knows, numbered, with its Vietnamese name, the ladder and rank the \
translation gave (often wrong or missing), where it first appears and a description.\n\
- Put the levels in ladders: one per system of the story (cultivation, body tempering, \
alchemy, rune master, beast ranks…). Name each ladder in Vietnamese, short; keep a name \
already used when it fits.\n\
- In each ladder, list the levels from lowest to highest. A minor stage (sơ kỳ, trung kỳ, \
hậu kỳ, đỉnh phong; tầng 1…9) comes right after its major realm, in order.\n\
- Every number goes in exactly one ladder. Do not invent levels.\n\
Reply with JSON only: {\"ladders\": [{\"system\": \"...\", \"ids\": [3, 1, 2]}]}";

#[derive(Deserialize, Default)]
#[serde(default)]
struct Answer {
    ladders: Vec<AnswerLadder>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AnswerLadder {
    system: String,
    ids: Vec<usize>,
}

/// Asks the AI to order every level of the book into ladders; nothing is changed.
/// Each level is in one ladder at most, as the AI first placed it.
pub fn ask(conn: &Connection, llm: &dyn Chat, novel_id: i64) -> Result<Vec<Ladder>, AiError> {
    let mut levels: Vec<Entry> = wiki::entries(conn, novel_id)?
        .into_iter()
        .filter(|e| e.kind == "level")
        .collect();
    levels.truncate(MAX);
    if levels.is_empty() {
        return Ok(vec![]);
    }
    let list: Vec<String> = levels
        .iter()
        .enumerate()
        .map(|(i, e)| line(i + 1, e))
        .collect();
    let reply = llm.chat(PROMPT, &list.join("\n"))?;
    let answer: Answer = match (reply.find('{'), reply.rfind('}')) {
        (Some(a), Some(b)) if a < b => serde_json::from_str(&reply[a..=b]).unwrap_or_default(),
        _ => Answer::default(),
    };
    let mut seen = vec![false; levels.len()];
    let mut out = Vec::new();
    for l in answer.ladders {
        let uids: Vec<String> = l
            .ids
            .into_iter()
            .filter(|i| *i >= 1 && *i <= levels.len())
            .filter(|i| !std::mem::replace(&mut seen[i - 1], true))
            .map(|i| levels[i - 1].uid.clone())
            .collect();
        if !uids.is_empty() {
            out.push(Ladder {
                system: l.system.trim().to_string(),
                uids,
            });
        }
    }
    if out.is_empty() {
        return Err(AiError::Other("levels-no-answer".into()));
    }
    Ok(out)
}

/// `ask`, then each ladder set as the AI ordered it. Returns the ladders.
pub fn order_by_ai(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
) -> Result<Vec<Ladder>, AiError> {
    let ladders = ask(conn, llm, novel_id)?;
    for l in &ladders {
        set_ladder(conn, novel_id, &l.system, &l.uids)?;
    }
    Ok(ladders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::book;
    use crate::library::wiki::EntryInput;
    use std::collections::BTreeMap;

    struct Judge(String);
    impl Chat for Judge {
        fn chat(&self, _s: &str, _u: &str) -> Result<String, AiError> {
            Ok(self.0.clone())
        }
        fn model(&self) -> &str {
            "judge"
        }
    }

    fn level(conn: &Connection, novel: i64, name: &str, rank: Option<i64>) -> String {
        let e = wiki::save(
            conn,
            novel,
            EntryInput {
                uid: None,
                kind: "level".into(),
                name: name.into(),
                names: BTreeMap::new(),
                aliases: vec![],
                summaries: BTreeMap::new(),
                rank,
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

    #[test]
    fn the_ai_orders_levels_into_ladders_and_they_become_the_users() {
        let (conn, novel) = book("levels-ai");
        let jin = level(&conn, novel, "Kim Đan", Some(1));
        let zhu = level(&conn, novel, "Trúc Cơ", Some(1));
        let lian = level(&conn, novel, "Luyện Khí", None);
        let dan = level(&conn, novel, "Nhất phẩm Đan sư", None);
        // The list goes by name: Kim Đan, Luyện Khí, Nhất phẩm Đan sư, Trúc Cơ.
        let judge = Judge(
            r#"{"ladders":[{"system":"Tu luyện","ids":[2,4,1,2]},{"system":"Luyện đan","ids":[3,9]}]}"#.into(),
        );
        let ladders = order_by_ai(&conn, &judge, novel).unwrap();
        assert_eq!(
            ladders,
            vec![
                Ladder {
                    system: "Tu luyện".into(),
                    uids: vec![lian.clone(), zhu.clone(), jin.clone()]
                },
                Ladder {
                    system: "Luyện đan".into(),
                    uids: vec![dan.clone()]
                },
            ]
        );
        let all = wiki::entries(&conn, novel).unwrap();
        let get = |u: &str| all.iter().find(|e| e.uid == u).unwrap();
        assert_eq!(
            (get(&lian).rank, get(&zhu).rank, get(&jin).rank),
            (Some(1), Some(2), Some(3))
        );
        assert_eq!(get(&jin).meta.system, "Tu luyện");
        assert_eq!(get(&dan).meta.system, "Luyện đan");
        assert_eq!(get(&jin).source, "user");
    }

    #[test]
    fn moving_a_level_renumbers_its_ladder() {
        let (conn, novel) = book("levels-move");
        let a = level(&conn, novel, "A", Some(1));
        let b = level(&conn, novel, "B", Some(2));
        set_ladder(&conn, novel, "Võ đạo", &[b.clone(), a.clone()]).unwrap();
        let all = wiki::entries(&conn, novel).unwrap();
        let get = |u: &str| all.iter().find(|e| e.uid == u).unwrap();
        assert_eq!((get(&b).rank, get(&a).rank), (Some(1), Some(2)));
        assert_eq!(get(&a).meta.system, "Võ đạo");
        // No ladder name: the level leaves its ladder.
        set_ladder(&conn, novel, "", &[a.clone()]).unwrap();
        let a2 = wiki::entries(&conn, novel)
            .unwrap()
            .into_iter()
            .find(|e| e.uid == a)
            .unwrap();
        assert_eq!(a2.meta.system, "");
    }
}
