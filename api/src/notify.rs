//! Sending a notification to one person, and delivering it outside the app.
//!
//! `send` records the in-app notification and queues one delivery per channel the person has
//! turned on (email, browser push, SMS), honouring their quiet hours. It is idempotent: a
//! notification with the same dedupe key (or for the same reminder) is only ever created once,
//! so its deliveries are only ever queued once. `deliver_due` sends queued push and SMS
//! deliveries with retries; email goes through the email outbox, which retries on its own.
//! Every delivery, including ones that could not be attempted, is logged with its outcome.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::{uuid_of, Db, Q};
use crate::error::Result;
use crate::mail::{self, Email};
use crate::push::PushError;

const MAX_ATTEMPTS: i64 = 6;

pub struct Notice<'a> {
    pub org_id: Uuid,
    pub user_id: Uuid,
    /// What it is about: a reminder kind or an event name.
    pub kind: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub asset_id: Option<Uuid>,
    /// A derived (workspace) reminder.
    pub reminder_id: Option<i64>,
    /// A reminder the person set themselves.
    pub user_reminder_id: Option<Uuid>,
    /// Notifications with the same key are created once.
    pub dedupe_key: Option<String>,
    /// Where in the app it leads, e.g. /?asset=DT-1001.
    pub link: Option<String>,
    pub email_kind: &'a str,
    pub email_dedupe_key: Option<String>,
    /// Channels this kind of notification may use; None means every channel the person turned on.
    pub channels: Option<Channels>,
}

