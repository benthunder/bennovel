//! The cast of a book as its translations see it: each character's gender and age, and
//! how two characters address each other (what one calls themself and the other, and
//! the tone), both as the original text writes it (raw) and as each translation writes
//! it. It comes from the facts the translation requests bring (translate.rs WIKI_MARK,
//! alongside the wiki's), costs no request of its own, and goes back into the prompts of
//! later chapters, picked by the raw names each part of a chapter has, so the
//! translation stays consistent.
//! Kept on this device only (`cast_people`, `cast_address_raw`, `cast_address`),
//! whether or not the wiki learns.

use super::names::Finder;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Characters put in a prompt at most.
const MAX_PEOPLE: usize = 30;
/// Forms of address put in a prompt at most.
const MAX_ADDRESS: usize = 40;

/// The two lists the translation requests add to their facts.
pub fn shape(lang: &str) -> String {
    format!(
        "\"people\": [{{\"name\": original name of a character, \"gender\": male, female or unknown, \"age\": child, teen, young adult, adult, middle-aged or elder}}] for the characters in the text, and \"address\": [{{\"from\": original name of the speaker, \"to\": original name of the one spoken to, \"self_raw\": how the speaker refers to themself as written in the original text, \"calls_raw\": how the speaker calls the other as written in the original text, \"self\": the same in the {lang} translation, \"calls\": the same in the {lang} translation, \"tone\": respectful, familiar, intimate, cold, hostile or playful}}] for each pair who speak to each other",
        lang = super::translate::lang_name(lang)
    )
}

#[derive(Debug, Default, Deserialize)]
struct Facts {
    #[serde(default)]
    people: Vec<Person>,
    #[serde(default)]
    address: Vec<Address>,
}

#[derive(Debug, Deserialize)]
struct Person {
    name: String,
    #[serde(default)]
    gender: String,
    #[serde(default)]
    age: String,
}

#[derive(Debug, Deserialize)]
struct Address {
    from: String,
    to: String,
    #[serde(default)]
    self_raw: String,
    #[serde(default)]
    calls_raw: String,
    #[serde(default, rename = "self")]
    self_term: String,
    #[serde(default)]
    calls: String,
    #[serde(default)]
    tone: String,
}

/// The JSON object anywhere in a reply part.
fn parse(found: &str) -> Option<Facts> {
    let (a, b) = (found.find('{')?, found.rfind('}')?);
    serde_json::from_str(found.get(a..=b)?).ok()
}

/// "unknown" and the like say nothing.
fn known(s: &str) -> &str {
    let s = s.trim();
    if s.eq_ignore_ascii_case("unknown") {
        ""
    } else {
        s
    }
}

