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
    };
    Ok(if state.is_empty() { None } else { Some(state) })
}

/// Replaces the saved state. Snapshots are updated for the novels given and kept for
/// others still listed; novels no longer in favourites or history are dropped.
pub fn save(conn: &mut Connection, state: &ReadingState) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
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
            "insert or ignore into online_favorites (novel_id, position) values (?1, ?2)",
            params![id, pos as i64],
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
    tx.commit()
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
            },
        )
        .unwrap();
        let got = load(&c).unwrap().unwrap();
        assert_eq!(got.favs, vec![5]);
        assert!(got.novels[0].data.is_null());
    }
}
