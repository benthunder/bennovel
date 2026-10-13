//! Where a library syncs to. Each place is a `Remote` (a strategy): it only knows how to
//! fetch the one synced file and how to store it if nobody changed it since. The sync
//! itself (download, merge, upload, start over when another device got in first) is
//! `sync` below, the same for every place, and `open` picks the place for a library.
//! Adding a place (or letting an admin panel choose one) means a new `Remote` and a line
//! in `open`; nothing else changes.
//!
//! The places today:
//! - the BenNovel server (server.rs), for signed-in accounts: a Supabase Edge Function
//!   that holds the storage keys, so the app never has them;
//! - S3-compatible storage the user runs (s3.rs), with keys they entered on this device.

use super::{db, s3, server, sync};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const ATTEMPTS: usize = 3;
/// Snapshots are read whole into memory; refuse a download larger than this.
pub const MAX_DOWNLOAD: u64 = 1 << 30;

/// The synced file, as fetched.
#[derive(Debug, PartialEq)]
pub enum Fetched {
    /// Still the one with the ETag this device knows.
    Unchanged,
    /// Nothing synced yet.
    Missing,
    File {
        body: Vec<u8>,
        etag: Option<String>,
    },
}

/// What came of storing the file.
#[derive(Debug, PartialEq)]
pub enum Stored {
    /// Stored; its new ETag, if the place gives one.
    Saved(Option<String>),
    /// Another device stored a newer file since this one fetched.
    Conflict,
}

pub trait Remote {
    /// Names the file synced with, so switching places starts over.
    fn target(&self) -> String;
    /// The file, unless its ETag is still `known`.
    fn fetch(&self, known: Option<&str>) -> Result<Fetched, RemoteError>;
    /// Stores `body` only if the file still has ETag `current` (`None`: if there is none).
    fn store(&self, body: &[u8], current: Option<&str>) -> Result<Stored, RemoteError>;
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

/// Syncs the library at `path` with `remote`. Returns the ids of local books removed
/// because they were deleted on another device.
pub fn sync(path: &Path, remote: &dyn Remote) -> Result<Vec<i64>, RemoteError> {
    let target = remote.target();
    let mut gone = Vec::new();
    for _ in 0..ATTEMPTS {
        let mut conn = db::open(path)?;
        let mut known = db::get_state(&conn, ETAG)?
            .filter(|_| db::get_state(&conn, TARGET).ok().flatten().as_deref() == Some(&target));

        // Take in what other devices synced since this one last did.
        let current = match remote.fetch(known.as_deref())? {
            Fetched::File { body, etag } => {
                gone.extend(sync::merge(
                    &mut conn,
                    &body,
                    &path.with_extension("sync-in"),
                )?);
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

        let bytes = sync::export(&conn, &path.with_extension("sync-out"))?;
        match remote.store(&bytes, current.as_deref())? {
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
        fn fetch(&self, known: Option<&str>) -> Result<Fetched, RemoteError> {
            Ok(match &*self.file.borrow() {
                None => Fetched::Missing,
                Some((_, etag)) if Some(etag.as_str()) == known => Fetched::Unchanged,
                Some((body, etag)) => Fetched::File {
                    body: body.clone(),
                    etag: Some(etag.clone()),
                },
            })
        }
        fn store(&self, body: &[u8], current: Option<&str>) -> Result<Stored, RemoteError> {
            *self.stores.borrow_mut() += 1;
            if let Some(other) = self.race.borrow_mut().take() {
                self.put(other);
            }
            let now = self.file.borrow().as_ref().map(|(_, e)| e.clone());
            if now.as_deref() != current {
                return Ok(Stored::Conflict);
            }
            Ok(Stored::Saved(Some(self.put(body.to_vec()))))
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

        sync(&a, &&place).unwrap(); // first upload
        sync(&b, &&place).unwrap(); // takes A's, adds its own
        sync(&a, &&place).unwrap(); // takes B's
        for path in [&a, &b] {
            assert_eq!(rules_of(path, 1), Some(rule("one")));
            assert_eq!(rules_of(path, 2), Some(rule("two")));
        }
        assert_eq!(*place.stores.borrow(), 3);
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
        sync(&a, &&place).unwrap();
        assert_eq!(*place.stores.borrow(), 2);
        assert_eq!(rules_of(&a, 2), Some(rule("two")));
        sync(&b, &&place).unwrap();
        assert_eq!(rules_of(&b, 1), Some(rule("one")));
    }
}
