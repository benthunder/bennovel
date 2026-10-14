//! S3-compatible storage the user runs (MinIO, RustFS…), as a `Remote` (remote.rs):
//! requests signed (AWS Signature V4) with the user's own keys. The keys stay in a file
//! next to the library, never in it, so they don't travel with the synced snapshot.
//!
//! The bucket holds one file, `<folder>/library.sqlite.gz`. A store only goes through if
//! the file is still the one merged (`If-Match` its ETag, or `If-None-Match: *` when
//! there was none). A server that ignores those headers still works, but the last
//! upload then wins.

use super::remote::{agent, Fetched, Progress, Remote, RemoteError, Step, Stored, MAX_DOWNLOAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const FILE: &str = "library.sqlite.gz";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// `https://host[:port]`, without the bucket (path-style requests).
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// Folder in the bucket for this library's file, e.g. `bennovel/mai`.
    pub folder: String,
    pub access_key: String,
    pub secret_key: String,
}

/// Where a library's S3 settings are kept: beside the library file.
pub fn config_path(library: &Path) -> PathBuf {
    library.with_extension("s3.json")
}

pub fn load_config(path: &Path) -> Option<Config> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn save_config(path: &Path, config: Option<&Config>) -> std::io::Result<()> {
    match config {
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        Some(c) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, serde_json::to_vec(c).map_err(std::io::Error::other)?)
        }
    }
}

impl Config {
    /// Trims the fields and fills the defaults; an error names what is missing.
    pub fn normalized(mut self) -> Result<Self, RemoteError> {
        for f in [
            &mut self.endpoint,
            &mut self.region,
            &mut self.bucket,
            &mut self.folder,
            &mut self.access_key,
            &mut self.secret_key,
        ] {
            *f = f.trim().to_string();
        }
        self.endpoint = self.endpoint.trim_end_matches('/').to_string();
        if !self.endpoint.contains("://") {
            self.endpoint = format!("https://{}", self.endpoint);
        }
        self.folder = self.folder.trim_matches('/').to_string();
        if self.region.is_empty() {
            self.region = "us-east-1".into();
        }
        let host = self
            .endpoint
            .split_once("://")
            .map(|(_, h)| h)
            .unwrap_or("");
        if host.is_empty() || host.contains('/') {
            return Err(RemoteError::Config("s3-endpoint"));
        }
        if self.bucket.is_empty() {
            return Err(RemoteError::Config("s3-bucket"));
        }
        if self.access_key.is_empty() || self.secret_key.is_empty() {
            return Err(RemoteError::Config("s3-keys"));
        }
        Ok(self)
    }

    fn object(&self) -> String {
        if self.folder.is_empty() {
            FILE.into()
        } else {
            format!("{}/{FILE}", self.folder)
        }
    }

    /// Identifies the file synced with, so a changed setting starts over.
    fn target(&self) -> String {
        format!("{}/{}/{}", self.endpoint, self.bucket, self.object())
    }
}

/// An answer from the storage.
struct Reply {
    status: u16,
    etag: Option<String>,
    body: Vec<u8>,
}

struct Client<'a> {
    config: &'a Config,
    agent: ureq::Agent,
}

impl<'a> Client<'a> {
    fn new(config: &'a Config) -> Self {
        Self {
            config,
            agent: agent(),
        }
    }

    /// Sends a signed request for `key` in the bucket (`None`: the bucket itself).
    /// Any status comes back as a `Reply`; only a failed connection is an error.
    fn send(
        &self,
        method: &str,
        key: Option<&str>,
        body: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<Reply, RemoteError> {
        let c = self.config;
        let host = c.endpoint.split_once("://").map(|(_, h)| h).unwrap_or("");
        let mut path = format!("/{}", uri_encode(&c.bucket, false));
        if let Some(key) = key {
            path.push('/');
            path.push_str(&uri_encode(key, true));
        }
        let payload = hex(&Sha256::digest(body));
        let now = AmzTime::now();
        let headers = [
            ("host", host),
            ("x-amz-content-sha256", payload.as_str()),
            ("x-amz-date", now.stamp.as_str()),
        ];
        let auth = authorization(c, method, &path, &headers, &payload, &now);

        let mut req = self
            .agent
            .request(method, &format!("{}{}", c.endpoint, path))
            .set("x-amz-content-sha256", &payload)
            .set("x-amz-date", &now.stamp)
            .set("authorization", &auth);
        for (k, v) in extra {
            req = req.set(k, v);
        }
        let res = match if body.is_empty() && method != "PUT" {
            req.call()
        } else {
            req.send_bytes(body)
        } {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(RemoteError::Network(t.to_string())),
        };
        let status = res.status();
        let etag = res.header("etag").map(str::to_string);
        let mut body = Vec::new();
        if method != "HEAD" {
            res.into_reader()
                .take(MAX_DOWNLOAD)
                .read_to_end(&mut body)
                .map_err(|e| RemoteError::Network(e.to_string()))?;
        }
        Ok(Reply { status, etag, body })
    }
}

/// The `<Code>` of an S3 error answer, if any.
fn error_code(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    text.split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map(|(code, _)| code.to_string())
        .unwrap_or_default()
}

fn fail(r: &Reply) -> RemoteError {
    RemoteError::Status {
        status: r.status,
        code: error_code(&r.body),
    }
}

/// Checks the settings reach the bucket with these keys (the file may not exist yet).
pub fn check(config: &Config) -> Result<(), RemoteError> {
    let r = Client::new(config).send(
        "GET",
        Some(&config.object()),
        &[],
        &[("range", "bytes=0-0")],
    )?;
    match (r.status, error_code(&r.body).as_str()) {
        (200 | 206 | 416, _) | (404, "NoSuchKey") => Ok(()),
        _ => Err(fail(&r)),
    }
}

/// The user's own storage, as a place to sync with.
pub struct S3 {
    config: Config,
}

impl S3 {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Remote for S3 {
    fn target(&self) -> String {
        self.config.target()
    }

