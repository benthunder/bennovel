//! The AI translation queue: "translate the whole book" (or "from this chapter on")
//! puts one row per chapter in `ai_jobs`, and one tokio task works through them, a
//! chapter at a time, with the graph in translate.rs. Rows stay in the library file,
//! so a queue cut short by closing the app carries on the next time it starts.
//!
//! Events for the app: `ai-progress` after each chapter, `ai-paused` when the LLM
//! can't be used (no settings, key turned down, no connection…), `ai-done` once the
//! last chapter of a request is through, which also shows a system notification.

use super::db;
use super::llm::{self, AiError};
use super::translate::{self, Request};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::mpsc;

/// One chapter to do.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: i64,
    pub novel_id: i64,
    pub chapter_id: i64,
    pub lang: String,
    pub style: String,
    pub glossary: bool,
}

impl Job {
    fn request(&self) -> Request {
        Request {
            novel_id: self.novel_id,
            chapter_id: self.chapter_id,
            lang: self.lang.clone(),
            style: self.style.clone(),
            glossary: self.glossary,
        }
    }
}

/// Queues the chapters from `from_index` (0-based) to the end that have no AI text in
/// `lang` and `style` yet; returns how many were added. Chapters that failed before
/// are tried again.
pub fn enqueue(
    conn: &Connection,
    novel_id: i64,
    from_index: usize,
    lang: &str,
    style: &str,
    glossary: bool,
) -> rusqlite::Result<usize> {
    conn.execute(
        "delete from ai_jobs where novel_id = ?1 and lang = ?2 and style = ?3 and status in ('done', 'failed')",
        params![novel_id, lang, style],
    )?;
    conn.execute(
        "insert or ignore into ai_jobs (novel_id, chapter_id, lang, style, glossary, created_at)
         select c.novel_id, c.id, ?3, ?4, ?5, ?6 from chapters c
         where c.novel_id = ?1
           and c.number >= (select number from chapters where novel_id = ?1 order by number limit 1 offset ?2)
           and exists (select 1 from chapter_translations t where t.chapter_id = c.id and t.source <> 'ai'
                       and length(trim(t.content)) > 0)
           and not exists (select 1 from chapter_translations t where t.chapter_id = c.id
                           and t.lang = ?3 and t.style = ?4 and t.source = 'ai')",
        params![novel_id, from_index as i64, lang, style, glossary, db::now_ms()],
    )
}

/// Marks the next queued chapter running and returns it: requests in the order they
/// came, chapters in reading order.
pub fn claim(conn: &Connection) -> rusqlite::Result<Option<Job>> {
    conn.query_row(
        "update ai_jobs set status = 'running' where id = (
           select j.id from ai_jobs j join chapters c on c.id = j.chapter_id
           where j.status = 'queued' order by j.created_at, c.number limit 1)
         returning id, novel_id, chapter_id, lang, style, glossary",
        [],
        |r| {
            Ok(Job {
                id: r.get(0)?,
                novel_id: r.get(1)?,
                chapter_id: r.get(2)?,
                lang: r.get(3)?,
                style: r.get(4)?,
                glossary: r.get(5)?,
            })
        },
    )
    .optional()
}

pub fn finish(conn: &Connection, id: i64, error: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "update ai_jobs set status = ?2, error = ?3 where id = ?1",
        params![id, if error.is_some() { "failed" } else { "done" }, error],
    )?;
    Ok(())
}

/// Puts a chapter back in the queue (the LLM can't be used right now).
fn requeue(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("update ai_jobs set status = 'queued' where id = ?1", [id])?;
    Ok(())
}

/// Chapters left running when the app closed go back in the queue.
pub fn recover(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "update ai_jobs set status = 'queued' where status = 'running'",
        [],
    )
}

/// Stops a book's queue; the chapter being translated still finishes.
pub fn cancel(conn: &Connection, novel_id: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "delete from ai_jobs where novel_id = ?1 and status = 'queued'",
        [novel_id],
    )
}

/// One request's state: per book, language and style.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub novel_id: i64,
    pub lang: String,
    pub style: String,
    pub total: i64,
    pub done: i64,
    pub failed: i64,
    pub queued: i64,
    pub running: i64,
}

