use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;
use crate::device::Device;

pub enum Failure {
    /// The server could not be reached or answered with a server error: try again later.
    Retry(String),
    /// The credential was revoked or is unknown: stop and pair again.
    Unpaired(String),
    /// The server refused the request for another reason.
    Refused(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Retry(m) | Failure::Unpaired(m) | Failure::Refused(m) => f.write_str(m),
        }
    }
}

fn env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| std::env::var(n).ok()).filter(|v| !v.is_empty())
}

/// Whether NO_PROXY exempts this host (exact names, domain suffixes, or "*").
pub fn bypass(host: &str, no_proxy: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_lowercase();
    no_proxy
        .split(',')
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .any(|e| e == "*" || host == e || host.ends_with(&format!(".{e}")))
}

/// Uses HTTPS_PROXY / HTTP_PROXY when set (as corporate networks often require), except for
/// hosts listed in NO_PROXY.
fn agent_for(url: &str) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("devicetrustline-agent/", env!("CARGO_PKG_VERSION")));
    let host = url.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
    let proxy = if url.starts_with("https://") {
        env(&["HTTPS_PROXY", "https_proxy"])
    } else {
        env(&["HTTP_PROXY", "http_proxy"])
    };
    let exempt = env(&["NO_PROXY", "no_proxy"]).is_some_and(|n| bypass(host, &n));
    if let (Some(p), false) = (proxy, exempt) {
        if let Ok(p) = ureq::Proxy::new(p) {
            builder = builder.proxy(p);
        }
    }
    builder.build()
}

fn call(req: ureq::Request, body: Option<Value>) -> Result<Value, Failure> {
    let res = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match res {
        Ok(r) => r.into_json().map_err(|e| Failure::Retry(format!("unreadable response: {e}"))),
        Err(ureq::Error::Status(code, r)) => {
            let body: Value = r.into_json().unwrap_or_default();
            let message = body["error"].as_str().unwrap_or("request refused").to_string();
            match code {
                401 => Err(Failure::Unpaired(message)),
                429 | 500.. => Err(Failure::Retry(format!("server error {code}: {message}"))),
                _ => Err(Failure::Refused(message)),
            }
        }
        Err(e) => Err(Failure::Retry(format!("cannot reach the server: {e}"))),
    }
}

pub fn pair(server: &str, code: &str, name: &str, platform: &str) -> Result<Value, Failure> {
    let body = json!({ "code": code, "name": name, "platform": platform, "version": env!("CARGO_PKG_VERSION") });
    let url = format!("{server}/api/agent/pair");
    call(agent_for(&url).post(&url), Some(body))
}

fn authed(c: &Config, method: &str, path: &str) -> ureq::Request {
    let url = format!("{}{path}", c.server);
    agent_for(&url).request(method, &url).set("Authorization", &format!("Bearer {}", c.token))
}

pub fn status(c: &Config) -> Result<Value, Failure> {
    call(authed(c, "GET", "/api/agent/status"), None)
}

pub fn report(c: &Config, devices: &[Device]) -> Result<Value, Failure> {
    let body = json!({ "version": env!("CARGO_PKG_VERSION"), "complete": true, "devices": devices });
    call(authed(c, "POST", "/api/agent/report"), Some(body))
}

pub fn disconnect(c: &Config) -> Result<Value, Failure> {
    call(authed(c, "POST", "/api/agent/disconnect"), Some(json!({})))
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_proxy_matching() {
        let list = "localhost,127.0.0.1,.corp.example";
        assert!(super::bypass("127.0.0.1", list));
        assert!(super::bypass("app.corp.example", list));
        assert!(!super::bypass("devicetrustline.com", list));
        assert!(super::bypass("anything", "*"));
    }
}
