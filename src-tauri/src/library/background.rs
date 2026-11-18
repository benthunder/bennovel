//! Android: keeps the AI translation running with the screen off, with its progress on
//! a notification, the way a video keeps playing. While the queue (queue.rs) or a
//! chapter from the reader is being translated, the background-work plugin runs a
//! foreground service whose notification says which book, which chapter and node, and
//! how far the request is; once nothing is left, the service stops.
//!
//! The queue and the reader's translation call `changed` whenever they move. A thread
//! of its own reads `Queue::running`, builds the notification and hands it to the
//! plugin, at most a few times a second (Android drops faster updates anyway). Between
//! two chapters the worker is idle for a moment, so the service stops only once nothing
//! ran for a few seconds. On other systems `changed` does nothing.
//!
//! Swiping the app off the recent apps (or closing it with Back) ends its window; while
//! something is being translated the app then stays without one (`keep_for_work`), the
//! way a download or music goes on, and ends once the work is done.

// The notification is built on Android only; tests build it everywhere.
#![cfg_attr(not(target_os = "android"), allow(dead_code, unused_imports))]

use super::queue::{self, Progress, Queue, Running};
use super::translate::Stage;
use std::path::PathBuf;
#[cfg(target_os = "android")]
use std::sync::{mpsc, OnceLock};
pub use tauri_plugin_background_work::Notice;

#[cfg(target_os = "android")]
static WAKE: OnceLock<mpsc::Sender<()>> = OnceLock::new();

/// The app's window went away (swiped off the recent apps, closed with Back) while a
/// translation ran: the process stays for the work, and ends once it is done.
#[cfg(target_os = "android")]
static HEADLESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether something is being translated now (the queue or a chapter from the reader).
pub fn busy<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use tauri::Manager;
    app.try_state::<Queue>()
        .is_some_and(|q| !q.running().is_empty())
}

/// The last window is closing. Returns whether the app should stay for the work under
/// way: it then goes on with the notification and no window, like a download, and the
/// app ends once it is done. Opened again meanwhile, the app starts afresh and its
/// queue carries on from the parts it has (the plugin's Relaunch).
#[cfg(target_os = "android")]
pub fn keep_for_work<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use std::sync::atomic::Ordering;
    use tauri::Manager;
    use tauri_plugin_background_work::BackgroundWork;
    if !busy(app) {
        return false;
    }
    let Some(work) = app.try_state::<BackgroundWork<R>>() else {
        return false;
    };
    if let Err(e) = work.set_headless(true) {
        eprintln!("background work: {e}");
        return false;
    }
    HEADLESS.store(true, Ordering::SeqCst);
    changed();
    true
}

/// The work is over with no window left: the app ends (see `keep_for_work`).
#[cfg(target_os = "android")]
fn end_if_headless() {
    if HEADLESS.load(std::sync::atomic::Ordering::SeqCst) {
        std::process::exit(0);
    }
}

/// Something being translated moved, started or ended.
pub fn changed() {
    #[cfg(target_os = "android")]
    if let Some(tx) = WAKE.get() {
        let _ = tx.send(());
    }
}

/// Starts the thread that keeps the notification in step (Android only).
#[cfg(target_os = "android")]
pub fn start<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;
    use tauri::Manager;
    use tauri_plugin_background_work::BackgroundWork;
    /// Least time between two updates of the notification.
    const THROTTLE: Duration = Duration::from_millis(700);
    /// How long nothing must run before the service stops.
    const HIDE_AFTER: Duration = Duration::from_secs(4);

    let (tx, rx) = mpsc::channel::<()>();
    if WAKE.set(tx).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let mut cache = Cache::default();
        let mut shown: Option<Notice> = None;
        let current = |cache: &mut Cache| -> Option<Notice> {
            let q = app.try_state::<Queue>()?;
            let running = q.running();
            let first = running.first()?;
            let vi = q.ui_lang(&first.db) == "vi";
            Some(cache.notice(first, vi))
        };
        while rx.recv().is_ok() {
            loop {
                while rx.try_recv().is_ok() {}
                let Some(work) = app.try_state::<BackgroundWork<R>>() else {
                    return;
                };
                match current(&mut cache) {
                    Some(n) => {
                        if shown.as_ref() != Some(&n) {
                            if let Err(e) = work.show(&n) {
                                eprintln!("background notification: {e}");
                            }
                            shown = Some(n);
                        }
                    }
                    None if shown.is_some() => match rx.recv_timeout(HIDE_AFTER) {
                        Ok(()) => continue,
                        Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => {
                            if current(&mut cache).is_none() {
                                if let Err(e) = work.hide() {
                                    eprintln!("background notification: {e}");
                                }
                                shown = None;
                                cache = Cache::default();
                                end_if_headless();
                            } else {
                                continue;
                            }
                        }
                    },
                    None => match rx.recv_timeout(HIDE_AFTER) {
                        // Nothing shown and nothing running: with no window, the app ends.
                        Ok(()) => continue,
                        Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => {
                            if current(&mut cache).is_none() {
                                end_if_headless();
                            } else {
                                continue;
                            }
                        }
                    },
                }
                std::thread::sleep(THROTTLE);
                if rx.try_recv().is_err() {
                    break;
                }
            }
        }
    });
}

