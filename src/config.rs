//! Local connection settings, persisted to the OS config directory.

use std::path::PathBuf;

/// Where the client remembers how to reach AS2Expert.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// "free", "b2b", or "custom".
    pub environment: String,
    /// Explicit base URL (used when environment == "custom").
    pub base_url: String,
    /// API bearer token.
    pub token: String,
    /// Whether to persist the token to disk (off by default for safety).
    pub remember_token: bool,
}

impl Config {
    /// Resolve the base URL the client should target.
    pub fn resolved_base_url(&self) -> String {
        match self.environment.as_str() {
            "custom" => self.base_url.trim().trim_end_matches('/').to_string(),
            other => as2expert::environment_url(other)
                .unwrap_or_default()
                .to_string(),
        }
    }

    /// Load config from disk, if present. Never fails — returns defaults instead.
    pub fn load() -> Self {
        let mut cfg = Config {
            environment: "free".to_string(),
            ..Default::default()
        };
        let Some(path) = config_file() else {
            return cfg;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return cfg;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            return cfg;
        };
        if let Some(s) = v.get("environment").and_then(|x| x.as_str()) {
            cfg.environment = s.to_string();
        }
        if let Some(s) = v.get("base_url").and_then(|x| x.as_str()) {
            cfg.base_url = s.to_string();
        }
        if let Some(s) = v.get("token").and_then(|x| x.as_str()) {
            cfg.token = s.to_string();
            cfg.remember_token = !s.is_empty();
        }
        cfg
    }

    /// Persist config to disk. The token is written only if `remember_token`.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = config_file() else {
            return Err(std::io::Error::other("no config directory"));
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let token = if self.remember_token {
            self.token.as_str()
        } else {
            ""
        };
        let body = serde_json::json!({
            "environment": self.environment,
            "base_url": self.base_url,
            "token": token,
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&body).unwrap_or_default())
    }
}

/// The per-user config directory for this app, computed from environment
/// variables (no extra crates).
fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("as2expert"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/as2expert"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("as2expert"))
    }
}

fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.json"))
}