pub fn progress(conn: &Connection, novel_id: i64) -> rusqlite::Result<Vec<Progress>> {
    let mut stmt = conn.prepare(
        "select novel_id, lang, style, count(*), sum(status = 'done'), sum(status = 'failed'),
           sum(status = 'queued'), sum(status = 'running')
         from ai_jobs where novel_id = ?1 group by lang, style order by min(created_at) desc",
    )?;
    let rows = stmt.query_map([novel_id], |r| {
        Ok(Progress {
            novel_id: r.get(0)?,
            lang: r.get(1)?,
            style: r.get(2)?,
            total: r.get(3)?,
            done: r.get(4)?,
            failed: r.get(5)?,
            queued: r.get(6)?,
            running: r.get(7)?,
        })
    })?;
    rows.collect()
}

fn progress_of(conn: &Connection, job: &Job) -> rusqlite::Result<Option<Progress>> {
    Ok(progress(conn, job.novel_id)?
        .into_iter()
        .find(|p| p.lang == job.lang && p.style == job.style))
}

/// Errors that stop the whole queue instead of one chapter: they would fail every
/// chapter the same way.
fn pauses(e: &AiError) -> bool {
    matches!(
        e,
        AiError::NotSet
            | AiError::Auth(_)
            | AiError::Network(_)
            | AiError::Http(402 | 404 | 429 | 500..=599, _)
    )
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    user: String,
    #[serde(flatten)]
    progress: Progress,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoneEvent {
    user: String,
    title: String,
    #[serde(flatten)]
    progress: Progress,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PausedEvent {
    user: String,
    novel_id: i64,
    error: String,
}

/// What one turn of the worker did.
enum Step {
    Idle,
    Did(Progress, Option<String>),
    Paused(i64, AiError),
}

struct Wake {
    db: PathBuf,
    user: String,
}

/// The queue's handle, kept in the app's state.
pub struct Queue {
    tx: mpsc::UnboundedSender<Wake>,
    /// Language of the notification texts, per library file (the app's UI language).
    ui_lang: Mutex<HashMap<PathBuf, String>>,
}

impl Queue {
    /// Starts the worker: a tokio task that empties a library's queue each time it is
    /// woken, one chapter after another.
    pub fn start<R: Runtime>(app: AppHandle<R>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Wake>();
        tauri::async_runtime::spawn(async move {
            while let Some(wake) = rx.recv().await {
                work(&app, &wake).await;
            }
        });
        Self {
            tx,
            ui_lang: Mutex::new(HashMap::new()),
        }
    }

    /// Wakes the worker for this library file.
    pub fn kick(&self, db: PathBuf, user: String, ui_lang: Option<String>) {
        if let Some(l) = ui_lang {
            self.ui_lang.lock().unwrap().insert(db.clone(), l);
        }
        let _ = self.tx.send(Wake { db, user });
    }

    fn ui_lang(&self, db: &PathBuf) -> String {
        self.ui_lang
            .lock()
            .unwrap()
            .get(db)
            .cloned()
            .unwrap_or_else(|| "vi".into())
    }
}

async fn work<R: Runtime>(app: &AppHandle<R>, wake: &Wake) {
    loop {
        let db_file = wake.db.clone();
        let step = tauri::async_runtime::spawn_blocking(move || step(&db_file))
            .await
            .unwrap_or_else(|e| Err(AiError::Other(e.to_string())));
        match step {
            Ok(Step::Idle) => return,
            Ok(Step::Did(p, finished)) => {
                let _ = app.emit(
                    "ai-progress",
                    ProgressEvent {
                        user: wake.user.clone(),
                        progress: p.clone(),
                    },
                );
                if let Some(title) = finished {
                    notify(app, wake, &title, &p);
                    let _ = app.emit(
                        "ai-done",
                        DoneEvent {
                            user: wake.user.clone(),
                            title,
                            progress: p,
                        },
                    );
                }
            }
            Ok(Step::Paused(novel_id, e)) => {
                let _ = app.emit(
                    "ai-paused",
                    PausedEvent {
                        user: wake.user.clone(),
                        novel_id,
                        error: e.to_string(),
                    },
                );
                return;
            }
            Err(e) => {
                eprintln!("AI queue: {e}");
                return;
            }
        }
    }
}

/// Translates the next queued chapter of a library file.
fn step(db_file: &std::path::Path) -> Result<Step, AiError> {
    let conn = db::open(db_file)?;
    let Some(job) = claim(&conn)? else {
        return Ok(Step::Idle);
    };
    let Some(config) = llm::load(&conn)? else {
        requeue(&conn, job.id)?;
        return Ok(Step::Paused(job.novel_id, AiError::NotSet));
    };
    let model = std::sync::Arc::new(llm::OpenAi::new(config));
    // Already translated on its own from the reader meanwhile.
    let done = translate::get(&conn, job.chapter_id, &job.lang, &job.style)?.is_some();
    match if done {
        Ok(())
    } else {
        translate::run(&conn, model, job.request()).map(|_| ())
    } {
        Ok(_) => finish(&conn, job.id, None)?,
        Err(e) if pauses(&e) => {
            requeue(&conn, job.id)?;
            return Ok(Step::Paused(job.novel_id, e));
        }
        Err(e) => finish(&conn, job.id, Some(&e.to_string()))?,
    }
    let p = progress_of(&conn, &job)?.ok_or(AiError::Other("job vanished".into()))?;
    // The last chapter of this request: the book's title for the notification.
    let finished = if p.queued + p.running == 0 {
        conn.query_row(
            "select t.title from novels n join novel_translations t
               on t.novel_id = n.id and t.lang = n.original_lang where n.id = ?1",
            [job.novel_id],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    Ok(Step::Did(p, finished))
}

/// "Translation done" as a system notification, in the app's language.
fn notify<R: Runtime>(app: &AppHandle<R>, wake: &Wake, title: &str, p: &Progress) {
    use tauri::Manager;
    use tauri_plugin_notification::NotificationExt;
    let vi = app
        .try_state::<Queue>()
        .map(|q| q.ui_lang(&wake.db) == "vi")
        .unwrap_or(true);
    let lang = match (p.lang.as_str(), vi) {
        ("vi", true) => "tiếng Việt",
        ("en", true) => "tiếng Anh",
        ("vi", false) => "Vietnamese",
        _ => "English",
    };
    let (head, body) = if vi {
        (
            format!("Đã dịch xong: {title}"),
            if p.failed > 0 {
                format!("{} chương sang {lang}, {} chương lỗi.", p.done, p.failed)
            } else {
                format!("{} chương sang {lang}.", p.done)
            },
        )
    } else {
        (
            format!("Translation done: {title}"),
            if p.failed > 0 {
                format!("{} chapters into {lang}, {} failed.", p.done, p.failed)
            } else {
                format!("{} chapters into {lang}.", p.done)
            },
        )
    };
    if let Err(e) = app.notification().builder().title(head).body(body).show() {
        eprintln!("notification: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::translate::tests::{book, Fake};

    #[test]
    fn queues_works_through_and_skips_done_chapters() {
        let (conn, novel) = book("queue");
        let llm = std::sync::Arc::new(Fake::new("en"));
        // Chapter 2 already has its translation: only chapter 1 is queued.
        translate::run(
            &conn,
            llm.clone(),
            Request {
                novel_id: novel,
                chapter_id: 11,
                lang: "vi".into(),
                style: "natural".into(),
                glossary: false,
            },
        )
        .unwrap();
        assert_eq!(enqueue(&conn, novel, 0, "vi", "natural", true).unwrap(), 1);
        // Asking again adds nothing while it waits.
        assert_eq!(enqueue(&conn, novel, 0, "vi", "natural", true).unwrap(), 0);
        let job = claim(&conn).unwrap().unwrap();
        assert_eq!(job.chapter_id, 10);
        assert!(claim(&conn).unwrap().is_none());
        translate::run(&conn, llm.clone(), job.request()).unwrap();
        finish(&conn, job.id, None).unwrap();
        let p = progress(&conn, novel).unwrap();
        assert_eq!((p[0].total, p[0].done, p[0].queued), (1, 1, 0));
        // Everything is translated now.
        assert_eq!(enqueue(&conn, novel, 0, "vi", "natural", true).unwrap(), 0);
        // From the second chapter on, another style.
        assert_eq!(enqueue(&conn, novel, 1, "vi", "casual", true).unwrap(), 1);
        assert_eq!(claim(&conn).unwrap().unwrap().chapter_id, 11);
        assert_eq!(recover(&conn).unwrap(), 1);
        assert_eq!(cancel(&conn, novel).unwrap(), 1);
    }

    #[test]
    fn some_errors_pause_the_queue() {
        assert!(pauses(&AiError::Auth("x".into())));
        assert!(pauses(&AiError::Http(429, "slow down".into())));
        assert!(!pauses(&AiError::Http(400, "too long".into())));
        assert!(!pauses(&AiError::Empty));
    }
}
