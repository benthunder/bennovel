//! The user's own LLM for AI translation on this device: any chat API in the OpenAI
//! format (`POST {base_url}/chat/completions`), so OpenAI, Gemini, DeepSeek,
//! OpenRouter, a local Ollama… all work by their base URL, key and model name.
//! The settings live in the library file (`llm_config`) and go into backups, key
//! included (`sync::backup_to`); the key never goes back to the web side.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// No LLM is set up in Profile.
    #[error("llm-not-set")]
    NotSet,
    /// The API turned the key down (401/403).
    #[error("llm-auth: {0}")]
    Auth(String),
    /// The API could not be reached.
    #[error("llm-network: {0}")]
    Network(String),
    /// The API took longer than the read timeout to answer. Not tried again: the
    /// next try would most likely wait as long.
    #[error("llm-timeout")]
    Timeout,
    /// Any other answer than a reply (bad model name, rate limit…).
    #[error("llm-http {0}: {1}")]
    Http(u16, String),
    /// A reply without text.
    #[error("llm-empty")]
    Empty,
    #[error("llm-other: {0}")]
    Other(String),
    #[error("llm-other: {0}")]
    Db(#[from] rusqlite::Error),
}

impl Serialize for AiError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// The saved LLM. `provider` is the preset picked in the app ("openai", "custom"…).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// The saved LLM as shown in the app: no key.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
}

pub fn load(conn: &Connection) -> rusqlite::Result<Option<Config>> {
    conn.query_row(
        "select provider, base_url, api_key, model from llm_config where id = 1",
        [],
        |r| {
            Ok(Config {
                provider: r.get(0)?,
                base_url: r.get(1)?,
                api_key: r.get(2)?,
                model: r.get(3)?,
            })
        },
    )
    .optional()
}

pub fn settings(conn: &Connection) -> rusqlite::Result<Option<Settings>> {
    Ok(load(conn)?.map(|c| Settings {
        provider: c.provider,
        base_url: c.base_url,
        model: c.model,
        has_key: !c.api_key.is_empty(),
    }))
}

/// `config` with the saved key when its key is left empty, as long as it is for the
/// same provider and address (a key is never sent to another server).
pub fn with_saved_key(conn: &Connection, mut config: Config) -> rusqlite::Result<Config> {
    config.base_url = normalize_base(&config.base_url);
    if config.api_key.trim().is_empty() {
        if let Some(saved) = load(conn)? {
            if saved.provider == config.provider.trim() && saved.base_url == config.base_url {
                config.api_key = saved.api_key;
            }
        }
    }
    Ok(config)
}

/// Saves the LLM as given (None forgets it).
pub fn save(conn: &Connection, config: Option<&Config>) -> rusqlite::Result<()> {
    match config {
        None => {
            conn.execute("delete from llm_config", [])?;
        }
        Some(c) => {
            conn.execute(
                "insert into llm_config (id, provider, base_url, api_key, model, updated_at)
                 values (1, ?1, ?2, ?3, ?4, ?5)
                 on conflict (id) do update set provider = excluded.provider,
                   base_url = excluded.base_url, model = excluded.model,
                   api_key = excluded.api_key, updated_at = excluded.updated_at",
                params![
                    c.provider.trim(),
                    normalize_base(&c.base_url),
                    c.api_key.trim(),
                    c.model.trim(),
                    super::db::now_ms()
                ],
            )?;
        }
    }
    Ok(())
}

/// `https://api.openai.com/v1/` → `https://api.openai.com/v1`; a full
/// `…/chat/completions` URL pasted by mistake is cut back too.
pub fn normalize_base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    url.strip_suffix("/chat/completions")
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_string()
}

/// Whether the API runs on this computer or the local network.
fn is_local(base_url: &str) -> bool {
    let rest = base_url
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(base_url);
    let host = rest.split(['/', '?']).next().unwrap_or("");
    let host = if host.starts_with('[') {
        host.split(']').next().unwrap_or("").trim_start_matches('[')
    } else {
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".local")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| match ip {
                std::net::IpAddr::V4(v4) => {
                    v4.is_loopback() || v4.is_private() || v4.is_link_local()
                }
                std::net::IpAddr::V6(v6) => v6.is_loopback(),
            })
            .unwrap_or(false)
}