    fn fetch(
        &self,
        known: Option<&str>,
        dest: &Path,
        progress: &Progress,
    ) -> Result<Fetched, RemoteError> {
        progress(Step::Download { done: 0, total: 0 });
        let mut headers = Vec::new();
        if let Some(etag) = known {
            headers.push(("if-none-match", etag));
        }
        let r =
            Client::new(&self.config).send("GET", Some(&self.config.object()), &[], &headers)?;
        match r.status {
            200 => {
                std::fs::write(dest, &r.body)?;
                Ok(Fetched::File { etag: r.etag })
            }
            304 => Ok(Fetched::Unchanged),
            404 => Ok(Fetched::Missing),
            _ => Err(fail(&r)),
        }
    }

    fn store(
        &self,
        file: &Path,
        current: Option<&str>,
        progress: &Progress,
    ) -> Result<Stored, RemoteError> {
        // Sent in one request, the way any S3 server takes it.
        let body = std::fs::read(file)?;
        let total = body.len() as u64;
        progress(Step::Upload { done: 0, total });
        let condition = match current {
            Some(etag) => ("if-match", etag),
            None => ("if-none-match", "*"),
        };
        let r = Client::new(&self.config).send(
            "PUT",
            Some(&self.config.object()),
            &body,
            &[condition, ("content-type", "application/gzip")],
        )?;
        match r.status {
            200 => {
                progress(Step::Upload { done: total, total });
                Ok(Stored::Saved(r.etag))
            }
            412 | 409 => Ok(Stored::Conflict),
            _ => Err(fail(&r)),
        }
    }
}

// --- AWS Signature Version 4 -------------------------------------------------------

struct AmzTime {
    /// `20130524T000000Z`
    stamp: String,
    /// `20130524`
    date: String,
}

impl AmzTime {
    fn now() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self::at(secs)
    }

