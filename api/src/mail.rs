//! Email delivery. Everything is queued in `email_outbox`; a worker on any server instance claims
//! due messages with `FOR UPDATE SKIP LOCKED`, so several instances never send the same message.
//! A message is marked `sent` only after the provider accepted it.

use std::time::Duration;

use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::MailConfig;
use crate::db::{Db, Q};
use crate::error::{AppError, Result};

const MAX_ATTEMPTS: i32 = 6;

pub struct Email<'a> {
    pub org_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub to: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
    pub kind: &'a str,
    /// Messages with the same key are only ever queued once.
    pub dedupe_key: Option<String>,
}

/// Queues a message. With no email provider configured the row is recorded as `unavailable`.
pub async fn enqueue(db: &mut Db, mail: &MailConfig, e: Email<'_>) -> Result<()> {
    let status = if mail.available() { "queued" } else { "unavailable" };
    Q::new(
        "INSERT INTO email_outbox (org_id, user_id, to_address, subject, body_text, kind, dedupe_key, status)
         VALUES ($1, $2, $3::citext, $4, $5, $6, $7, $8) ON CONFLICT (dedupe_key) DO NOTHING",
    )
    .bind(e.org_id)
    .bind(e.user_id)
    .bind(e.to)
    .bind(e.subject)
    .bind(e.body)
    .bind(e.kind)
    .bind(e.dedupe_key)
    .bind(status)
    .run(db)
    .await?;
    Ok(())
}

enum Sender {
    Smtp(Box<AsyncSmtpTransport<Tokio1Executor>>, Mailbox),
    Capture,
}

fn sender(mail: &MailConfig) -> std::result::Result<Option<Sender>, String> {
    Ok(match mail {
        MailConfig::Disabled => None,
        MailConfig::Capture { .. } => Some(Sender::Capture),
        MailConfig::Smtp { url, from } => {
            let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
                .map_err(|e| format!("SMTP_URL is not valid: {e}"))?
                .timeout(Some(Duration::from_secs(20)))
                .build();
            let from: Mailbox = from.parse().map_err(|e| format!("MAIL_FROM is not valid: {e}"))?;
            Some(Sender::Smtp(Box::new(transport), from))
        }
    })
}

impl Sender {
    /// Hands one message to the provider; returns the provider's response on acceptance.
    async fn send(&self, to: &str, subject: &str, body: &str) -> std::result::Result<String, String> {
        match self {
            // Addresses starting with "bounce" are refused, so retries can be tested locally.
            Sender::Capture if to.starts_with("bounce") => Err("capture: simulated rejection".into()),
            Sender::Capture => Ok("captured".into()),
            Sender::Smtp(transport, from) => {
                let to: Mailbox = to.parse().map_err(|e| format!("bad recipient: {e}"))?;
                let message = Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject(subject)
                    .body(body.to_string())
                    .map_err(|e| e.to_string())?;
                let response = transport.send(message).await.map_err(|e| e.to_string())?;
                Ok(response.message().collect::<Vec<_>>().join(" "))
            }
        }
    }
}

/// Delivers due messages until none are left. Returns how many were accepted.
pub async fn deliver_due(pool: &PgPool, mail: &MailConfig) -> Result<usize> {
    let Some(sender) = sender(mail).map_err(AppError::Internal)? else { return Ok(0) };
    let mut db = pool.acquire().await?;
    // A message stuck in 'sending' (a server stopped mid-send) goes back to the queue.
    Q::new(
        "UPDATE email_outbox SET status = 'queued', locked_at = NULL
          WHERE status = 'sending' AND locked_at < now() - interval '10 minutes'",
    )
    .run(&mut db)
    .await?;
    let mut sent = 0;
    loop {
        let claimed = Q::new(
            "UPDATE email_outbox SET status = 'sending', locked_at = now(), attempts = attempts + 1
              WHERE id = (SELECT id FROM email_outbox WHERE status = 'queued' AND next_attempt_at <= now()
                           ORDER BY next_attempt_at, id LIMIT 1 FOR UPDATE SKIP LOCKED)
              RETURNING id, to_address, subject, body_text, attempts",
        )
        .opt(&mut db)
        .await?;
        let Some(m) = claimed else { break };
        let id = m["id"].as_i64().unwrap();
        let s = |v: &Value| v.as_str().unwrap_or("").to_string();
        match sender.send(&s(&m["to_address"]), &s(&m["subject"]), &s(&m["body_text"])).await {
            Ok(response) => {
                Q::new(
                    "UPDATE email_outbox SET status = 'sent', sent_at = now(), locked_at = NULL,
                            provider_message_id = $2, last_error = NULL WHERE id = $1",
                )
                .bind(id)
                .bind(response)
                .run(&mut db)
                .await?;
                sent += 1;
            }
            Err(error) => {
                let attempts = m["attempts"].as_i64().unwrap_or(1) as i32;
                // Retry with backoff: 1, 4, 9, 16, 25 minutes, then give up.
                Q::new(
                    "UPDATE email_outbox SET locked_at = NULL, last_error = $2,
                            status = CASE WHEN $3 >= $4 THEN 'failed' ELSE 'queued' END,
                            next_attempt_at = now() + make_interval(mins => $3 * $3)
                      WHERE id = $1",
                )
                .bind(id)
                .bind(error)
                .bind(attempts)
                .bind(MAX_ATTEMPTS)
                .run(&mut db)
                .await?;
            }
        }
    }
    Ok(sent)
}