/// A chat model: one system prompt and one user message in, the reply out. The graph
/// in translate.rs runs on this, so tests can stand in for the API.
pub trait Chat: Send + Sync {
    fn chat(&self, system: &str, user: &str) -> Result<String, AiError>;
    /// The model name kept with each translation.
    fn model(&self) -> &str;
    /// One turn with tools (OpenAI `tools` format): the assistant message as the API
    /// gives it, with `tool_calls` when the model wants a tool run.
    fn chat_tools(
        &self,
        _messages: &[serde_json::Value],
        _tools: &serde_json::Value,
    ) -> Result<serde_json::Value, AiError> {
        Err(AiError::Other("tools-unsupported".into()))
    }
}

/// An OpenAI-compatible chat API.
pub struct OpenAi {
    config: Config,
    agent: ureq::Agent,
}

impl OpenAi {
    pub fn new(config: Config) -> Self {
        let agent = ureq::AgentBuilder::new()
            // A model on this computer or the home network (Ollama, LM Studio) is
            // reached directly, not through the system proxy.
            .try_proxy_from_env(!is_local(&config.base_url))
            .timeout_connect(Duration::from_secs(20))
            // One request is a part of a chapter (about 3,500 characters); a slow
            // local model can take a few minutes to write it, but not more.
            .timeout_read(Duration::from_secs(300))
            .timeout_write(Duration::from_secs(60))
            .build();
        Self { config, agent }
    }

    fn post(&self, body: &serde_json::Value) -> Result<String, AiError> {
        reply_text(&self.post_to("chat/completions", body)?).ok_or(AiError::Empty)
    }

    /// POSTs `body` to `{base_url}/{path}` and returns the JSON answer.
    fn post_to(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, AiError> {
        let text = self.send(path, body)?.into_string().map_err(io_error)?;
        serde_json::from_str(&text).map_err(|e| AiError::Other(e.to_string()))
    }

    /// A chat request with `"stream": true`: the text comes in pieces as the model
    /// writes it, so a slow model keeps the connection busy and a gateway in between
    /// (Cloudflare cuts a silent request after about 100 s, some proxies after a few
    /// minutes) does not drop it; the read timeout counts from the last piece. A
    /// server that answers in one JSON anyway is read as such.
    fn post_stream(&self, body: &serde_json::Value) -> Result<String, AiError> {
        let mut body = body.clone();
        body["stream"] = serde_json::Value::Bool(true);
        let res = self.send("chat/completions", &body)?;
        if !res.content_type().contains("event-stream") {
            let text = res.into_string().map_err(io_error)?;
            let data: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| AiError::Other(e.to_string()))?;
            return reply_text(&data).ok_or(AiError::Empty);
        }
        let text = read_stream(std::io::BufReader::new(res.into_reader()))?;
        let text = text.trim();
        if text.is_empty() {
            return Err(AiError::Empty);
        }
        Ok(text.to_string())
    }

    /// POSTs `body` to `{base_url}/{path}`; an answer other than 2xx is an error.
    fn send(&self, path: &str, body: &serde_json::Value) -> Result<ureq::Response, AiError> {
        let url = format!("{}/{path}", normalize_base(&self.config.base_url));
        let mut req = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json");
        if !self.config.api_key.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.config.api_key));
        }
        match req.send_string(&body.to_string()) {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                let msg = error_message(&text);
                Err(match code {
                    401 | 403 => AiError::Auth(msg),
                    _ => AiError::Http(code, msg),
                })
            }
            Err(ureq::Error::Transport(t)) => {
                let msg = t.to_string();
                Err(if is_timeout(&msg) {
                    AiError::Timeout
                } else {
                    AiError::Network(msg)
                })
            }
        }
    }

    /// `send`, tried twice more when the server is busy or rate limits.
    fn retry<T>(&self, send: impl Fn() -> Result<T, AiError>) -> Result<T, AiError> {
        let mut wait = 4;
        for _ in 0..2 {
            match send() {
                Err(AiError::Http(429 | 500 | 502 | 503 | 504, _)) | Err(AiError::Network(_)) => {
                    std::thread::sleep(Duration::from_secs(wait));
                    wait *= 4;
                }
                other => return other,
            }
        }
        send()
    }
}

