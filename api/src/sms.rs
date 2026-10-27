//! Text messages behind a provider interface. Off unless SMS_PROVIDER is set; then deliveries say
//! `unavailable` instead of pretending to send. `log` is a development adapter that only records
//! the message; `twilio` sends through Twilio's REST API.

#[derive(Clone, Debug)]
pub enum SmsConfig {
    Disabled,
    /// Local development and tests: accepted and recorded, never sent. Not allowed in production.
    Log,
    Twilio {
        account_sid: String,
        auth_token: String,
        from: String,
        api_base: String,
    },
}

impl SmsConfig {
    pub fn available(&self) -> bool {
        !matches!(self, SmsConfig::Disabled)
    }

    /// Sends one message; returns the provider's message id.
    pub async fn send(&self, to: &str, body: &str) -> Result<String, String> {
        match self {
            SmsConfig::Disabled => Err("SMS is not configured".into()),
            // Numbers ending in 0000 are refused, so retries can be tested locally.
            SmsConfig::Log if to.ends_with("0000") => Err("log: simulated rejection".into()),
            SmsConfig::Log => Ok(format!("log:{}", body.chars().count())),
            SmsConfig::Twilio { account_sid, auth_token, from, api_base } => {
                let url = format!("{api_base}/2010-04-01/Accounts/{account_sid}/Messages.json");
                let response = crate::push::http()
                    .post(url)
                    .basic_auth(account_sid, Some(auth_token))
                    .form(&[("To", to), ("From", from.as_str()), ("Body", body)])
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let status = response.status();
                let json: serde_json::Value = response.json().await.unwrap_or_default();
                if status.is_success() {
                    Ok(json["sid"].as_str().unwrap_or("accepted").to_string())
                } else {
                    Err(format!("twilio answered {status}: {}", json["message"].as_str().unwrap_or("")))
                }
            }
        }
    }
}