/// Keeps who the characters of a chapter are and how they address each other, as the
/// translation into `lang` found them. A later chapter's words replace an earlier one's;
/// what a part leaves out stays as it was. Returns the rows written.
pub fn keep(
    conn: &Connection,
    novel_id: i64,
    chapter_id: i64,
    lang: &str,
    found: &[String],
) -> rusqlite::Result<usize> {
    let chapter: Option<i64> = conn
        .query_row(
            "select number from chapters where id = ?1",
            [chapter_id],
            |r| r.get(0),
        )
        .optional()?;
    let now = super::db::now_ms();
    let mut n = 0;
    for facts in found.iter().filter_map(|f| parse(f)) {
        for p in facts.people {
            let name = p.name.trim();
            if name.is_empty() {
                continue;
            }
            n += conn.execute(
                "insert into cast_people (novel_id, name, gender, age, chapter, updated_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6)
                 on conflict (novel_id, name) do update set
                   gender = case when excluded.gender <> '' then excluded.gender else gender end,
                   age = case when excluded.age <> '' then excluded.age else age end,
                   chapter = excluded.chapter, updated_at = excluded.updated_at
                 where coalesce(excluded.chapter, 0) >= coalesce(cast_people.chapter, 0)",
                params![
                    novel_id,
                    name,
                    known(&p.gender),
                    known(&p.age),
                    chapter,
                    now
                ],
            )?;
        }
        for a in facts.address {
            let (from, to) = (a.from.trim(), a.to.trim());
            if from.is_empty() || to.is_empty() || from == to {
                continue;
            }
            n += conn.execute(
                "insert into cast_address_raw (novel_id, from_name, to_name, self_term,
                   other_term, tone, chapter, updated_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 on conflict (novel_id, from_name, to_name) do update set
                   self_term = case when excluded.self_term <> '' then excluded.self_term else self_term end,
                   other_term = case when excluded.other_term <> '' then excluded.other_term else other_term end,
                   tone = case when excluded.tone <> '' then excluded.tone else tone end,
                   chapter = excluded.chapter, updated_at = excluded.updated_at
                 where coalesce(excluded.chapter, 0) >= coalesce(cast_address_raw.chapter, 0)",
                params![
                    novel_id,
                    from,
                    to,
                    known(&a.self_raw),
                    known(&a.calls_raw),
                    known(&a.tone),
                    chapter,
                    now
                ],
            )?;
            if known(&a.self_term).is_empty() && known(&a.calls).is_empty() {
                continue;
            }
            n += conn.execute(
                "insert into cast_address (novel_id, lang, from_name, to_name, self_term,
                   other_term, chapter, updated_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 on conflict (novel_id, lang, from_name, to_name) do update set
                   self_term = case when excluded.self_term <> '' then excluded.self_term else self_term end,
                   other_term = case when excluded.other_term <> '' then excluded.other_term else other_term end,
                   chapter = excluded.chapter, updated_at = excluded.updated_at
                 where coalesce(excluded.chapter, 0) >= coalesce(cast_address.chapter, 0)",
                params![
                    novel_id,
                    lang,
                    from,
                    to,
                    known(&a.self_term),
                    known(&a.calls),
                    chapter,
                    now
                ],
            )?;
        }
    }
    Ok(n)
}

/// One pair's forms of address, raw and in the prompt's language.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pair {
    from: String,
    to: String,
    self_raw: String,
    calls_raw: String,
    tone: String,
    self_term: Option<String>,
    calls: Option<String>,
}

/// The cast a chapter names, loaded once and put in each part's prompt (`prompt`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cast {
    /// Name, gender, age.
    people: Vec<(String, String, String)>,
    pairs: Vec<Pair>,
}

/// Which of `names` the text has, in one pass (names.rs).
fn present<'a>(names: impl Iterator<Item = &'a str>, text: &str) -> HashSet<String> {
    let names: Vec<Vec<&str>> = names.map(|n| vec![n]).collect();
    let hits = Finder::new(&names).hits(text);
    names
        .into_iter()
        .zip(hits)
        .filter(|(_, h)| *h > 0)
        .map(|(n, _)| n[0].to_string())
        .collect()
}