impl<'a> Notice<'a> {
    pub fn new(org_id: Uuid, user_id: Uuid, kind: &'a str, title: &'a str, body: &'a str) -> Notice<'a> {
        Notice {
            org_id,
            user_id,
            kind,
            title,
            body,
            asset_id: None,
            reminder_id: None,
            user_reminder_id: None,
            dedupe_key: None,
            link: None,
            email_kind: "notification",
            email_dedupe_key: None,
            channels: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channels {
    pub in_app: bool,
    pub email: bool,
    pub push: bool,
    pub sms: bool,
}

impl Channels {
    pub const ALL: Channels = Channels { in_app: true, email: true, push: true, sms: true };
}

async fn log(
    db: &mut Db,
    notification: i64,
    user: Uuid,
    channel: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    Q::new(
        "INSERT INTO notification_deliveries (notification_id, user_id, channel, status, last_error)
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(notification)
    .bind(user)
    .bind(channel)
    .bind(status)
    .bind(error)
    .run(db)
    .await?;
    Ok(())
}

/// Creates the notification and queues its deliveries. Returns its id, or None when the person
/// turned notifications off, the notification already exists, or every channel is off for it.
pub async fn send(db: &mut Db, config: &Config, n: Notice<'_>) -> Result<Option<i64>> {
    let prefs = Q::new(
        "SELECT u.email::text AS email, u.email_notifications AS email_on, u.is_active,
                COALESCE(s.enabled, true) AS enabled, COALESCE(s.push, false) AS push, COALESCE(s.sms, false) AS sms,
                s.phone, o.name AS workspace,
                quiet_hours_end(COALESCE(s.timezone, o.timezone), s.quiet_start, s.quiet_end, now()) AS send_at
           FROM users u JOIN organizations o ON o.id = $2
           LEFT JOIN notification_settings s ON s.user_id = u.id
          WHERE u.id = $1",
    )
    .bind(n.user_id)
    .bind(n.org_id)
    .opt(&mut *db)
    .await?;
    let Some(p) = prefs else { return Ok(None) };
    let allowed = n.channels.unwrap_or(Channels::ALL);
    if p["enabled"] != true || p["is_active"] != true || !allowed.in_app {
        return Ok(None);
    }
    let id: Option<i64> = Q::new(
        "INSERT INTO notifications (org_id, user_id, reminder_id, user_reminder_id, asset_id, kind, title, body, dedupe_key, link)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(n.org_id)
    .bind(n.user_id)
    .bind(n.reminder_id)
    .bind(n.user_reminder_id)
    .bind(n.asset_id)
    .bind(n.kind)
    .bind(n.title)
    .bind(n.body)
    .bind(&n.dedupe_key)
    .bind(&n.link)
    .scalars(&mut *db)
    .await?
    .into_iter()
    .next();
    let Some(id) = id else { return Ok(None) };
    let send_at = p["send_at"].as_str().unwrap_or("now()").to_string();

    if allowed.email && p["email_on"] == true {
        if config.mail.available() {
            let link = n.link.as_ref().map(|l| format!("\n\nOpen: {}{l}", config.app_url)).unwrap_or_default();
            let text = format!(
                "{}{link}\n\nWorkspace: {}\n\nYou get these because email notifications are on in your DeviceTrustline notification settings.",
                n.body,
                p["workspace"].as_str().unwrap_or("")
            );
            let message = Email {
                org_id: Some(n.org_id),
                user_id: Some(n.user_id),
                to: p["email"].as_str().unwrap_or(""),
                subject: n.title,
                body: &text,
                kind: n.email_kind,
                dedupe_key: Some(n.email_dedupe_key.clone().unwrap_or_else(|| format!("notification:{id}"))),
            };
            if let Some(email_id) = mail::enqueue(&mut *db, &config.mail, message).await? {
                Q::new("UPDATE email_outbox SET next_attempt_at = GREATEST(next_attempt_at, $2::timestamptz) WHERE id = $1")
                    .bind(email_id)
                    .bind(&send_at)
                    .run(&mut *db)
                    .await?;
                Q::new("UPDATE notifications SET email_id = $2 WHERE id = $1")
                    .bind(id)
                    .bind(email_id)
                    .run(&mut *db)
                    .await?;
                Q::new(
                    "INSERT INTO notification_deliveries (notification_id, user_id, channel, email_id, status)
                     VALUES ($1, $2, 'email', $3, 'queued') ON CONFLICT DO NOTHING",
                )
                .bind(id)
                .bind(n.user_id)
                .bind(email_id)
                .run(&mut *db)
                .await?;
            }
        } else {
            log(db, id, n.user_id, "email", "unavailable", Some("Email is not set up on this server")).await?;
        }
    }

    if allowed.push && p["push"] == true {
        let subscriptions: Vec<Uuid> =
            Q::new("SELECT id FROM push_subscriptions WHERE user_id = $1").bind(n.user_id).scalars(&mut *db).await?;
        if config.push.is_none() {
            log(db, id, n.user_id, "push", "unavailable", Some("Browser push is not set up on this server")).await?;
        } else if subscriptions.is_empty() {
            log(db, id, n.user_id, "push", "unavailable", Some("No browser is subscribed")).await?;
        }
        if config.push.is_some() {
            for sub in subscriptions {
                Q::new(
                    "INSERT INTO notification_deliveries (notification_id, user_id, channel, subscription_id, next_attempt_at)
                     VALUES ($1, $2, 'push', $3, $4::timestamptz) ON CONFLICT DO NOTHING",
                )
                .bind(id)
                .bind(n.user_id)
                .bind(sub)
                .bind(&send_at)
                .run(&mut *db)
                .await?;
            }
        }
    }

    if allowed.sms && p["sms"] == true {
        if !config.sms.available() {
            log(db, id, n.user_id, "sms", "unavailable", Some("Text messages are not set up on this server")).await?;
        } else if p["phone"].is_null() {
            log(db, id, n.user_id, "sms", "unavailable", Some("No phone number in notification settings")).await?;
        } else {
            Q::new(
                "INSERT INTO notification_deliveries (notification_id, user_id, channel, next_attempt_at)
                 VALUES ($1, $2, 'sms', $3::timestamptz) ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(n.user_id)
            .bind(&send_at)
            .run(&mut *db)
            .await?;
        }
    }
    Ok(Some(id))
}

/// Sends queued push and SMS deliveries that are due. Returns how many were accepted.
pub async fn deliver_due(pool: &PgPool, config: &Config) -> Result<usize> {
    let mut db = pool.acquire().await?;
    Q::new(
        "UPDATE notification_deliveries SET status = 'queued', locked_at = NULL
          WHERE status = 'sending' AND locked_at < now() - interval '10 minutes'",
    )
    .run(&mut db)
    .await?;
    let mut sent = 0;
    loop {
        let claimed = Q::new(
            "UPDATE notification_deliveries d SET status = 'sending', locked_at = now(), attempts = attempts + 1
              WHERE d.id = (SELECT id FROM notification_deliveries
                             WHERE status = 'queued' AND channel IN ('push', 'sms') AND next_attempt_at <= now()
                             ORDER BY next_attempt_at, id LIMIT 1 FOR UPDATE SKIP LOCKED)
              RETURNING d.id, d.channel, d.attempts, d.notification_id, d.subscription_id",
        )
        .opt(&mut db)
        .await?;
        let Some(d) = claimed else { break };
        let id = d["id"].as_i64().unwrap();
        let detail = Q::new(
            "SELECT n.id, n.title, n.body, n.link, s.endpoint, s.p256dh, s.auth, ns.phone
               FROM notifications n
               LEFT JOIN push_subscriptions s ON s.id = $2
               LEFT JOIN notification_settings ns ON ns.user_id = n.user_id
              WHERE n.id = $1",
        )
        .bind(d["notification_id"].as_i64())
        .bind(uuid_of(&d["subscription_id"]))
        .one(&mut db)
        .await?;
        let s = |v: &Value| v.as_str().unwrap_or("").to_string();
        let outcome = match (d["channel"].as_str(), &config.push) {
            (Some("push"), Some(push)) if !detail["endpoint"].is_null() => {
                let payload = json!({
                    "title": detail["title"], "body": detail["body"],
                    "url": detail["link"].as_str().unwrap_or("/"), "tag": format!("dt-{}", detail["id"]),
                });
                crate::push::send(
                    push,
                    &s(&detail["endpoint"]),
                    &s(&detail["p256dh"]),
                    &s(&detail["auth"]),
                    payload.to_string().as_bytes(),
                )
                .await
            }
            (Some("push"), _) => Err(PushError::Gone("The browser subscription or push setup was removed".into())),
            (_, _) if detail["phone"].is_null() => Err(PushError::Gone("No phone number".into())),
            _ => {
                let text = format!("{}: {}", s(&detail["title"]), s(&detail["body"]));
                let text: String = text.chars().take(320).collect();
                config.sms.send(&s(&detail["phone"]), &text).await.map_err(PushError::Retry)
            }
        };
        match outcome {
            Ok(reference) => {
                Q::new(
                    "UPDATE notification_deliveries SET status = 'sent', sent_at = now(), locked_at = NULL,
                            provider_ref = $2, last_error = NULL WHERE id = $1",
                )
                .bind(id)
                .bind(reference)
                .run(&mut db)
                .await?;
                if let Some(sub) = uuid_of(&d["subscription_id"]) {
                    Q::new("UPDATE push_subscriptions SET last_success_at = now() WHERE id = $1")
                        .bind(sub)
                        .run(&mut db)
                        .await?;
                }
                sent += 1;
            }
            Err(PushError::Gone(error)) => {
                Q::new("UPDATE notification_deliveries SET status = 'failed', locked_at = NULL, last_error = $2 WHERE id = $1")
                    .bind(id)
                    .bind(&error)
                    .run(&mut db)
                    .await?;
                // A browser that unsubscribed is forgotten, so it is not tried again.
                if let Some(sub) = uuid_of(&d["subscription_id"]).filter(|_| error.contains("expired")) {
                    Q::new("DELETE FROM push_subscriptions WHERE id = $1").bind(sub).run(&mut db).await?;
                }
            }
            Err(PushError::Retry(error)) => {
                let attempts = d["attempts"].as_i64().unwrap_or(1);
                // Retry with backoff: 1, 4, 9, 16, 25 minutes, then give up.
                Q::new(
                    "UPDATE notification_deliveries SET locked_at = NULL, last_error = $2,
                            status = CASE WHEN $3 >= $4 THEN 'failed' ELSE 'queued' END,
                            next_attempt_at = now() + make_interval(mins => $3::int * $3::int)
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

/// The delivery log of one notification: each channel's status, attempts and last error. Email
/// rows report what the email outbox recorded.
pub const DELIVERIES_SQL: &str = r#"
  SELECT COALESCE(json_agg(json_build_object(
           'channel', d.channel,
           'status', CASE WHEN d.channel = 'email' THEN CASE e.status WHEN 'sending' THEN 'queued' ELSE COALESCE(e.status, d.status) END
                          WHEN d.status = 'sending' THEN 'queued' ELSE d.status END,
           'attempts', CASE WHEN d.channel = 'email' THEN COALESCE(e.attempts, d.attempts) ELSE d.attempts END,
           'error', CASE WHEN d.channel = 'email' THEN e.last_error ELSE d.last_error END,
           'sentAt', CASE WHEN d.channel = 'email' THEN e.sent_at ELSE d.sent_at END,
           'nextAttemptAt', CASE WHEN d.channel = 'email' THEN e.next_attempt_at ELSE d.next_attempt_at END)
         ORDER BY d.id), '[]')
    FROM notification_deliveries d LEFT JOIN email_outbox e ON e.id = d.email_id
   WHERE d.notification_id = n.id"#;
