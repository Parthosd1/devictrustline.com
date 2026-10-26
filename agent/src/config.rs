use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What pairing leaves on this computer. The token is this computer's own credential; it reaches
/// only the device endpoints of one workspace and can be revoked from the web app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: String,
    pub agent_id: String,
    pub token: String,
    pub workspace: String,
    #[serde(default)]
    pub paused: bool,
}

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("DT_AGENT_CONFIG") {
        return PathBuf::from(p);
    }
    if cfg!(windows) {
        let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
        return PathBuf::from(base).join("DeviceTrustline").join("agent.json");
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("devicetrustline").join("agent.json")
}

pub fn load() -> Result<Config, String> {
    let p = path();
    let text = std::fs::read_to_string(&p)
        .map_err(|_| format!("This computer is not paired yet (no {}). Run: pair <server> <code>", p.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is damaged ({e}). Pair again.", p.display()))
}

pub fn save(c: &Config) -> Result<(), String> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c).unwrap())
        .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &p).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

pub fn forget() {
    let _ = std::fs::remove_file(path());
}

/// Only HTTPS, except to this computer itself (for local testing).
pub fn check_server(server: &str) -> Result<String, String> {
    let s = server.trim().trim_end_matches('/').to_string();
    let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|p| s == *p || s.starts_with(&format!("{p}:")) || s.starts_with(&format!("{p}/")));
    if s.starts_with("https://") || local {
        Ok(s)
    } else {
        Err("The server address must start with https://".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_https_or_this_computer() {
        assert_eq!(super::check_server("https://devicetrustline.com/").unwrap(), "https://devicetrustline.com");
        assert!(super::check_server("http://localhost:4000").is_ok());
        assert!(super::check_server("http://devicetrustline.com").is_err());
        assert!(super::check_server("http://localhost.evil.com").is_err());
    }
}