/// The request's progress and the book's title, read from the library file once per
/// chapter instead of on every part.
#[derive(Default)]
struct Cache {
    key: Option<(PathBuf, i64, String, String, i64)>,
    title: String,
    progress: Option<Progress>,
}

impl Cache {
    fn notice(&mut self, r: &Running, vi: bool) -> Notice {
        if let Some(title) = &r.manual_title {
            return notice(r, title, None, vi);
        }
        let l = &r.live;
        let key = (
            r.db.clone(),
            l.novel_id,
            l.lang.clone(),
            l.style.clone(),
            l.index,
        );
        if self.key.as_ref() != Some(&key) {
            let (title, progress) = read(r).unwrap_or_default();
            (self.key, self.title, self.progress) = (Some(key), title, progress);
        }
        notice(r, &self.title, self.progress.as_ref(), vi)
    }
}

fn read(r: &Running) -> Option<(String, Option<Progress>)> {
    let conn = super::db::open(&r.db).ok()?;
    let l = &r.live;
    let title = queue::book_title(&conn, l.novel_id).ok()??;
    let progress = queue::progress(&conn, l.novel_id)
        .ok()?
        .into_iter()
        .find(|p| p.lang == l.lang && p.style == l.style);
    Some((title, progress))
}

fn stage_name(stage: Stage, vi: bool) -> &'static str {
    match (stage, vi) {
        (Stage::Detect, true) => "Nhận diện",
        (Stage::Recall, true) => "Tra wiki",
        (Stage::Translate, true) => "Dịch",
        (Stage::Rewrite, true) => "Viết lại",
        (Stage::Save, true) => "Lưu",
        (Stage::Learn, true) => "Học wiki",
        (Stage::Detect, false) => "Detect",
        (Stage::Recall, false) => "Wiki lookup",
        (Stage::Translate, false) => "Translate",
        (Stage::Rewrite, false) => "Rewrite",
        (Stage::Save, false) => "Save",
        (Stage::Learn, false) => "Learn wiki",
    }
}

/// The notification for a chapter being translated: the book, the chapter with its
/// node and parts, and, for the queue, the request's chapters as the bar. A chapter
/// from the reader shows its parts as the bar.
fn notice(r: &Running, title: &str, progress: Option<&Progress>, vi: bool) -> Notice {
    let l = &r.live;
    let mut text = format!(
        "{} {} · {}",
        if vi { "Chương" } else { "Chapter" },
        l.index + 1,
        stage_name(l.stage, vi)
    );
    if l.parts_total > 0 {
        text.push_str(&format!(
            " {}/{} {}",
            l.parts_done,
            l.parts_total,
            if vi { "phần" } else { "parts" }
        ));
    }
    let (sub_text, done, total) = match progress {
        Some(p) => (
            format!(
                "{}/{} {}",
                p.done + p.failed,
                p.total,
                if vi { "chương" } else { "chapters" }
            ),
            (p.done + p.failed).max(0) as u32,
            p.total.max(0) as u32,
        ),
        None => (String::new(), l.parts_done as u32, l.parts_total as u32),
    };
    Notice {
        title: if title.is_empty() {
            (if vi { "Đang dịch" } else { "Translating" }).into()
        } else if vi {
            format!("Đang dịch: {title}")
        } else {
            format!("Translating: {title}")
        },
        text,
        sub_text,
        done,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::queue::Live;

    fn running(manual: Option<&str>) -> Running {
        Running {
            db: PathBuf::from("x.db"),
            live: Live {
                novel_id: 1,
                lang: "vi".into(),
                style: "plain".into(),
                index: 4,
                stage: Stage::Translate,
                parts_done: 3,
                parts_total: 8,
            },
            manual_title: manual.map(String::from),
        }
    }

    fn progress(done: i64, failed: i64, total: i64) -> Progress {
        Progress {
            novel_id: 1,
            lang: "vi".into(),
            style: "plain".into(),
            total,
            done,
            failed,
            queued: total - done - failed - 1,
            running: 1,
            paused: 0,
            error: None,
        }
    }

    #[test]
    fn the_queue_shows_its_chapters_as_the_bar() {
        let n = notice(
            &running(None),
            "Phàm Nhân",
            Some(&progress(10, 2, 120)),
            true,
        );
        assert_eq!(n.title, "Đang dịch: Phàm Nhân");
        assert_eq!(n.text, "Chương 5 · Dịch 3/8 phần");
        assert_eq!(n.sub_text, "12/120 chương");
        assert_eq!((n.done, n.total), (12, 120));
    }

    #[test]
    fn a_chapter_from_the_reader_shows_its_parts_as_the_bar() {
        let mut r = running(Some("Book"));
        r.live.stage = Stage::Detect;
        (r.live.parts_done, r.live.parts_total) = (0, 0);
        let n = Cache::default().notice(&r, false);
        assert_eq!(n.title, "Translating: Book");
        assert_eq!(n.text, "Chapter 5 · Detect");
        assert_eq!(n.sub_text, "");
        // No parts yet: an indeterminate bar.
        assert_eq!((n.done, n.total), (0, 0));
    }
}
