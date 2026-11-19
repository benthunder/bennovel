//! The AI step of smart merge for the whole wiki as a queue, like the translation's
//! (`queue.rs`): asking cuts the wiki in blocks (`smart_merge_ai::wiki_blocks`) and puts
//! one row per block in `wiki_merge_blocks`; a background task asks the LLM about them,
//! `PARALLEL` at a time, and keeps each block's answer. Rows stay in the library file,
//! so a run cut short by closing the app carries on the next time it starts
//! (`recover`, from `ai_resume`), and the user can review the groups found so far while
//! the rest is still being read (`status`).
//!
//! A lost connection, a busy server or a rate limit is tried again a few times; when it
//! still fails, the book's blocks left wait as `paused` with the error and carry on when
//! the app starts again or the user asks (`resume`). A block whose answer can't be used
//! fails alone and can be tried again (`retry`).
//!
//! `wiki-merge-progress` is sent after each block with the book's counts.

use super::db;
use super::llm::{self, AiError, Chat};
use super::smart_merge;
use super::smart_merge_ai::{self, Answered, Group, Records, PARALLEL};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::mpsc;

/// Tries of a block when the LLM can't be reached, and the wait before each next one.
const WAITS: [u64; 2] = [15, 45];

/// A book's run: its blocks by state, and the groups found so far.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub novel_id: i64,
    pub total: i64,
    pub done: i64,
    pub failed: i64,
    pub queued: i64,
    pub running: i64,
    pub paused: i64,
    /// Why the run waits (or a block failed), as an `AiError` text.
    pub error: Option<String>,
    /// The groups of the blocks done, against the wiki as it is now.
    pub groups: Vec<Group>,
}

/// Starts a book's run: the wiki cut in blocks, all queued. A run with blocks left
/// carries on instead (its paused and failed blocks queued again), unless `fresh`.
/// Returns the blocks queued.
pub fn start(conn: &Connection, novel_id: i64, fresh: bool) -> rusqlite::Result<usize> {
    let left: i64 = conn.query_row(
        "select count(*) from wiki_merge_blocks where novel_id = ?1 and status <> 'done'",
        [novel_id],
        |r| r.get(0),
    )?;
    if left > 0 && !fresh {
        return conn.execute(
            "update wiki_merge_blocks set status = 'queued', error = null
             where novel_id = ?1 and status in ('paused', 'failed')",
            [novel_id],
        );
    }
    let blocks = smart_merge_ai::wiki_blocks(conn, novel_id)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "delete from wiki_merge_blocks where novel_id = ?1",
        [novel_id],
    )?;
    let now = db::now_ms();
    for (i, b) in blocks.iter().enumerate() {
        tx.execute(
            "insert into wiki_merge_blocks (novel_id, idx, uids, created_at) values (?1, ?2, ?3, ?4)",
            params![novel_id, i as i64, serde_json::to_string(b).unwrap_or_default(), now],
        )?;
    }
    tx.commit()?;
    Ok(blocks.len())
}

/// Marks the next queued block running and returns it: runs in the order they came.
pub fn claim(conn: &Connection) -> rusqlite::Result<Option<(i64, i64, Vec<String>)>> {
    conn.query_row(
        "update wiki_merge_blocks set status = 'running' where rowid = (
           select rowid from wiki_merge_blocks where status = 'queued'
           order by created_at, novel_id, idx limit 1)
         returning novel_id, idx, uids",
        [],
        |r| {
            let uids: String = r.get(2)?;
            Ok((
                r.get(0)?,
                r.get(1)?,
                serde_json::from_str(&uids).unwrap_or_default(),
            ))
        },
    )
    .optional()
}

