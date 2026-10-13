//! Where a library syncs to. Each place is a `Remote` (a strategy): it only knows how to
//! fetch the one synced file and how to store it if nobody changed it since. The sync
//! itself (download, merge, upload, start over when another device got in first) is
//! `sync` below, the same for every place, and `open` picks the place for a library.
//! Adding a place (or letting an admin panel choose one) means a new `Remote` and a line
//! in `open`; nothing else changes.
//!
//! Files go through disk, not memory: the snapshot is packed into a file, uploaded from
//! it, and a download is saved to a file before it is merged.
//!
//! The places today:
//! - the BenNovel server (server.rs), for signed-in accounts: a Supabase Edge Function
//!   that holds the storage keys and hands out signed links, so the app never has them;
//! - S3-compatible storage the user runs (s3.rs), with keys they entered on this device.

use super::{db, s3, server, sync};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const ATTEMPTS: usize = 3;
/// Refuse to download a file larger than this.
pub const MAX_DOWNLOAD: u64 = 4 << 30;

/// The synced file, as fetched.
#[derive(Debug, PartialEq)]
pub enum Fetched {
    /// Still the one with the ETag this device knows.
    Unchanged,
    /// Nothing synced yet.
    Missing,
    /// Downloaded into the file given to `fetch`.
    File { etag: Option<String> },
}

/// What came of storing the file.
#[derive(Debug, PartialEq)]
pub enum Stored {
    /// Stored; its new ETag, if the place gives one.
    Saved(Option<String>),
    /// Another device stored a newer file since this one fetched.
    Conflict,
}

/// Where a sync is, for the app to show.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum Step {
    /// Bytes downloaded so far, of `total` (0 when not known yet).
    Download {
        done: u64,
        total: u64,
    },
    Merge,
    /// Packing the library into the file to upload.
    Pack,
    Upload {
        done: u64,
        total: u64,
    },
}

/// Told each step of a sync, and how far a transfer has come.
pub type Progress<'a> = dyn Fn(Step) + 'a;

pub trait Remote {
    /// Names the file synced with, so switching places starts over.
    fn target(&self) -> String;
    /// Downloads the file into `dest`, unless its ETag is still `known`.
    fn fetch(
        &self,
        known: Option<&str>,
        dest: &Path,
        progress: &Progress,
    ) -> Result<Fetched, RemoteError>;
    /// Uploads the file at `file`, only if the stored one still has ETag `current`
    /// (`None`: only if there is none).
    fn store(
        &self,
        file: &Path,
        current: Option<&str>,
        progress: &Progress,
    ) -> Result<Stored, RemoteError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    /// The place answered with an error status (and its error code, if any).
    #[error("remote {status} {code}")]
    Status { status: u16, code: String },
    #[error("{0}")]
    Network(String),
    /// Settings missing or wrong; the word names which.
    #[error("{0}")]
    Config(&'static str),
    /// The account's session was refused: sign in again.
    #[error("sign-in")]
    SignIn,
    #[error(transparent)]
    Sync(#[from] sync::SyncError),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Other devices kept syncing in between every attempt.
    #[error("sync-busy")]
    Busy,
}

/// The signed-in account, for the BenNovel server.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// The sync function's URL, `https://<project>.supabase.co/functions/v1/sync`.
    pub url: String,
    /// The project's publishable key.
    pub api_key: String,
    /// The account's current access token.
    pub token: String,
}

/// Where the library at `library` syncs: its own S3 storage when one is set, else the
/// BenNovel server for the signed-in `account`.
pub fn open(library: &Path, account: Option<Account>) -> Result<Box<dyn Remote>, RemoteError> {
    if let Some(config) = s3::load_config(&s3::config_path(library)) {
        return Ok(Box::new(s3::S3::new(config)));
    }
    match account {
        Some(account) => Ok(Box::new(server::Server::new(account))),
        None => Err(RemoteError::Config("sync-not-set")),
    }
}

/// An HTTP client for the places, through the system proxy when one is set.
pub fn agent() -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(120))
        .timeout_write(Duration::from_secs(300));
    #[cfg(test)]
    let builder = s3::tests::with_test_ca(builder);
    builder.build()
}

// Kept under their first names, so a library that synced with its own storage before
// carries on where it was.
const ETAG: &str = "s3_etag";
const TARGET: &str = "s3_target";