/// The cast of a book that chapter `text` (its original text) names, with the words the
/// translation into `lang` used before. Picked by the raw names.
pub fn load(conn: &Connection, novel_id: i64, text: &str, lang: &str) -> rusqlite::Result<Cast> {
    let mut stmt = conn.prepare(
        "select name, gender, age from cast_people where novel_id = ?1
           and (gender <> '' or age <> '') order by name",
    )?;
    let people: Vec<(String, String, String)> = stmt
        .query_map([novel_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare(
        "select r.from_name, r.to_name, r.self_term, r.other_term, r.tone, t.self_term, t.other_term
         from cast_address_raw r
         left join cast_address t on t.novel_id = r.novel_id and t.lang = ?2
           and t.from_name = r.from_name and t.to_name = r.to_name
         where r.novel_id = ?1 order by r.from_name, r.to_name",
    )?;
    let pairs: Vec<Pair> = stmt
        .query_map(params![novel_id, lang], |r| {
            Ok(Pair {
                from: r.get(0)?,
                to: r.get(1)?,
                self_raw: r.get(2)?,
                calls_raw: r.get(3)?,
                tone: r.get(4)?,
                self_term: r.get(5)?,
                calls: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Cast::from(people, pairs, text))
}

impl Cast {
    /// Only the people, and the pairs with both people, `text` names.
    fn from(people: Vec<(String, String, String)>, pairs: Vec<Pair>, text: &str) -> Self {
        let names = people
            .iter()
            .map(|p| p.0.as_str())
            .chain(pairs.iter().flat_map(|p| [p.from.as_str(), p.to.as_str()]));
        let here = present(names, text);
        Self {
            people: people.into_iter().filter(|p| here.contains(&p.0)).collect(),
            pairs: pairs
                .into_iter()
                .filter(|p| here.contains(&p.from) && here.contains(&p.to))
                .collect(),
        }
    }

    /// The cast part of the prompt for one part of the chapter: the characters it names
    /// with their gender and age, and how those pairs address each other, raw words and
    /// the words this translation used for them. Empty when nothing is known of them.
    pub fn prompt(&self, text: &str) -> String {
        let mine = Cast::from(self.people.clone(), self.pairs.clone(), text);
        let mut s = String::new();
        if !mine.people.is_empty() {
            s.push_str("\nCharacters in this text (keep their gender and age in pronouns and forms of address):\n");
            for (name, gender, age) in mine.people.iter().take(MAX_PEOPLE) {
                let about: Vec<&str> = [gender.as_str(), age.as_str()]
                    .into_iter()
                    .filter(|x| !x.is_empty())
                    .collect();
                s.push_str(&format!("- {name}: {}\n", about.join(", ")));
            }
        }
        if !mine.pairs.is_empty() {
            s.push_str("\nHow they address each other (speaker → listener: how the speaker calls themself; how they call the listener; tone), as the original writes it → as this translation wrote it before; keep it unless the story changes their relationship:\n");
            for p in mine.pairs.iter().take(MAX_ADDRESS) {
                let word = |raw: &str, done: &Option<String>| match done.as_deref() {
                    Some(t) if !t.is_empty() && !raw.is_empty() => format!("{raw} → {t}"),
                    Some(t) if !t.is_empty() => format!("→ {t}"),
                    _ if !raw.is_empty() => raw.to_string(),
                    _ => "—".into(),
                };
                let mut line = format!(
                    "- {} → {}: {}; {}",
                    p.from,
                    p.to,
                    word(&p.self_raw, &p.self_term),
                    word(&p.calls_raw, &p.calls)
                );
                if !p.tone.is_empty() {
                    line.push_str(&format!("; {}", p.tone));
                }
                s.push_str(&line);
                s.push('\n');
            }
        }
        s
    }
}

/// `load` and `prompt` for one text.
#[cfg(test)]
fn context(conn: &Connection, novel_id: i64, text: &str, lang: &str) -> rusqlite::Result<String> {
    Ok(load(conn, novel_id, text, lang)?.prompt(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::book;

    #[test]
    fn keeps_the_cast_raw_and_translated_and_puts_it_back_in_prompts() {
        let (conn, novel) = book("cast");
        let found = vec![
            r#"{"entries": [], "people": [{"name": "林动", "gender": "male", "age": "young adult"},
                {"name": "青竹", "gender": "female", "age": "unknown"}],
              "address": [{"from": "林动", "to": "青竹", "self_raw": "我", "calls_raw": "你",
                "self": "ta", "calls": "nàng", "tone": "intimate"}]}"#
                .to_string(),
            "not json".to_string(),
        ];
        // Two people, the raw pair and its Vietnamese words.
        assert_eq!(keep(&conn, novel, 11, "vi", &found).unwrap(), 4);
        // An earlier chapter does not undo what a later one found.
        let earlier = vec![
            r#"{"address": [{"from": "林动", "to": "青竹", "self_raw": "在下", "self": "tại hạ"}]}"#
                .to_string(),
        ];
        assert_eq!(keep(&conn, novel, 10, "vi", &earlier).unwrap(), 0);

        let s = context(&conn, novel, "林动看着青竹。", "vi").unwrap();
        assert!(s.contains("- 林动: male, young adult"));
        assert!(s.contains("- 青竹: female"));
        assert!(s.contains("- 林动 → 青竹: 我 → ta; 你 → nàng; intimate"));
        // Each part only gets who it names.
        let cast = load(&conn, novel, "林动看着青竹。", "vi").unwrap();
        let part = cast.prompt("林动走了。");
        assert!(part.contains("- 林动: male") && !part.contains("青竹"));
        // Picked by the raw names the chapter has.
        assert!(context(&conn, novel, "Nobody here.", "vi")
            .unwrap()
            .is_empty());
        // Another language: the raw words, not the Vietnamese ones.
        let en = context(&conn, novel, "林动看着青竹。", "en").unwrap();
        assert!(en.contains("- 林动 → 青竹: 我; 你; intimate"));
    }
}
