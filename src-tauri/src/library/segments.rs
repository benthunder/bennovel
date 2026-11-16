//! A chapter's content, one row per block (paragraph or picture) in table `segments`.
//!
//! The book's own text (raw) is cut into segments when it is imported; `position` is
//! the block's place in the chapter, from 0. A translation is other rows of the same
//! table, with its language (and AI style) and the `position` of the raw segment each
//! one translates, so every translated block is tied to its place in the original
//! whatever order the parts come back in, and a chapter half translated simply has
//! no row yet for the blocks still to come.
//!
//! `content` is a JSON array of sentences for a text segment (blocks.rs cuts them)
//! and the picture's key in `chapter_images` for an img segment. `id` is SQLite's
//! own integer row id: the fastest key it has (a UUID would make every index bigger
//! and scatter the inserts); devices are matched by chapter number, language, style
//! and position, never by id.

use super::blocks;
use crate::book::image;
use rusqlite::{params, Connection};

/// `style` as the table keeps it: empty for the book's own text.
fn style_key(style: Option<&str>) -> &str {
    style.unwrap_or("")
}

/// (type, content) of the segment for paragraph `p`; None for an empty one.
fn row_of(p: &str) -> Option<(&'static str, String)> {
    if let Some(key) = image::key_of(p) {
        return Some(("img", key.to_string()));
    }
    let sentences = blocks::sentences(p);
    if sentences.is_empty() {
        return None;
    }
    Some(("text", serde_json::to_string(&sentences).ok()?))
}

/// The paragraph a segment stands for (a picture is `IMAGE_MARK + key`).
fn paragraph_of(kind: &str, content: &str) -> String {
    if kind == "img" {
        return image::paragraph(content);
    }
    match serde_json::from_str::<Vec<String>>(content) {
        Ok(sentences) => blocks::join(&sentences),
        Err(_) => content.to_string(),
    }
}

/// Writes `blocks` as the segments at positions `start..` of the chapter's text in
/// `lang` (and `style` for an AI translation), replacing those already there. An
/// empty block leaves no segment at its position.
pub fn put<S: AsRef<str>>(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: Option<&str>,
    start: usize,
    blocks: &[S],
) -> rusqlite::Result<()> {
    let style = style_key(style);
    let mut insert = conn.prepare_cached(
        "insert into segments (chapter_id, lang, style, position, type, content)
         values (?1, ?2, ?3, ?4, ?5, ?6)
         on conflict (chapter_id, lang, style, position) do update set
           type = excluded.type, content = excluded.content",
    )?;
    let mut delete = conn.prepare_cached(
        "delete from segments where chapter_id = ?1 and lang = ?2 and style = ?3 and position = ?4",
    )?;
    for (k, block) in blocks.iter().enumerate() {
        let position = (start + k) as i64;
        match row_of(block.as_ref()) {
            Some((kind, content)) => {
                insert.execute(params![chapter_id, lang, style, position, kind, content])?
            }
            None => delete.execute(params![chapter_id, lang, style, position])?,
        };
    }
    Ok(())
}

/// Makes `paragraphs` the whole text of the chapter in `lang` (and `style`): one
/// segment per paragraph, at its index.
pub fn write<S: AsRef<str>>(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: Option<&str>,
    paragraphs: &[S],
) -> rusqlite::Result<()> {
    clear(conn, chapter_id, lang, style)?;
    put(conn, chapter_id, lang, style, 0, paragraphs)
}

pub fn clear(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "delete from segments where chapter_id = ?1 and lang = ?2 and style = ?3",
        params![chapter_id, lang, style_key(style)],
    )?;
    Ok(())
}

/// The chapter's paragraphs in `lang` (and `style`), in the original's order.
pub fn paragraphs(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: Option<&str>,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "select type, content from segments
         where chapter_id = ?1 and lang = ?2 and style = ?3 order by position",
    )?;
    let rows = stmt.query_map(params![chapter_id, lang, style_key(style)], |r| {
        Ok(paragraph_of(
            &r.get::<_, String>(0)?,
            &r.get::<_, String>(1)?,
        ))
    })?;
    rows.collect()
}