/// Keeps a block's answer, or why it failed. Left alone when the run was cleared or
/// started again meanwhile (the row is gone or no longer running).
pub fn finish(
    conn: &Connection,
    novel_id: i64,
    idx: i64,
    answer: Result<&[Answered], &str>,
) -> rusqlite::Result<()> {
    let (status, groups, error) = match answer {
        Ok(a) => ("done", serde_json::to_string(a).ok(), None),
        Err(e) => ("failed", None, Some(e)),
    };
    conn.execute(
        "update wiki_merge_blocks set status = ?3, groups = ?4, error = ?5
         where novel_id = ?1 and idx = ?2 and status = 'running'",
        params![novel_id, idx, status, groups, error],
    )?;
    Ok(())
}

/// The LLM can't be used now: the block and the book's blocks still queued wait, with
/// the reason, until the app starts again or the user carries on.
fn hold(conn: &Connection, novel_id: i64, idx: i64, error: &str) -> rusqlite::Result<()> {
    conn.execute(
        "update wiki_merge_blocks set status = 'paused', error = ?3
         where novel_id = ?1 and ((idx = ?2 and status = 'running') or status = 'queued')",
        params![novel_id, idx, error],
    )?;
    Ok(())
}

/// Blocks left running when the app closed go back in the queue, and so do the ones the
/// LLM could not be used for; the ones the user paused wait.
pub fn recover(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "update wiki_merge_blocks set status = 'queued', error = null
         where status = 'running' or (status = 'paused' and error is not null)",
        [],
    )
}

/// What the user can do with a book's run.
pub fn control(conn: &Connection, novel_id: i64, action: &str) -> rusqlite::Result<usize> {
    let sql = match action {
        // The blocks being asked finish; the ones queued wait.
        "pause" => {
            "update wiki_merge_blocks set status = 'paused', error = null
             where novel_id = ?1 and status = 'queued'"
        }
        "resume" => {
            "update wiki_merge_blocks set status = 'queued', error = null
             where novel_id = ?1 and status = 'paused'"
        }
        "retry" => {
            "update wiki_merge_blocks set status = 'queued', error = null
             where novel_id = ?1 and status = 'failed'"
        }
        "clear" => "delete from wiki_merge_blocks where novel_id = ?1",
        _ => return Ok(0),
    };
    conn.execute(sql, [novel_id])
}

/// A book's counts, without the groups.
pub fn counts(conn: &Connection, novel_id: i64) -> rusqlite::Result<Status> {
    conn.query_row(
        "select count(*), coalesce(sum(status = 'done'), 0), coalesce(sum(status = 'failed'), 0),
           coalesce(sum(status = 'queued'), 0), coalesce(sum(status = 'running'), 0),
           coalesce(sum(status = 'paused'), 0),
           coalesce(max(case when status = 'paused' then error end),
                    max(case when status = 'failed' then error end))
         from wiki_merge_blocks where novel_id = ?1",
        [novel_id],
        |r| {
            Ok(Status {
                novel_id,
                total: r.get(0)?,
                done: r.get(1)?,
                failed: r.get(2)?,
                queued: r.get(3)?,
                running: r.get(4)?,
                paused: r.get(5)?,
                error: r.get(6)?,
                groups: Vec::new(),
            })
        },
    )
}

/// A book's run with the groups its blocks found, against the wiki now: a group already
/// merged or renamed is gone from the list.
pub fn status(conn: &Connection, novel_id: i64) -> rusqlite::Result<Status> {
    let mut s = counts(conn, novel_id)?;
    if s.done == 0 {
        return Ok(s);
    }
    let rec = Records::load(conn, novel_id)?;
    let mut stmt = conn.prepare(
        "select groups from wiki_merge_blocks where novel_id = ?1 and status = 'done' order by idx",
    )?;
    let answers: Vec<String> = stmt
        .query_map([novel_id], |r| r.get::<_, Option<String>>(0))?
        .filter_map(|r| r.transpose())
        .collect::<rusqlite::Result<_>>()?;
    // Groups the user set aside as already right are not shown again.
    let mut seen: Vec<String> = smart_merge::skipped(conn, novel_id)?.into_iter().collect();
    for a in answers {
        let list: Vec<Answered> = serde_json::from_str(&a).unwrap_or_default();
        for g in list.iter().filter_map(|a| smart_merge_ai::build(&rec, a)) {
            let id = g.key();
            if !seen.contains(&id) {
                seen.push(id);
                s.groups.push(g);
            }
        }
    }
    Ok(s)
}