    fn at(secs: i64) -> Self {
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        let date = format!("{year:04}{month:02}{day:02}");
        let stamp = format!(
            "{date}T{:02}{:02}{:02}Z",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        );
        Self { stamp, date }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encodes as S3 expects: unreserved characters stay, `/` too in a key.
fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The `Authorization` header for a request without a query string. `headers` are the
/// signed ones, lowercase names.
fn authorization(
    c: &Config,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    payload: &str,
    now: &AmzTime,
) -> String {
    let mut headers = headers.to_vec();
    headers.sort_by_key(|(k, _)| *k);
    let canonical_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
    let signed = signed.join(";");
    let request = format!("{method}\n{path}\n\n{canonical_headers}\n{signed}\n{payload}");
    let scope = format!("{}/{}/s3/aws4_request", now.date, c.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        now.stamp,
        hex(&Sha256::digest(request.as_bytes()))
    );
    let key = hmac(format!("AWS4{}", c.secret_key).as_bytes(), &now.date);
    let key = hmac(&key, &c.region);
    let key = hmac(&key, "s3");
    let key = hmac(&key, "aws4_request");
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={}",
        c.access_key,
        hex(&hmac(&key, &to_sign))
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::library::db;

    /// Trusts the certificates in `S3_TEST_CA` (a PEM file) too, when it is set.
    pub(crate) fn with_test_ca(builder: ureq::AgentBuilder) -> ureq::AgentBuilder {
        use rustls_pki_types::{pem::PemObject, CertificateDer};
        let Ok(file) = std::env::var("S3_TEST_CA") else {
            return builder;
        };
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_file_iter(file).unwrap() {
            let _ = roots.add(cert.unwrap());
        }
        let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        builder.tls_config(std::sync::Arc::new(config))
    }

    #[test]
    fn signs_like_the_aws_example() {
        // "Example: GET Object" from the AWS Signature Version 4 documentation.
        let c = Config {
            region: "us-east-1".into(),
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            ..Default::default()
        };
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let now = AmzTime::at(1_369_353_600);
        assert_eq!(now.stamp, "20130524T000000Z");
        let auth = authorization(
            &c,
            "GET",
            "/test.txt",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", empty),
                ("x-amz-date", "20130524T000000Z"),
            ],
            empty,
            &now,
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn settings_are_checked_and_defaulted() {
        let c = Config {
            endpoint: " minio.example.com:9000/ ".into(),
            bucket: "books".into(),
            folder: "/bennovel/mai/".into(),
            access_key: "a".into(),
            secret_key: "b".into(),
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(c.endpoint, "https://minio.example.com:9000");
        assert_eq!(c.region, "us-east-1");
        assert_eq!(c.object(), "bennovel/mai/library.sqlite.gz");
        let no_bucket = Config {
            bucket: "".into(),
            ..c.clone()
        }
        .normalized();
        assert!(matches!(no_bucket, Err(RemoteError::Config("s3-bucket"))));
        let path = Config {
            endpoint: "https://h/x".into(),
            ..c
        }
        .normalized();
        assert!(matches!(path, Err(RemoteError::Config("s3-endpoint"))));
    }

    #[test]
    fn keys_are_encoded_per_segment() {
        assert_eq!(uri_encode("a b/c+d.gz", true), "a%20b/c%2Bd.gz");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
    }

    /// Against a real server, when `S3_TEST_ENDPOINT` (and `S3_TEST_ACCESS_KEY`,
    /// `S3_TEST_SECRET_KEY`) name one, as in CI with MinIO and RustFS; skipped otherwise.
    #[test]
    fn two_devices_sync_through_real_storage() {
        use super::super::rules::{self, Kind, Rule};
        let Ok(endpoint) = std::env::var("S3_TEST_ENDPOINT") else {
            eprintln!("S3_TEST_ENDPOINT not set; skipped");
            return;
        };
        let run = std::process::id();
        let config = Config {
            endpoint,
            region: String::new(),
            // An existing bucket (S3_TEST_BUCKET), or a new one made for the test.
            bucket: std::env::var("S3_TEST_BUCKET")
                .unwrap_or_else(|_| format!("bennovel-test-{run}")),
            folder: format!("bennovel-test/{run}"),
            access_key: std::env::var("S3_TEST_ACCESS_KEY").unwrap(),
            secret_key: std::env::var("S3_TEST_SECRET_KEY").unwrap(),
        }
        .normalized()
        .unwrap();
        let client = Client::new(&config);
        if std::env::var("S3_TEST_BUCKET").is_err() {
            let made = client.send("PUT", None, &[], &[]).unwrap();
            assert_eq!(made.status, 200, "{}", String::from_utf8_lossy(&made.body));
        }
        check(&config).unwrap();
        // Local stand-ins such as moto accept any key; set S3_TEST_ANY_KEY for those.
        if std::env::var("S3_TEST_ANY_KEY").is_err() {
            let wrong = Config {
                secret_key: "wrong".into(),
                ..config.clone()
            };
            assert!(matches!(
                check(&wrong),
                Err(RemoteError::Status { status: 403, .. })
            ));
        }

        let device = |name: &str| {
            let path = crate::book::test_dir().join(format!("s3-{name}-{run}.sqlite3"));
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
            }
            db::open(&path).unwrap();
            path
        };
        let rule = |find: &str| {
            vec![Rule {
                find: find.into(),
                replace: "x".into(),
            }]
        };
        let rules_of = |path: &Path, id: i64| {
            rules::get(&db::open(path).unwrap(), Kind::Online, id, "en").unwrap()
        };
        let (a, b) = (device("a"), device("b"));
        rules::set(&db::open(&a).unwrap(), Kind::Online, 1, "en", &rule("one")).unwrap();
        rules::set(&db::open(&b).unwrap(), Kind::Online, 2, "en", &rule("two")).unwrap();

        let place = S3::new(config.clone());
        let sync = |path: &Path| super::super::remote::sync(path, &place, &|_| ()).unwrap();
        sync(&a); // first upload
        sync(&b); // takes A's, adds its own
        sync(&a); // takes B's
        for path in [&a, &b] {
            assert_eq!(rules_of(path, 1), Some(rule("one")));
            assert_eq!(rules_of(path, 2), Some(rule("two")));
        }

        // An upload over a file that changed since is refused (what keeps two devices
        // syncing at once from losing each other's changes).
        let stale = client
            .send(
                "PUT",
                Some(&config.object()),
                b"x",
                &[("if-match", "\"stale\"")],
            )
            .unwrap();
        assert_eq!(stale.status, 412);
        let first = client
            .send(
                "PUT",
                Some(&config.object()),
                b"x",
                &[("if-none-match", "*")],
            )
            .unwrap();
        assert_eq!(first.status, 412);
        let removed = client
            .send("DELETE", Some(&config.object()), &[], &[])
            .unwrap();
        assert_eq!(removed.status, 204);
    }
}
