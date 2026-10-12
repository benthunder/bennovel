//! Favourites and reading history of online novels, kept in the user's library file
//! with a snapshot of each novel, so the app shows them without a connection.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HistoryEntry {
    pub id: i64,
    pub ch: i64,
    /// Epoch milliseconds of the last read.
    pub at: i64,
}

/// A novel snapshot: its id and the app's JSON for it (null when not known yet).
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: i64,
    pub data: Value,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ReadingState {
    pub favs: Vec<i64>,
    /// Most recent first.
    pub history: Vec<HistoryEntry>,
    pub novels: Vec<Snapshot>,
    /// Bumped when a sync changes the lists, so a save from lists loaded before it is
    /// refused instead of undoing the sync. Absent (0) until the first sync.
    #[serde(default)]
    pub rev: i64,
}

const REV: &str = "reading_rev";

pub fn rev(conn: &Connection) -> rusqlite::Result<i64> {
    Ok(super::db::get_state(conn, REV)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

/// Marks the lists as changed by a sync (see `ReadingState::rev`).
pub fn bump_rev(conn: &Connection) -> rusqlite::Result<()> {
    let next = rev(conn)? + 1;
    super::db::set_state(conn, REV, &next.to_string())
}

impl ReadingState {
    fn is_empty(&self) -> bool {
        self.favs.is_empty() && self.history.is_empty()
    }
}

/// The saved state, or None when nothing was saved yet.
pub fn load(conn: &Connection) -> rusqlite::Result<Option<ReadingState>> {
    let favs = conn
        .prepare("select novel_id from online_favorites order by position")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    let history = conn
        .prepare("select novel_id, chapter_number, last_read_at from online_history order by last_read_at desc, novel_id")?
        .query_map([], |r| Ok(HistoryEntry { id: r.get(0)?, ch: r.get(1)?, at: r.get(2)? }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let novels = conn
        .prepare("select id, data from online_novels order by id")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|row| match row {
            Ok((id, data)) => serde_json::from_str(&data)
                .ok()
                .map(|data| Ok(Snapshot { id, data })),
            Err(e) => Some(Err(e)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let state = ReadingState {
        favs,
        history,
        novels,
        rev: rev(conn)?,
    };
    // Lists emptied by a sync are still a saved state.
    Ok(if state.is_empty() && state.rev == 0 {
        None
    } else {
        Some(state)
    })
}

/// Replaces the saved state. Snapshots are updated for the novels given and kept for
/// others still listed; novels no longer in favourites or history are dropped, and
/// what was removed is remembered for sync. Returns false (and saves nothing) when the
/// state was loaded before the last sync.
pub fn save(conn: &mut Connection, state: &ReadingState) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if state.rev != rev(&tx)? {
        return Ok(false);
    }
    let now = super::db::now_ms();
    let old_favs: std::collections::HashMap<i64, i64> = tx
        .prepare("select novel_id, added_at from online_favorites")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let old_history: Vec<i64> = tx
        .prepare("select novel_id from online_history")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let tombstone = "insert into tombstones (kind, key, deleted_at) values (?1, ?2, ?3)
                     on conflict (kind, key) do update set deleted_at = excluded.deleted_at";
    for id in old_favs.keys().filter(|id| !state.favs.contains(id)) {
        tx.execute(tombstone, params!["fav", id.to_string(), now])?;
    }
    for id in old_history
        .iter()
        .filter(|id| !state.history.iter().any(|h| h.id == **id))
    {
        tx.execute(tombstone, params!["history", id.to_string(), now])?;
    }
    tx.execute("delete from online_favorites", [])?;
    tx.execute("delete from online_history", [])?;
    for n in state.novels.iter().filter(|n| !n.data.is_null()) {
        tx.execute(
            "insert into online_novels (id, data) values (?1, ?2)
             on conflict (id) do update set data = excluded.data,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![n.id, n.data.to_string()],
        )?;
    }
    // A novel without a snapshot yet gets a placeholder until the catalog is reached.
    let ensure =
        "insert into online_novels (id, data) values (?1, 'null') on conflict (id) do nothing";
    for (pos, id) in state.favs.iter().enumerate() {
        tx.execute(ensure, [id])?;
        tx.execute(
            "insert or ignore into online_favorites (novel_id, position, added_at) values (?1, ?2, ?3)",
            params![id, pos as i64, old_favs.get(id).copied().unwrap_or(now)],
        )?;
    }
    for h in &state.history {
        tx.execute(ensure, [h.id])?;
        tx.execute(
            "insert or replace into online_history (novel_id, chapter_number, last_read_at) values (?1, ?2, ?3)",
            params![h.id, h.ch.max(1), h.at],
        )?;
    }
    tx.execute(
        "delete from online_novels where id not in (select novel_id from online_favorites)
           and id not in (select novel_id from online_history)",
        [],
    )?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let dir = std::env::temp_dir().join(format!(
            "bennovel-reading-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        super::super::db::open(&dir.join("t.sqlite3")).unwrap()
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    fn snap(id: i64, title: &str) -> Snapshot {
        Snapshot {
            id,
            data: serde_json::json!({ "id": id, "title": title }),
        }
    }

    #[test]
    fn empty_until_saved() {
        assert!(load(&conn()).unwrap().is_none());
    }

    #[test]
    fn round_trips_and_drops_unused_snapshots() {
        let mut c = conn();
        let state = ReadingState {
            favs: vec![2, 11],
            history: vec![
                HistoryEntry {
                    id: 1,
                    ch: 12,
                    at: 200,
                },
                HistoryEntry {
                    id: 2,
                    ch: 4,
                    at: 100,
                },
            ],
            novels: vec![
                snap(1, "One"),
                snap(2, "Two"),
                snap(11, "Eleven"),
                snap(99, "Gone"),
            ],
            rev: 0,
        };
        save(&mut c, &state).unwrap();
        let got = load(&c).unwrap().unwrap();
        assert_eq!(got.favs, vec![2, 11]);
        assert_eq!(got.history, state.history);
        let ids: Vec<i64> = got.novels.iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![1, 2, 11]);
        assert_eq!(got.novels[0].data["title"], "One");

        // Unfavouriting keeps the snapshot while the novel is still in history.
        save(
            &mut c,
            &ReadingState {
                favs: vec![],
                history: vec![HistoryEntry {
                    id: 11,
                    ch: 1,
                    at: 300,
                }],
                novels: vec![],
                rev: 0,
            },
        )
        .unwrap();
        let got = load(&c).unwrap().unwrap();
        assert!(got.favs.is_empty());
        assert_eq!(
            got.novels.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![11]
        );
        assert_eq!(got.novels[0].data["title"], "Eleven");
    }

    #[test]
    fn ids_without_a_snapshot_are_kept() {
        let mut c = conn();
        save(
            &mut c,
            &ReadingState {
                favs: vec![5],
                history: vec![],
                novels: vec![],
                rev: 0,
            },
        )
        .unwrap();
        let got = load(&c).unwrap().unwrap();
        assert_eq!(got.favs, vec![5]);
        assert!(got.novels[0].data.is_null());
    }
}
