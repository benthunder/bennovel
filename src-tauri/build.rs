/// Settings of the BenNovel sync server (RustFS), built into the app: read from the
/// environment (GitHub Secrets in CI), else `.env.local`, else `.env` at the repo root.
/// The keys live in `.env.local`, which git ignores.
const SYNC_VARS: [&str; 5] = [
    "BENNOVEL_S3_ENDPOINT",
    "BENNOVEL_S3_REGION",
    "BENNOVEL_S3_BUCKET",
    "BENNOVEL_S3_ACCESS_KEY",
    "BENNOVEL_S3_SECRET_KEY",
];

fn main() {
    let mut files = std::collections::HashMap::new();
    for name in ["../.env", "../.env.local"] {
        println!("cargo:rerun-if-changed={name}");
        let Ok(text) = std::fs::read_to_string(name) else {
            continue;
        };
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                files.insert(k.trim().to_string(), v.to_string());
            }
        }
    }
    for var in SYNC_VARS {
        println!("cargo:rerun-if-env-changed={var}");
        let value = std::env::var(var)
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| files.get(var).cloned())
            .unwrap_or_default();
        println!("cargo:rustc-env={var}={value}");
    }
    tauri_build::build()
}