/// Asks about one block; a lost connection, a busy server or a rate limit is tried
/// again after `WAITS`.
fn ask(rec: &Records, llm: &dyn Chat, uids: &[String]) -> Result<Vec<Answered>, AiError> {
    let block: Vec<_> = uids.iter().filter_map(|u| rec.get(u)).collect();
    if block.is_empty() {
        return Ok(Vec::new());
    }
    let mut waits = WAITS.iter();
    loop {
        match smart_merge_ai::ask_block(rec, llm, &block, false) {
            Err(e) if again(&e) => match waits.next() {
                Some(w) => std::thread::sleep(Duration::from_secs(*w)),
                None => return Err(e),
            },
            other => return other,
        }
    }
}

/// Errors worth another try in a while (not a missing setting or a refused key).
fn again(e: &AiError) -> bool {
    super::queue::pauses(e) && !matches!(e, AiError::NotSet | AiError::Auth(_))
}

/// Works through the library file's queued blocks, `PARALLEL` at a time, until none is
/// left; `told` gets each book's counts after each block.
pub fn run(db_file: &Path, llm: &(dyn Chat + Sync), told: &(dyn Fn(Status) + Sync)) {
    std::thread::scope(|s| {
        for _ in 0..PARALLEL {
            s.spawn(|| {
                let Ok(conn) = db::open(db_file) else { return };
                let mut books: HashMap<i64, Records> = HashMap::new();
                while let Ok(Some((novel_id, idx, uids))) = claim(&conn) {
                    let rec = match books.get(&novel_id) {
                        Some(r) => r,
                        None => match Records::load(&conn, novel_id) {
                            Ok(r) => books.entry(novel_id).or_insert(r),
                            Err(e) => {
                                let _ = finish(&conn, novel_id, idx, Err(&e.to_string()));
                                continue;
                            }
                        },
                    };
                    let result = ask(rec, llm, &uids);
                    let wrote = match &result {
                        Ok(a) => finish(&conn, novel_id, idx, Ok(a)),
                        Err(e) if super::queue::pauses(e) => {
                            hold(&conn, novel_id, idx, &e.to_string())
                        }
                        Err(e) => finish(&conn, novel_id, idx, Err(&e.to_string())),
                    };
                    if let Err(e) = wrote {
                        eprintln!("merge queue: {e}");
                    }
                    if let Ok(c) = counts(&conn, novel_id) {
                        told(c);
                    }
                }
            });
        }
    });
}

/// Holds every queued block of the library file with `error` (no LLM set up).
fn hold_all(conn: &Connection, error: &str) -> rusqlite::Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("select distinct novel_id from wiki_merge_blocks where status = 'queued'")?;
    let books: Vec<i64> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    conn.execute(
        "update wiki_merge_blocks set status = 'paused', error = ?1 where status = 'queued'",
        [error],
    )?;
    Ok(books)
}

struct Wake {
    db: PathBuf,
    user: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    user: String,
    #[serde(flatten)]
    status: Status,
}

/// The merge queue's handle, kept in the app's state.
pub struct MergeQueue {
    tx: mpsc::UnboundedSender<Wake>,
}

impl MergeQueue {
    /// Starts the worker: a task that empties a library's queued blocks each time it is
    /// woken.
    pub fn start<R: Runtime>(app: AppHandle<R>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Wake>();
        tauri::async_runtime::spawn(async move {
            while let Some(wake) = rx.recv().await {
                let mut wakes = vec![wake];
                // Wakes that came meanwhile for the same file are one turn.
                while let Ok(w) = rx.try_recv() {
                    if !wakes.iter().any(|x| x.db == w.db) {
                        wakes.push(w);
                    }
                }
                for w in wakes {
                    let app = app.clone();
                    let _ = tauri::async_runtime::spawn_blocking(move || work(&app, &w)).await;
                }
            }
        });
        Self { tx }
    }