/// `paragraphs`, or those of `content` for a row whose text is still kept in
/// `chapter_translations.content` (from before segments).
pub fn paragraphs_or(
    conn: &Connection,
    chapter_id: i64,
    lang: &str,
    style: Option<&str>,
    content: &str,
) -> rusqlite::Result<Vec<String>> {
    let found = paragraphs(conn, chapter_id, lang, style)?;
    if found.is_empty() {
        return Ok(blocks::paragraphs(content));
    }
    Ok(found)
}

/// SQL: whether `chapter_translations` row `t` has any text.
pub const HAS_TEXT: &str = "(exists (select 1 from segments s where s.chapter_id = t.chapter_id
    and s.lang = t.lang and s.style = coalesce(t.style, ''))
  or (length(trim(t.content)) > 0 and trim(t.content) <> '[]'))";

/// The migration: every `chapter_translations.content` becomes segments (a block's
/// index is its position) and is emptied.
pub fn migrate_all(conn: &Connection) -> rusqlite::Result<usize> {
    let mut done = 0;
    let mut after = i64::MIN;
    loop {
        let rows: Vec<(i64, i64, String, Option<String>, String)> = conn
            .prepare(
                "select id, chapter_id, lang, style, content from chapter_translations
                 where id > ?1 order by id limit 200",
            )?
            .query_map([after], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let Some(last) = rows.last() else {
            return Ok(done);
        };
        after = last.0;
        let mut empty =
            conn.prepare("update chapter_translations set content = '' where id = ?1")?;
        for (id, chapter_id, lang, style, content) in &rows {
            if content.is_empty() {
                continue;
            }
            // A block not translated yet (null) keeps its place empty.
            let slots: Vec<String> = blocks::slots(content)
                .into_iter()
                .map(Option::unwrap_or_default)
                .collect();
            write(conn, *chapter_id, lang, style.as_deref(), &slots)?;
            empty.execute([id])?;
            done += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Connection {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("bennovel-seg-{nanos}"));
        let conn = super::super::db::open(&dir.join("lib.sqlite3")).unwrap();
        conn.execute_batch(
            "insert into authors (id, slug, name) values (1, 'a', 'A');
             insert into novels (id, slug, author_id, original_lang, book_key) values (1, 'n', 1, 'zh', 'k');
             insert into chapters (id, novel_id, number) values (5, 1, 1);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn a_translation_follows_the_positions_of_the_raw_text() {
        let conn = library();
        let raw = [
            "林动睁开眼睛。“这是哪里？”他喃喃道。",
            "\u{FFFC}img_1",
            "窗外下着雨。",
        ];
        write(&conn, 5, "zh", None, &raw).unwrap();
        assert_eq!(paragraphs(&conn, 5, "zh", None).unwrap(), raw);
        let stored: Vec<(i64, String, String)> = conn
            .prepare(
                "select position, type, content from segments where lang = 'zh' order by position",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            stored,
            vec![
                (
                    0,
                    "text".into(),
                    r#"["林动睁开眼睛。","“这是哪里？”他喃喃道。"]"#.into()
                ),
                (1, "img".into(), "img_1".into()),
                (2, "text".into(), r#"["窗外下着雨。"]"#.into()),
            ]
        );

        // The last block comes back first: it still lands on position 2.
        put(
            &conn,
            5,
            "vi",
            Some("natural"),
            2,
            &["Ngoài cửa sổ trời đang mưa."],
        )
        .unwrap();
        assert_eq!(
            paragraphs(&conn, 5, "vi", Some("natural")).unwrap(),
            vec!["Ngoài cửa sổ trời đang mưa."]
        );
        put(
            &conn,
            5,
            "vi",
            Some("natural"),
            0,
            &["Lâm Động mở mắt. Hắn lẩm bẩm.", "\u{FFFC}img_1"],
        )
        .unwrap();
        assert_eq!(
            paragraphs(&conn, 5, "vi", Some("natural")).unwrap(),
            vec![
                "Lâm Động mở mắt. Hắn lẩm bẩm.",
                "\u{FFFC}img_1",
                "Ngoài cửa sổ trời đang mưa."
            ]
        );
        // The raw text is untouched, and gone with its chapter.
        assert_eq!(paragraphs(&conn, 5, "zh", None).unwrap(), raw);
        conn.execute("delete from chapters where id = 5", [])
            .unwrap();
        let left: i64 = conn
            .query_row("select count(*) from segments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }
}
