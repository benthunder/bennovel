//! The BenNovel server: the Supabase Edge Function `sync` (supabase/functions/sync),
//! called with the account's session. It keeps the account's file in the storage the
//! project chose and holds that storage's keys, so the app never sees them.

use super::remote::{agent, Account, Fetched, Remote, RemoteError, Stored, MAX_DOWNLOAD};
use std::io::Read;

pub struct Server {
    account: Account,
    agent: ureq::Agent,
}

impl Server {
    pub fn new(account: Account) -> Self {
        Self {
            account,
            agent: agent(),
        }
    }

    fn request(&self, method: &str) -> ureq::Request {
        self.agent
            .request(method, &self.account.url)
            .set("authorization", &format!("Bearer {}", self.account.token))
            .set("apikey", &self.account.api_key)
    }
}

/// Turns any answer, error statuses included, into the response; only a failed
/// connection is an error.
fn answer(result: Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, RemoteError> {
    match result {
        Ok(r) | Err(ureq::Error::Status(_, r)) => Ok(r),
        Err(ureq::Error::Transport(t)) => Err(RemoteError::Network(t.to_string())),
    }
}

fn fail(res: ureq::Response) -> RemoteError {
    let status = res.status();
    let code = res
        .into_string()
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("error")?.as_str().map(str::to_string))
        .unwrap_or_default();
    match (status, code.as_str()) {
        (401, _) => RemoteError::SignIn,
        (503, "not-configured") => RemoteError::Config("sync-server-off"),
        _ => RemoteError::Status { status, code },
    }
}

impl Remote for Server {
    fn target(&self) -> String {
        // The server keeps one file per account; the token is not part of it.
        format!("server {}", self.account.url)
    }

    fn fetch(&self, known: Option<&str>) -> Result<Fetched, RemoteError> {
        let mut req = self.request("GET");
        if let Some(etag) = known {
            req = req.set("x-sync-etag", etag);
        }
        let res = answer(req.call())?;
        match res.status() {
            200 => {
                let etag = res.header("x-sync-etag").map(str::to_string);
                let mut body = Vec::new();
                res.into_reader()
                    .take(MAX_DOWNLOAD)
                    .read_to_end(&mut body)
                    .map_err(|e| RemoteError::Network(e.to_string()))?;
                Ok(Fetched::File { body, etag })
            }
            304 => Ok(Fetched::Unchanged),
            404 => Ok(Fetched::Missing),
            _ => Err(fail(res)),
        }
    }

    fn store(&self, body: &[u8], current: Option<&str>) -> Result<Stored, RemoteError> {
        let mut req = self.request("PUT").set("content-type", "application/gzip");
        if let Some(etag) = current {
            req = req.set("x-sync-expected", etag);
        }
        let res = answer(req.send_bytes(body))?;
        match res.status() {
            200 => Ok(Stored::Saved(res.header("x-sync-etag").map(str::to_string))),
            412 => Ok(Stored::Conflict),
            _ => Err(fail(res)),
        }
    }
}