    pub fn kick(&self, db: PathBuf, user: String) {
        let _ = self.tx.send(Wake { db, user });
    }
}

fn work<R: Runtime>(app: &AppHandle<R>, wake: &Wake) {
    let tell = |status: Status| {
        let _ = app.emit(
            "wiki-merge-progress",
            ProgressEvent {
                user: wake.user.clone(),
                status,
            },
        );
    };
    let Ok(conn) = db::open(&wake.db) else { return };
    let model = match llm::load(&conn) {
        Ok(Some(c)) => llm::OpenAi::new(c),
        _ => {
            for novel_id in hold_all(&conn, &AiError::NotSet.to_string()).unwrap_or_default() {
                if let Ok(c) = counts(&conn, novel_id) {
                    tell(c);
                }
            }
            return;
        }
    };
    drop(conn);
    run(&wake.db, &model, &tell);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::book;
    use crate::library::wiki::{self, EntryInput};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

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

    /// Fails the first `drop` requests as a lost connection, then answers `reply`.
    struct Flaky {
        drop: AtomicUsize,
        reply: String,
        sent: Mutex<usize>,
    }

    impl Chat for Flaky {
        fn chat(&self, _system: &str, _user: &str) -> Result<String, AiError> {
            *self.sent.lock().unwrap() += 1;
            if self.drop.load(Ordering::SeqCst) > 0 {
                self.drop.fetch_sub(1, Ordering::SeqCst);
                return Err(AiError::Auth("down".into()));
            }
            Ok(self.reply.clone())
        }
        fn model(&self) -> &str {
            "flaky"
        }
    }

    #[test]
    fn a_run_survives_a_lost_connection_and_a_restart_and_keeps_its_answers() {
        let (conn, novel) = book("merge-queue");
        let path = conn.path().unwrap().to_string();
        let path = Path::new(&path);
        let a = add(&conn, novel, "Seol-Hwi", "Tuyết Huy");
        let b = add(&conn, novel, "Seol Hwi", "Tuyết Hui");
        assert_eq!(start(&conn, novel, false).unwrap(), 1);
        // The LLM refuses: the block waits with the reason, nothing is lost.
        let down = Flaky {
            drop: AtomicUsize::new(1),
            reply: String::new(),
            sent: Mutex::new(0),
        };
        let told = Mutex::new(Vec::new());
        run(path, &down, &|s| told.lock().unwrap().push(s));
        let c = counts(&conn, novel).unwrap();
        assert_eq!((c.paused, c.done), (1, 0));
        assert!(c.error.is_some());
        assert_eq!(told.lock().unwrap().len(), 1);
        // Asking again carries on the same run instead of starting over.
        assert_eq!(start(&conn, novel, false).unwrap(), 1);
        // As if the app closed while it was being asked.
        conn.execute("update wiki_merge_blocks set status = 'running'", [])
            .unwrap();
        assert_eq!(recover(&conn).unwrap(), 1);
        let up = Flaky {
            drop: AtomicUsize::new(0),
            reply:
                r#"{"groups":[{"ids":[1,2],"keep":1,"name":"Tuyết Huy","why":"cùng một người"}]}"#
                    .into(),
            sent: Mutex::new(0),
        };
        run(path, &up, &|_| {});
        let s = status(&conn, novel).unwrap();
        assert_eq!((s.total, s.done), (1, 1));
        assert_eq!(s.groups.len(), 1, "{s:?}");
        let g = &s.groups[0];
        let mut uids = vec![g.into.uid.clone(), g.members[0].uid.clone()];
        uids.sort();
        let mut want = vec![a.clone(), b.clone()];
        want.sort();
        assert_eq!(uids, want);
        // Once merged, the group is gone from the kept answers.
        wiki::merge(&conn, novel, &b, &a, true).unwrap();
        assert!(status(&conn, novel).unwrap().groups.is_empty());
        // Clearing ends the run.
        control(&conn, novel, "clear").unwrap();
        assert_eq!(counts(&conn, novel).unwrap().total, 0);
    }
}