/// A reader that reports how many bytes went through it, every 256 KiB or so.
pub struct Counted<'a, R> {
    inner: R,
    done: u64,
    reported: u64,
    report: &'a dyn Fn(u64),
}

impl<'a, R: Read> Counted<'a, R> {
    pub fn new(inner: R, start: u64, report: &'a dyn Fn(u64)) -> Self {
        Self {
            inner,
            done: start,
            reported: start,
            report,
        }
    }
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        if n == 0 || self.done - self.reported >= 256 * 1024 {
            self.reported = self.done;
            (self.report)(self.done);
        }
        Ok(n)
    }
}

/// Saves a download into `dest`, telling `progress` how far it has come.
pub fn save_download(
    res: ureq::Response,
    dest: &Path,
    progress: &Progress,
) -> Result<(), RemoteError> {
    let total = res
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let report = |done| progress(Step::Download { done, total });
    report(0);
    let mut body = Counted::new(res.into_reader().take(MAX_DOWNLOAD), 0, &report);
    let mut file = std::io::BufWriter::new(std::fs::File::create(dest)?);
    std::io::copy(&mut body, &mut file).map_err(|e| RemoteError::Network(e.to_string()))?;
    std::io::Write::flush(&mut file)?;
    Ok(())
}

/// Syncs the library at `path` with `remote`. Returns the ids of local books removed
/// because they were deleted on another device.
pub fn sync(
    path: &Path,
    remote: &dyn Remote,
    progress: &Progress,
) -> Result<Vec<i64>, RemoteError> {
    let incoming = path.with_extension("sync-in.gz");
    let outgoing = path.with_extension("sync-out.gz");
    let result = sync_files(path, remote, progress, &incoming, &outgoing);
    let _ = std::fs::remove_file(&incoming);
    let _ = std::fs::remove_file(&outgoing);
    result
}