impl Chat for OpenAi {
    fn chat(&self, system: &str, user: &str) -> Result<String, AiError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "temperature": 0.3,
        });
        // Rate limits and busy servers pass; try twice more before giving up.
        self.retry(|| self.post_stream(&body))
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn chat_tools(
        &self,
        messages: &[serde_json::Value],
        tools: &serde_json::Value,
    ) -> Result<serde_json::Value, AiError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0.2,
        });
        let data = self.retry(|| self.post_to("chat/completions", &body))?;
        data.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or(AiError::Empty)
    }
}

/// A read that failed while the answer came in.
fn io_error(e: std::io::Error) -> AiError {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => AiError::Timeout,
        _ => AiError::Network(e.to_string()),
    }
}

/// The text of a streamed chat answer (server-sent events): every
/// `choices[0].delta.content` until `data: [DONE]` or the end. The model's thinking
/// (`reasoning_content`) is left out; an `error` sent mid-stream fails it.
fn read_stream(reader: impl std::io::BufRead) -> Result<String, AiError> {
    let mut text = String::new();
    for line in reader.lines() {
        let line = line.map_err(io_error)?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if let Some(e) = event.get("error") {
            let msg = e
                .get("message")
                .and_then(|m| m.as_str())
                .map(String::from)
                .unwrap_or_else(|| e.to_string());
            return Err(AiError::Other(msg));
        }
        if let Some(piece) = event
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            text.push_str(piece);
        }
    }
    Ok(text)
}

/// Whether a transport error is the read or write timeout running out.
fn is_timeout(msg: &str) -> bool {
    let msg = msg.to_ascii_lowercase();
    msg.contains("timed out") || msg.contains("timeout")
}

/// `choices[0].message.content` of a chat reply.
fn reply_text(data: &serde_json::Value) -> Option<String> {
    let text = data
        .get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?
        .as_str()?
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// `error.message` of an error reply, or the start of the body.
fn error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            let e = v.get("error")?;
            e.get("message")
                .and_then(|m| m.as_str())
                .or_else(|| e.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| body.chars().take(300).collect())
}

