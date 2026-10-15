//! S3-compatible storage the user runs (MinIO, RustFS…), for backups of the library:
//! requests signed (AWS Signature V4) with the user's own keys. The keys stay in a file
//! next to the library, never in it, so they don't travel with a backup.
//!
//! The bucket holds one backup per library, `<folder>/bennovel-backup.bnbackup` (the
//! same gzipped file as a backup saved to the device). A new backup replaces it.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FILE: &str = "bennovel-backup.bnbackup";
/// Refuse to download a file larger than this.
const MAX_DOWNLOAD: u64 = 4 << 30;

#[derive(Debug, thiserror::Error)]
pub enum S3Error {
    /// The storage answered with an error status (and its error code, if any).
    #[error("s3 {status} {code}")]
    Status { status: u16, code: String },
    #[error("{0}")]
    Network(String),
    /// Settings missing or wrong; the word names which.
    #[error("{0}")]
    Config(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// An HTTP client for the storage, through the system proxy when one is set.
fn agent() -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(120))
        .timeout_write(Duration::from_secs(300));
    #[cfg(test)]
    let builder = tests::with_test_ca(builder);
    builder.build()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// `https://host[:port]`, without the bucket (path-style requests).
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// Folder in the bucket for this library's backup, e.g. `bennovel/mai`.
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
    pub fn normalized(mut self) -> Result<Self, S3Error> {
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
            return Err(S3Error::Config("s3-endpoint"));
        }
        if self.bucket.is_empty() {
            return Err(S3Error::Config("s3-bucket"));
        }
        if self.access_key.is_empty() || self.secret_key.is_empty() {
            return Err(S3Error::Config("s3-keys"));
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
}

/// An answer from the storage.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
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
    fn request(
        &self,
        method: &str,
        key: Option<&str>,
        body: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<ureq::Response, S3Error> {
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
        match if body.is_empty() && method != "PUT" {
            req.call()
        } else {
            req.send_bytes(body)
        } {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(_, r)) => Ok(r),
            Err(ureq::Error::Transport(t)) => Err(S3Error::Network(t.to_string())),
        }
    }

    /// `request`, read whole. Any status comes back as a `Reply`; only a failed
    /// connection is an error.
    fn send(
        &self,
        method: &str,
        key: Option<&str>,
        body: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<Reply, S3Error> {
        let res = self.request(method, key, body, extra)?;
        let status = res.status();
        let headers = res
            .headers_names()
            .into_iter()
            .filter_map(|k| res.header(&k).map(|v| (k.clone(), v.to_string())))
            .collect();
        let mut body = Vec::new();
        if method != "HEAD" {
            res.into_reader()
                .take(MAX_DOWNLOAD)
                .read_to_end(&mut body)
                .map_err(|e| S3Error::Network(e.to_string()))?;
        }
        Ok(Reply {
            status,
            headers,
            body,
        })
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

fn fail(r: &Reply) -> S3Error {
    S3Error::Status {
        status: r.status,
        code: error_code(&r.body),
    }
}

/// Checks the settings reach the bucket with these keys (the backup may not exist yet).
pub fn check(config: &Config) -> Result<(), S3Error> {
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

/// The backup kept in the storage.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub size: u64,
    /// When it was stored, as the server says it (`Last-Modified`, an HTTP date).
    pub modified: Option<String>,
}

/// The backup in the storage, `None` when there is none yet.
pub fn info(config: &Config) -> Result<Option<Backup>, S3Error> {
    let r = Client::new(config).send("HEAD", Some(&config.object()), &[], &[])?;
    match r.status {
        200 => Ok(Some(Backup {
            size: r
                .header("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            modified: r.header("last-modified").map(str::to_string),
        })),
        404 => Ok(None),
        _ => Err(fail(&r)),
    }
}

/// Stores the backup file at `file`, replacing the one there.
pub fn upload(config: &Config, file: &Path) -> Result<(), S3Error> {
    // Sent in one request, the way any S3 server takes it.
    let body = std::fs::read(file)?;
    let r = Client::new(config).send(
        "PUT",
        Some(&config.object()),
        &body,
        &[("content-type", "application/gzip")],
    )?;
    match r.status {
        200 | 201 | 204 => Ok(()),
        _ => Err(fail(&r)),
    }
}

/// Downloads the backup into `dest`; false when the storage has none.
pub fn download(config: &Config, dest: &Path) -> Result<bool, S3Error> {
    let res = Client::new(config).request("GET", Some(&config.object()), &[], &[])?;
    match res.status() {
        200 => {
            let mut body = res.into_reader().take(MAX_DOWNLOAD);
            let mut file = std::io::BufWriter::new(std::fs::File::create(dest)?);
            std::io::copy(&mut body, &mut file).map_err(|e| S3Error::Network(e.to_string()))?;
            std::io::Write::flush(&mut file)?;
            Ok(true)
        }
        404 => Ok(false),
        status => {
            let mut body = Vec::new();
            let _ = res.into_reader().take(64 * 1024).read_to_end(&mut body);
            Err(S3Error::Status {
                status,
                code: error_code(&body),
            })
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
        assert_eq!(c.object(), "bennovel/mai/bennovel-backup.bnbackup");
        let no_bucket = Config {
            bucket: "".into(),
            ..c.clone()
        }
        .normalized();
        assert!(matches!(no_bucket, Err(S3Error::Config("s3-bucket"))));
        let path = Config {
            endpoint: "https://h/x".into(),
            ..c
        }
        .normalized();
        assert!(matches!(path, Err(S3Error::Config("s3-endpoint"))));
    }

    #[test]
    fn keys_are_encoded_per_segment() {
        assert_eq!(uri_encode("a b/c+d.gz", true), "a%20b/c%2Bd.gz");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
    }

    /// Against a real server, when `S3_TEST_ENDPOINT` (and `S3_TEST_ACCESS_KEY`,
    /// `S3_TEST_SECRET_KEY`) name one; skipped otherwise. A backup made on one device
    /// comes back on another.
    #[test]
    fn a_backup_goes_up_and_comes_back() {
        use super::super::{
            rules::{self, Kind, Rule},
            sync,
        };
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
                Err(S3Error::Status { status: 403, .. })
            ));
        }
        assert_eq!(info(&config).unwrap(), None);

        let device = |name: &str| {
            let path = crate::book::test_dir().join(format!("s3-{name}-{run}.sqlite3"));
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
            }
            db::open(&path).unwrap();
            path
        };
        let rule = vec![Rule {
            find: "one".into(),
            replace: "x".into(),
        }];
        let (a, b) = (device("a"), device("b"));
        rules::set(&db::open(&a).unwrap(), Kind::Online, 1, "en", &rule).unwrap();

        let gz = a.with_extension("up.gz");
        let size = sync::backup_to(&db::open(&a).unwrap(), &a.with_extension("up"), &gz).unwrap();
        upload(&config, &gz).unwrap();
        assert_eq!(info(&config).unwrap().map(|b| b.size), Some(size));

        let got = b.with_extension("down.gz");
        assert!(download(&config, &got).unwrap());
        let file = std::io::BufReader::new(std::fs::File::open(&got).unwrap());
        sync::restore_from(&mut db::open(&b).unwrap(), file, &b.with_extension("down")).unwrap();
        assert_eq!(
            rules::get(&db::open(&b).unwrap(), Kind::Online, 1, "en").unwrap(),
            Some(rule)
        );

        let removed = client
            .send("DELETE", Some(&config.object()), &[], &[])
            .unwrap();
        assert_eq!(removed.status, 204);
        assert!(!download(&config, &got).unwrap());
    }
}
