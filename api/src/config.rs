use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub is_production: bool,
    pub port: u16,
    pub database_url: String,
    pub database_ssl: Option<String>,
    pub database_pool_size: u32,
    pub jwt_secret: String,
    pub session_hours: i64,
    /// Browser origins allowed to make state-changing requests with the session cookie.
    pub app_origins: Vec<String>,
    /// Self-service workspace sign-up. Turn off once your organization is set up.
    pub allow_signup: bool,
    pub trust_proxy: bool,
    /// Directory holding the built web app (vite build output). When set, the API also serves the app.
    pub static_dir: Option<String>,
    pub log_requests: bool,
    pub auth_rate_limit: u32,
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        // NODE_ENV is still honoured so existing deployments keep their settings.
        let is_production = var("APP_ENV").or_else(|| var("NODE_ENV")).as_deref() == Some("production");
        let required = |name: &str, fallback: &str| -> Result<String, String> {
            match var(name) {
                Some(v) => Ok(v),
                None if !is_production => Ok(fallback.to_string()),
                None => Err(format!("Missing required environment variable {name}")),
            }
        };
        let jwt_secret = required("JWT_SECRET", "dev-only-secret-change-me-dev-only-secret")?;
        if is_production && jwt_secret.len() < 32 {
            return Err("JWT_SECRET must be at least 32 characters in production".into());
        }
        let flag = |name: &str, default: bool| var(name).map(|v| v == "true").unwrap_or(default);
        let number = |name: &str, default: u64| -> Result<u64, String> {
            var(name).map(|v| v.parse().map_err(|_| format!("{name} must be a number"))).unwrap_or(Ok(default))
        };
        Ok(Config {
            is_production,
            port: number("PORT", 4000)? as u16,
            database_url: required("DATABASE_URL", "postgres://postgres:postgres@localhost:5432/devicetrustline")?,
            database_ssl: var("DATABASE_SSL"),
            database_pool_size: number("DATABASE_POOL_SIZE", 10)? as u32,
            jwt_secret,
            session_hours: number("SESSION_HOURS", 8)? as i64,
            app_origins: var("APP_ORIGIN")
                .unwrap_or_else(|| "http://localhost:5173".into())
                .split(',')
                .map(|o| o.trim().to_string())
                .filter(|o| !o.is_empty())
                .collect(),
            allow_signup: flag("ALLOW_SIGNUP", !is_production),
            trust_proxy: flag("TRUST_PROXY", false),
            static_dir: var("STATIC_DIR"),
            log_requests: flag("LOG_REQUESTS", is_production),
            auth_rate_limit: number("AUTH_RATE_LIMIT", 20)? as u32,
        })
    }
}