/// Checks that the API answers with these settings (a tiny request).
pub fn check(config: &Config) -> Result<(), AiError> {
    OpenAi::new(config.clone()).post(&serde_json::json!({
        "model": config.model,
        "messages": [{ "role": "user", "content": "Reply with OK." }],
        "max_tokens": 5,
    }))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_servers_skip_the_proxy() {
        assert!(is_local("http://localhost:11434/v1"));
        assert!(is_local("http://127.0.0.1:1234/v1"));
        assert!(is_local("http://192.168.1.5:8000/v1"));
        assert!(is_local("http://[::1]:8080/v1"));
        assert!(!is_local("https://api.openai.com/v1"));
    }

    #[test]
    fn base_urls_are_normalized() {
        assert_eq!(
            normalize_base(" https://api.openai.com/v1/ "),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            normalize_base("http://localhost:11434/v1/chat/completions"),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn a_saved_key_stays_with_its_server() {
        let (conn, _) = crate::library::translate::tests::book("key");
        let c = |provider: &str, base: &str, key: &str| Config {
            provider: provider.into(),
            base_url: base.into(),
            api_key: key.into(),
            model: "m".into(),
        };
        save(
            &conn,
            Some(&c("openai", "https://api.openai.com/v1", "sk-1")),
        )
        .unwrap();
        let same = with_saved_key(&conn, c("openai", "https://api.openai.com/v1/", "")).unwrap();
        assert_eq!(same.api_key, "sk-1");
        let other = with_saved_key(&conn, c("custom", "http://localhost:11434/v1", "")).unwrap();
        assert_eq!(other.api_key, "");
    }

    /// A one-request HTTP server answering `status` and `body`; returns its base URL
    /// and the request it got.
    fn serve(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        serve_typed(status, "application/json", body)
    }

    /// `serve` with its own Content-Type.
    fn serve_typed(
        status: &'static str,
        content_type: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req);
                if let Some(i) = text.find("\r\n\r\n") {
                    let len = text[..i]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if req.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            write!(s, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&req).into_owned()
        });
        (url, handle)
    }

    fn config(base_url: String) -> Config {
        Config {
            provider: "custom".into(),
            base_url,
            api_key: "sk-test".into(),
            model: "m-1".into(),
        }
    }

    #[test]
    fn talks_to_an_openai_compatible_api() {
        let (url, server) = serve(
            "200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"Xin chào"}}]}"#,
        );
        let reply = OpenAi::new(config(url)).chat("Translate", "Hello").unwrap();
        assert_eq!(reply, "Xin chào");
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /v1/chat/completions"));
        assert!(req.contains("Bearer sk-test"));
        assert!(req.contains(r#""model":"m-1""#));
    }

    #[test]
    fn a_refused_key_is_an_auth_error() {
        let (url, server) = serve(
            "401 Unauthorized",
            r#"{"error":{"message":"Incorrect API key"}}"#,
        );
        match check(&config(url)) {
            Err(AiError::Auth(m)) => assert_eq!(m, "Incorrect API key"),
            other => panic!("{other:?}"),
        }
        server.join().unwrap();
    }

    #[test]
    fn sends_tools_and_returns_the_calls() {
        let (url, server) = serve(
            "200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"wiki_search","arguments":"{\"query\":\"x\"}"}}]}}]}"#,
        );
        let msg = OpenAi::new(config(url))
            .chat_tools(
                &[serde_json::json!({ "role": "user", "content": "hi" })],
                &serde_json::json!([{ "type": "function", "function": { "name": "wiki_search" } }]),
            )
            .unwrap();
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "wiki_search");
        let req = server.join().unwrap();
        assert!(req.contains(r#""tools":[{"#));
    }

    #[test]
    fn streams_the_reply() {
        let (url, server) = serve_typed(
            "200 OK",
            "text/event-stream",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"hmm\"}}]}\n\n: keep-alive\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Xin \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"chào\"}}]}\n\ndata: [DONE]\n\n",
        );
        let reply = OpenAi::new(config(url)).chat("Translate", "Hello").unwrap();
        assert_eq!(reply, "Xin chào");
        let req = server.join().unwrap();
        assert!(req.contains(r#""stream":true"#));
    }

    #[test]
    fn an_error_in_the_stream_fails_the_reply() {
        let r = read_stream(std::io::Cursor::new(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Xin\"}}]}\n\ndata: {\"error\":{\"message\":\"upstream gone\"}}\n\n",
        ));
        match r {
            Err(AiError::Other(m)) => assert_eq!(m, "upstream gone"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn timeouts_are_told_apart() {
        assert!(is_timeout(
            "http://localhost:11434/v1/chat/completions: Network Error: timed out reading response"
        ));
        assert!(!is_timeout(
            "Dns Failed: resolve dns name 'api.example.com:443'"
        ));
        assert_eq!(AiError::Timeout.to_string(), "llm-timeout");
    }

    #[test]
    fn reads_replies_and_errors() {
        let ok = serde_json::json!({ "choices": [{ "message": { "content": " Xin chào \n" } }] });
        assert_eq!(reply_text(&ok).as_deref(), Some("Xin chào"));
        assert_eq!(reply_text(&serde_json::json!({ "choices": [] })), None);
        assert_eq!(
            error_message(r#"{"error":{"message":"bad model"}}"#),
            "bad model"
        );
        assert_eq!(error_message("nope"), "nope");
    }
}
