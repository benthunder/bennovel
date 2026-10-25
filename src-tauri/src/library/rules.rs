//! The reader's replace rules ("find → replace" applied to the text), one list per
//! novel and language, kept in the library file so they work offline and travel with
//! a sync. Books are keyed by their `book_key` (their local id differs on each device).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Rule {
    pub find: String,
    pub replace: String,
}

/// (scope, key) of a book's list, or None for a book that isn't in the library.
fn target(conn: &Connection, id: i64) -> rusqlite::Result<Option<(&'static str, String)>> {
    Ok(conn
        .query_row("select book_key from novels where id = ?1", [id], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()?
        .flatten()
        .map(|key| ("book", key)))
}

/// The saved list, or None when this novel and language never had one here.
pub fn get(conn: &Connection, id: i64, lang: &str) -> rusqlite::Result<Option<Vec<Rule>>> {
    let Some((scope, key)) = target(conn, id)? else {
        return Ok(None);
    };
    let json: Option<String> = conn
        .query_row(
            "select rules from rule_lists where scope = ?1 and key = ?2 and lang = ?3",
            params![scope, key, lang],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json.map(|j| serde_json::from_str(&j).unwrap_or_default()))
}

/// Replaces the list (an empty one is kept, so the removal syncs).
pub fn set(conn: &Connection, id: i64, lang: &str, rules: &[Rule]) -> rusqlite::Result<()> {
    let Some((scope, key)) = target(conn, id)? else {
        return Ok(());
    };
    let json = serde_json::to_string(rules).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "insert into rule_lists (scope, key, lang, rules, updated_at) values (?1, ?2, ?3, ?4, ?5)
         on conflict (scope, key, lang) do update set rules = excluded.rules, updated_at = excluded.updated_at",
        params![scope, key, lang, json, super::db::now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let path = crate::book::test_dir().join(format!("rules-{}.sqlite3", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        super::super::db::open(&path).unwrap()
    }

    fn rule(find: &str, replace: &str) -> Rule {
        Rule {
            find: find.into(),
            replace: replace.into(),
        }
    }

    #[test]
    fn saves_per_novel_and_language() {
        let c = conn();
        c.execute_batch(
            "insert into authors (id, slug, name) values (1, 'a', 'A');
             insert into novels (id, slug, author_id, original_lang, book_key) values (5, 'b', 1, 'vi', 'k5');",
        )
        .unwrap();
        let book = 5;
        assert_eq!(get(&c, book, "vi").unwrap(), None);
        set(&c, book, "vi", &[rule("Lâm", "Lin"), rule("a", "")]).unwrap();
        assert_eq!(
            get(&c, book, "vi").unwrap(),
            Some(vec![rule("Lâm", "Lin"), rule("a", "")])
        );
        assert_eq!(get(&c, book, "en").unwrap(), None);
        set(&c, book, "vi", &[]).unwrap();
        assert_eq!(get(&c, book, "vi").unwrap(), Some(vec![]));
        // A book that isn't in the library has no list.
        set(&c, 99, "vi", &[rule("x", "y")]).unwrap();
        assert_eq!(get(&c, 99, "vi").unwrap(), None);
    }
}