fn sync_files(
    path: &Path,
    remote: &dyn Remote,
    progress: &Progress,
    incoming: &Path,
    outgoing: &Path,
) -> Result<Vec<i64>, RemoteError> {
    let target = remote.target();
    let mut gone = Vec::new();
    for _ in 0..ATTEMPTS {
        let mut conn = db::open(path)?;
        let mut known = db::get_state(&conn, ETAG)?
            .filter(|_| db::get_state(&conn, TARGET).ok().flatten().as_deref() == Some(&target));

        // Take in what other devices synced since this one last did.
        let current = match remote.fetch(known.as_deref(), incoming, progress)? {
            Fetched::File { etag } => {
                progress(Step::Merge);
                let file = std::io::BufReader::new(std::fs::File::open(incoming)?);
                gone.extend(sync::merge_from(
                    &mut conn,
                    file,
                    &path.with_extension("sync-in"),
                )?);
                let _ = std::fs::remove_file(incoming);
                known = etag.clone();
                etag
            }
            Fetched::Unchanged => known.clone(),
            Fetched::Missing => None,
        };
        if let Some(etag) = &known {
            db::set_state(&conn, TARGET, &target)?;
            db::set_state(&conn, ETAG, etag)?;
        }

        progress(Step::Pack);
        sync::export_to(&conn, &path.with_extension("sync-out"), outgoing)?;
        match remote.store(outgoing, current.as_deref(), progress)? {
            Stored::Saved(etag) => {
                db::set_state(&conn, TARGET, &target)?;
                match etag {
                    Some(etag) => db::set_state(&conn, ETAG, &etag)?,
                    None => {
                        conn.execute("delete from sync_state where key = ?1", [ETAG])?;
                    }
                }
                db::set_state(&conn, sync::SYNCED_AT, &db::now_ms().to_string())?;
                return Ok(gone);
            }
            // Another device stored since the download: merge theirs and try again.
            Stored::Conflict => continue,
        }
    }
    Err(RemoteError::Busy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::rules::{self, Kind, Rule};
    use std::cell::RefCell;

    /// A place in memory, numbering its ETags; `race` stores a file from "another
    /// device" right before the next store.
    #[derive(Default)]
    struct Memory {
        file: RefCell<Option<(Vec<u8>, String)>>,
        n: RefCell<u32>,
        race: RefCell<Option<Vec<u8>>>,
        stores: RefCell<u32>,
    }

    impl Memory {
        fn put(&self, body: Vec<u8>) -> String {
            *self.n.borrow_mut() += 1;
            let etag = format!("\"{}\"", self.n.borrow());
            *self.file.borrow_mut() = Some((body, etag.clone()));
            etag
        }
    }

    impl Remote for &Memory {
        fn target(&self) -> String {
            "memory".into()
        }
        fn fetch(
            &self,
            known: Option<&str>,
            dest: &Path,
            _: &Progress,
        ) -> Result<Fetched, RemoteError> {
            Ok(match &*self.file.borrow() {
                None => Fetched::Missing,
                Some((_, etag)) if Some(etag.as_str()) == known => Fetched::Unchanged,
                Some((body, etag)) => {
                    std::fs::write(dest, body)?;
                    Fetched::File {
                        etag: Some(etag.clone()),
                    }
                }
            })
        }
        fn store(
            &self,
            file: &Path,
            current: Option<&str>,
            progress: &Progress,
        ) -> Result<Stored, RemoteError> {
            let body = std::fs::read(file)?;
            progress(Step::Upload {
                done: body.len() as u64,
                total: body.len() as u64,
            });
            *self.stores.borrow_mut() += 1;
            if let Some(other) = self.race.borrow_mut().take() {
                self.put(other);
            }
            let now = self.file.borrow().as_ref().map(|(_, e)| e.clone());
            if now.as_deref() != current {
                return Ok(Stored::Conflict);
            }
            Ok(Stored::Saved(Some(self.put(body))))
        }
    }

    fn device(name: &str) -> std::path::PathBuf {
        let path =
            crate::book::test_dir().join(format!("remote-{name}-{}.sqlite3", std::process::id()));
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
        db::open(&path).unwrap();
        path
    }

    fn rule(find: &str) -> Vec<Rule> {
        vec![Rule {
            find: find.into(),
            replace: "x".into(),
        }]
    }

    fn rules_of(path: &Path, id: i64) -> Option<Vec<Rule>> {
        rules::get(&db::open(path).unwrap(), Kind::Online, id, "en").unwrap()
    }

    #[test]
    fn devices_share_their_changes_through_any_place() {
        let place = Memory::default();
        let (a, b) = (device("a"), device("b"));
        rules::set(&db::open(&a).unwrap(), Kind::Online, 1, "en", &rule("one")).unwrap();
        rules::set(&db::open(&b).unwrap(), Kind::Online, 2, "en", &rule("two")).unwrap();

        let steps = RefCell::new(Vec::new());
        let progress = |s: Step| steps.borrow_mut().push(s);
        sync(&a, &&place, &progress).unwrap(); // first upload
        sync(&b, &&place, &|_| ()).unwrap(); // takes A's, adds its own
        sync(&a, &&place, &progress).unwrap(); // takes B's
        for path in [&a, &b] {
            assert_eq!(rules_of(path, 1), Some(rule("one")));
            assert_eq!(rules_of(path, 2), Some(rule("two")));
        }
        assert_eq!(*place.stores.borrow(), 3);
        let names: Vec<_> = steps
            .borrow()
            .iter()
            .map(|s| match s {
                Step::Download { .. } => "download",
                Step::Merge => "merge",
                Step::Pack => "pack",
                Step::Upload { .. } => "upload",
            })
            .collect();
        assert_eq!(names, ["pack", "upload", "merge", "pack", "upload"]);
        assert!(sync::synced_at(&db::open(&a).unwrap()).unwrap().is_some());
    }

    #[test]
    fn a_device_that_loses_the_race_merges_and_stores_again() {
        let place = Memory::default();
        let (a, b) = (device("race-a"), device("race-b"));
        rules::set(&db::open(&b).unwrap(), Kind::Online, 2, "en", &rule("two")).unwrap();
        let b_file = sync::export(&db::open(&b).unwrap(), &b.with_extension("sync-out")).unwrap();
        rules::set(&db::open(&a).unwrap(), Kind::Online, 1, "en", &rule("one")).unwrap();

        // B stores its file between A's download and A's upload.
        *place.race.borrow_mut() = Some(b_file);
        sync(&a, &&place, &|_| ()).unwrap();
        assert_eq!(*place.stores.borrow(), 2);
        assert_eq!(rules_of(&a, 2), Some(rule("two")));
        sync(&b, &&place, &|_| ()).unwrap();
        assert_eq!(rules_of(&b, 1), Some(rule("one")));
    }
}
