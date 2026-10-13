//! The BenNovel server: the Supabase Edge Function `sync` (supabase/functions/sync),
//! called with the account's session. It holds the storage keys and only hands out
//! signed links for the account's file, so the file goes straight between this device
//! and the storage, uploaded in parts of `PART` bytes read from disk as they are sent.

use super::remote::{
    agent, save_download, Account, Counted, Fetched, Progress, Remote, RemoteError, Step, Stored,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Size of each uploaded part but the last (S3 needs at least 5 MiB).
const PART: u64 = 8 << 20;
/// Times a part is sent before the upload gives up.
const PART_ATTEMPTS: usize = 3;

pub struct Server {
    account: Account,
    agent: ureq::Agent,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Upload {
    upload_id: String,
    urls: Vec<String>,
}

/// Turns any answer, error statuses included, into the response; only a failed
/// connection is an error.
fn answer(result: Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, RemoteError> {
    match result {
        Ok(r) | Err(ureq::Error::Status(_, r)) => Ok(r),
        Err(ureq::Error::Transport(t)) => Err(RemoteError::Network(t.to_string())),
    }
}

/// The error a failed answer stands for: JSON `{ error }` from the function, or an
/// S3 `<Code>` from the storage.
fn fail(res: ureq::Response) -> RemoteError {
    let status = res.status();
    let text = res.into_string().unwrap_or_default();
    let code = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_string))
        .or_else(|| {
            let (_, rest) = text.split_once("<Code>")?;
            Some(rest.split_once("</Code>")?.0.to_string())
        })
        .unwrap_or_default();
    match (status, code.as_str()) {
        (401, _) => RemoteError::SignIn,
        (503, "not-configured") => RemoteError::Config("sync-server-off"),
        _ => RemoteError::Status { status, code },
    }
}

impl Server {
    pub fn new(account: Account) -> Self {
        Self {
            account,
            agent: agent(),
        }
    }

    /// Asks the function for `body`; a refusal comes back as `Err` with its status.
    fn call(&self, body: Value) -> Result<Result<Value, ureq::Response>, RemoteError> {
        let res = answer(
            self.agent
                .post(&self.account.url)
                .set("authorization", &format!("Bearer {}", self.account.token))
                .set("apikey", &self.account.api_key)
                .set("content-type", "application/json")
                .send_string(&body.to_string()),
        )?;
        if res.status() != 200 {
            return Ok(Err(res));
        }
        let text = res
            .into_string()
            .map_err(|e| RemoteError::Network(e.to_string()))?;
        serde_json::from_str(&text)
            .map(Ok)
            .map_err(|_| RemoteError::Status {
                status: 200,
                code: "bad-answer".into(),
            })
    }

    fn ask(&self, body: Value) -> Result<Value, RemoteError> {
        self.call(body)?.map_err(fail)
    }

    /// Sends bytes `start..start + len` of `file` to `url`; returns the part's ETag.
    fn send_part(
        &self,
        url: &str,
        file: &Path,
        start: u64,
        len: u64,
        report: &dyn Fn(u64),
    ) -> Result<String, RemoteError> {
        let mut last = RemoteError::Network("not sent".into());
        for _ in 0..PART_ATTEMPTS {
            let mut f = std::fs::File::open(file)?;
            f.seek(SeekFrom::Start(start))?;
            let body = Counted::new(f.take(len), start, report);
            let res = match answer(
                self.agent
                    .put(url)
                    .set("content-length", &len.to_string())
                    .send(body),
            ) {
                Ok(res) => res,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match res.status() {
                200 => {
                    return res
                        .header("etag")
                        .map(str::to_string)
                        .ok_or(RemoteError::Status {
                            status: 200,
                            code: "no-etag".into(),
                        })
                }
                // A server hiccup: try the part again.
                500..=599 => last = fail(res),
                _ => return Err(fail(res)),
            }
        }
        Err(last)
    }
}

impl Remote for Server {
    fn target(&self) -> String {
        // The server keeps one file per account; the token is not part of it.
        format!("server {}", self.account.url)
    }

    fn fetch(
        &self,
        known: Option<&str>,
        dest: &Path,
        progress: &Progress,
    ) -> Result<Fetched, RemoteError> {
        progress(Step::Download { done: 0, total: 0 });
        let link = self.ask(json!({ "op": "download" }))?;
        let url = link["url"].as_str().unwrap_or_default();
        let mut req = self.agent.get(url);
        if let Some(etag) = known {
            req = req.set("if-none-match", etag);
        }
        let res = answer(req.call())?;
        match res.status() {
            200 => {
                let etag = res.header("etag").map(str::to_string);
                save_download(res, dest, progress)?;
                Ok(Fetched::File { etag })
            }
            304 => Ok(Fetched::Unchanged),
            404 => Ok(Fetched::Missing),
            _ => Err(fail(res)),
        }
    }

    fn store(
        &self,
        file: &Path,
        current: Option<&str>,
        progress: &Progress,
    ) -> Result<Stored, RemoteError> {
        let total = std::fs::metadata(file)?.len();
        let parts = total.div_ceil(PART).max(1);
        let report = |done| progress(Step::Upload { done, total });
        report(0);
        let upload: Upload =
            serde_json::from_value(self.ask(json!({ "op": "upload-start", "parts": parts }))?)
                .map_err(|_| RemoteError::Status {
                    status: 200,
                    code: "bad-answer".into(),
                })?;

        let sent = (|| {
            let mut etags = Vec::new();
            for (i, url) in upload.urls.iter().enumerate() {
                let start = i as u64 * PART;
                let len = PART.min(total - start);
                etags.push(self.send_part(url, file, start, len, &report)?);
            }
            Ok::<_, RemoteError>(etags)
        })();
        let etags = match sent {
            Ok(etags) => etags,
            Err(e) => {
                let _ = self.call(json!({ "op": "upload-abort", "uploadId": upload.upload_id }));
                return Err(e);
            }
        };

        let finished = self.call(json!({
            "op": "upload-finish",
            "uploadId": upload.upload_id,
            "parts": etags,
            "etag": current,
        }))?;
        match finished {
            Ok(v) => Ok(Stored::Saved(v["etag"].as_str().map(str::to_string))),
            Err(res) if res.status() == 412 => Ok(Stored::Conflict),
            Err(res) => Err(fail(res)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a running sync function at `SYNC_TEST_SERVER` (with `SYNC_TEST_TOKEN` as
    /// the session), uploading a file of several parts; skipped otherwise.
    #[test]
    fn a_large_file_goes_up_in_parts_and_comes_back() {
        let Ok(url) = std::env::var("SYNC_TEST_SERVER") else {
            eprintln!("SYNC_TEST_SERVER not set; skipped");
            return;
        };
        let server = Server::new(Account {
            url,
            api_key: "test".into(),
            token: std::env::var("SYNC_TEST_TOKEN").unwrap_or_else(|_| "token".into()),
        });
        let dir = crate::book::test_dir();
        let (up, down) = (dir.join("server-up.bin"), dir.join("server-down.bin"));
        // 2.5 parts of bytes that don't compress.
        let mut seed = 0x2545_f491_u32;
        let bytes: Vec<u8> = (0..PART * 5 / 2)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        std::fs::write(&up, &bytes).unwrap();
        let sent = std::cell::RefCell::new(Vec::new());
        let progress = |s: Step| sent.borrow_mut().push(s);

        let first = match server.store(&up, None, &progress).unwrap() {
            Stored::Saved(Some(etag)) => etag,
            other => panic!("not stored: {other:?}"),
        };
        let total = bytes.len() as u64;
        assert_eq!(
            sent.borrow().last(),
            Some(&Step::Upload { done: total, total })
        );
        assert!(sent.borrow().len() > 10, "progress along the way");
        // Another device's upload over it, without having seen it, is refused.
        assert_eq!(server.store(&up, None, &|_| ()).unwrap(), Stored::Conflict);
        assert_eq!(
            server.store(&up, Some("\"stale\""), &|_| ()).unwrap(),
            Stored::Conflict
        );

        assert_eq!(
            server.fetch(Some(&first), &down, &|_| ()).unwrap(),
            Fetched::Unchanged
        );
        assert_eq!(
            server.fetch(None, &down, &|_| ()).unwrap(),
            Fetched::File {
                etag: Some(first.clone())
            }
        );
        assert!(std::fs::read(&down).unwrap() == bytes);
        // Keep the next run's first upload a first one.
        let small = dir.join("server-small.bin");
        std::fs::write(&small, b"x").unwrap();
        assert!(matches!(
            server.store(&small, Some(&first), &|_| ()).unwrap(),
            Stored::Saved(_)
        ));
    }
}
