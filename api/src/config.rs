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
    /// Public base URL used in emailed links and sign-in callbacks, e.g. https://devicetrustline.com.
    pub app_url: String,
    pub mail: MailConfig,
    pub google: Option<GoogleConfig>,
    /// Private directory for uploaded photos, receipts and documents.
    pub attachments_dir: String,
    pub max_upload_bytes: usize,
    /// New accounts must confirm their email before signing in (only when email delivery is configured).
    pub require_email_verification: bool,
    /// Browser push (Web Push); None until VAPID keys are set.
    pub push: Option<crate::push::PushConfig>,
    pub sms: crate::sms::SmsConfig,
}

#[derive(Clone, Debug)]
pub enum MailConfig {
    /// No email provider: nothing is sent and the app says so.
    Disabled,
    /// Local testing: messages are accepted into a mailbox readable at /api/dev/mailbox. Never in production.
    Capture { from: String },
    /// Any SMTP provider (Postmark, SendGrid, SES, Mailgun, Mailpit for local testing...).
    Smtp { url: String, from: String },
}

impl MailConfig {
    pub fn available(&self) -> bool {
        !matches!(self, MailConfig::Disabled)
    }
}

#[derive(Clone, Debug)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Only overridden by the test suite's stand-in identity provider.
    pub issuer: String,
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
        let app_origins: Vec<String> = var("APP_ORIGIN")
            .unwrap_or_else(|| "http://localhost:5173".into())
            .split(',')
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .collect();
        let app_url = var("APP_URL").unwrap_or_else(|| app_origins[0].clone()).trim_end_matches('/').to_string();
        let from = var("MAIL_FROM").unwrap_or_else(|| "DeviceTrustline <no-reply@devicetrustline.com>".into());
        let mail = match (var("MAIL_MODE").as_deref(), var("SMTP_URL")) {
            (Some("capture"), _) if is_production => return Err("MAIL_MODE=capture is for local testing only".into()),
            (Some("capture"), _) => MailConfig::Capture { from },
            (Some("disabled"), _) | (None, None) => MailConfig::Disabled,
            (_, Some(url)) => MailConfig::Smtp { url, from },
            (Some(other), None) => return Err(format!("MAIL_MODE={other} needs SMTP_URL")),
        };
        let google = match (var("GOOGLE_CLIENT_ID"), var("GOOGLE_CLIENT_SECRET")) {
            (Some(client_id), Some(client_secret)) => Some(GoogleConfig {
                client_id,
                client_secret,
                issuer: var("GOOGLE_ISSUER").unwrap_or_else(|| "https://accounts.google.com".into()),
            }),
            (None, None) => None,
            _ => return Err("Set both GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET, or neither".into()),
        };
        let push = match (var("VAPID_PRIVATE_KEY"), var("VAPID_SUBJECT")) {
            (Some(private), Some(subject)) => {
                Some(crate::push::PushConfig::new(&private, var("VAPID_PUBLIC_KEY").as_deref(), &subject)?)
            }
            (None, _) => None,
            (Some(_), None) => return Err("VAPID_PRIVATE_KEY needs VAPID_SUBJECT (mailto: or https: address)".into()),
        };
        let sms = match var("SMS_PROVIDER").as_deref() {
            None | Some("disabled") => crate::sms::SmsConfig::Disabled,
            Some("log") if is_production => return Err("SMS_PROVIDER=log is for local testing only".into()),
            Some("log") => crate::sms::SmsConfig::Log,
            Some("twilio") => match (var("TWILIO_ACCOUNT_SID"), var("TWILIO_AUTH_TOKEN"), var("TWILIO_FROM")) {
                (Some(account_sid), Some(auth_token), Some(from)) => crate::sms::SmsConfig::Twilio {
                    account_sid,
                    auth_token,
                    from,
                    api_base: var("TWILIO_API_BASE").unwrap_or_else(|| "https://api.twilio.com".into()),
                },
                _ => {
                    return Err("SMS_PROVIDER=twilio needs TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN and TWILIO_FROM".into())
                }
            },
            Some(other) => return Err(format!("Unknown SMS_PROVIDER {other}. Use twilio, log or disabled.")),
        };
        Ok(Config {
            push,
            sms,
            is_production,
            port: number("PORT", 4000)? as u16,
            database_url: required("DATABASE_URL", "postgres://postgres:postgres@localhost:5432/devicetrustline")?,
            database_ssl: var("DATABASE_SSL"),
            database_pool_size: number("DATABASE_POOL_SIZE", 10)? as u32,
            jwt_secret,
            session_hours: number("SESSION_HOURS", 8)? as i64,
            app_origins,
            allow_signup: flag("ALLOW_SIGNUP", !is_production),
            trust_proxy: flag("TRUST_PROXY", false),
            static_dir: var("STATIC_DIR"),
            log_requests: flag("LOG_REQUESTS", is_production),
            auth_rate_limit: number("AUTH_RATE_LIMIT", 20)? as u32,
            require_email_verification: flag("REQUIRE_EMAIL_VERIFICATION", matches!(mail, MailConfig::Smtp { .. })),
            app_url,
            mail,
            google,
            attachments_dir: var("ATTACHMENTS_DIR").unwrap_or_else(|| "./data/attachments".into()),
            max_upload_bytes: number("MAX_UPLOAD_MB", 10)? as usize * 1024 * 1024,
        })
    }
}
