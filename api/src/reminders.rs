//! Reminders and the in-app notification center.
//!
//! Reminders are derived from the data: warranty end dates, return windows, maintenance due dates
//! (on items and open work orders), open audits and loans. `sync` keeps the `reminders` table in
//! step with that data and `fire` delivers the ones that are due. Both run on every server every
//! few seconds and are safe to run concurrently:
//! - each reminder has a dedupe key (kind, source, deadline, days before), so it exists once;
//! - a changed or removed deadline cancels pending reminders for the old one;
//! - firing claims rows with `FOR UPDATE SKIP LOCKED` inside a transaction, so a reminder is
//!   delivered once, and a server that stops mid-way leaves it pending for the next run;
//! - "today" is each workspace's own date in its time zone.

use serde_json::{json, Value};
use sqlx::PgPool;

use crate::config::Config;
use crate::db::{uuid_of, Q};
use crate::error::Result;
use crate::notify::{self, Notice};

/// Default days-before lists, overridable per workspace in settings.reminders.
pub const DEFAULT_DAYS: &[(&str, &str, &[i64])] = &[
    ("warranty", "warrantyDays", &[90, 30, 7, 1]),
    ("return_window", "returnDays", &[7, 1]),
    ("maintenance", "maintenanceDays", &[7, 1]),
    ("audit", "auditDays", &[3, 1]),
    ("loan", "loanDays", &[3, 1, 0]),
];

/// Every upcoming deadline in every workspace: (org_id, asset_id, kind, source_id, due_on, title).
pub const DEADLINES: &str = r#"
  SELECT w.org_id, w.asset_id, 'warranty' AS kind, w.id::text AS source_id, w.ends_on AS due_on,
         a.name || ': ' || CASE w.kind WHEN 'extended' THEN 'extended warranty' ELSE 'warranty' END || ' ends' AS title
    FROM warranties w JOIN assets a ON a.id = w.asset_id
   WHERE w.ends_on IS NOT NULL AND a.status <> 'Retired' AND a.archived_at IS NULL
  UNION ALL
  SELECT org_id, id, 'return_window', id::text, return_deadline, name || ': last day to return it'
    FROM assets WHERE return_deadline IS NOT NULL AND status <> 'Retired' AND archived_at IS NULL
  UNION ALL
  SELECT org_id, id, 'maintenance', id::text, maintenance_due, name || ': maintenance due'
    FROM assets WHERE maintenance_due IS NOT NULL AND status <> 'Retired' AND archived_at IS NULL
  UNION ALL
  SELECT wo.org_id, wo.asset_id, 'maintenance', 'wo:' || wo.id, wo.due_date, a.name || ': ' || wo.title || ' due'
    FROM work_orders wo JOIN assets a ON a.id = wo.asset_id
   WHERE wo.due_date IS NOT NULL AND wo.status NOT IN ('resolved', 'cancelled')
  UNION ALL
  SELECT r.org_id, NULL, 'audit', r.id::text, (r.due_at AT TIME ZONE o.timezone)::date, r.name || ': audit due'
    FROM audit_runs r JOIN organizations o ON o.id = r.org_id WHERE r.status = 'open'
  UNION ALL
  SELECT a.org_id, a.id, 'loan', a.id::text, a.loan_due, a.name || ': due back from ' || p.name
    FROM assets a JOIN people p ON p.id = a.assigned_person_id WHERE a.loan_due IS NOT NULL"#;

fn desired_sql() -> String {
    let defaults: Vec<String> = DEFAULT_DAYS
        .iter()
        .map(|(kind, key, days)| format!("('{kind}', '{key}', '{}'::jsonb)", serde_json::to_string(days).unwrap()))
        .collect();
    format!(
        "WITH cfg(kind, setting, days) AS (VALUES {}),
         deadlines AS ({DEADLINES}),
         desired AS (
           SELECT d.org_id, d.asset_id, d.kind, d.source_id, d.due_on, d.title, n::int AS days_before,
                  d.due_on - n::int AS remind_on, (now() AT TIME ZONE o.timezone)::date AS today,
                  d.kind || ':' || d.source_id || ':' || d.due_on || ':' || n AS dedupe_key
             FROM deadlines d
             JOIN organizations o ON o.id = d.org_id
             JOIN cfg c ON c.kind = d.kind
            CROSS JOIN LATERAL jsonb_array_elements_text(
                  CASE WHEN jsonb_typeof(o.settings->'reminders'->c.setting) = 'array'
                       THEN o.settings->'reminders'->c.setting ELSE c.days END) AS n
         )",
        defaults.join(", ")
    )
}

/// Brings the reminders table in line with the current deadlines. Returns (created, cancelled).
pub async fn sync(pool: &PgPool) -> Result<(u64, u64)> {
    let mut tx = pool.begin().await?;
    // One server syncs at a time; the others skip this round.
    let locked: bool = Q::new("SELECT pg_try_advisory_xact_lock(727275)").scalar(&mut tx).await?;
    if !locked {
        return Ok((0, 0));
    }
    let desired = desired_sql();
    let created = Q::new(format!(
        "{desired}
         INSERT INTO reminders (org_id, asset_id, kind, source_id, due_on, remind_on, days_before, title, dedupe_key)
         SELECT org_id, asset_id, kind, source_id, due_on, remind_on, days_before, title, dedupe_key
           FROM desired WHERE remind_on >= today
         ON CONFLICT (dedupe_key) DO NOTHING"
    ))
    .run(&mut tx)
    .await?;
    let cancelled = Q::new(format!(
        "{desired}
         UPDATE reminders r SET status = 'cancelled'
          WHERE r.status = 'pending' AND NOT EXISTS (SELECT 1 FROM desired d WHERE d.dedupe_key = r.dedupe_key)"
    ))
    .run(&mut tx)
    .await?;
    tx.commit().await?;
    Ok((created, cancelled))
}

fn when(days: i64) -> String {
    match days {
        0 => "today".into(),
        1 => "tomorrow".into(),
        n => format!("in {n} days"),
    }
}

/// Delivers due reminders: an in-app notification for each recipient, plus the channels each
/// recipient turned on. Returns how many reminders were delivered.
pub async fn fire(pool: &PgPool, config: &Config) -> Result<usize> {
    let mut fired = 0;
    loop {
        let mut tx = pool.begin().await?;
        let claimed = Q::new(
            "WITH next AS (
               SELECT r.id, (now() AT TIME ZONE o.timezone)::date AS today
                 FROM reminders r JOIN organizations o ON o.id = r.org_id
                WHERE r.status = 'pending' AND r.remind_on <= (now() AT TIME ZONE o.timezone)::date
                ORDER BY r.remind_on, r.id LIMIT 1 FOR UPDATE OF r SKIP LOCKED
             )
             UPDATE reminders r SET status = CASE WHEN r.due_on < next.today THEN 'missed' ELSE 'sent' END, fired_at = now()
               FROM next WHERE r.id = next.id
             RETURNING r.id, r.org_id, r.asset_id, r.kind, r.title, r.due_on, r.status, r.due_on - next.today AS days_left",
        )
        .opt(&mut tx)
        .await?;
        let Some(r) = claimed else { break };
        // A deadline that already passed (the server was down) is recorded, not announced late.
        if r["status"] == "sent" {
            deliver(&mut tx, config, &r).await?;
        }
        tx.commit().await?;
        fired += 1;
    }
    Ok(fired)
}

/// The kind of personal reminder that covers the same deadline as a workspace reminder.
fn personal_kind(kind: &str) -> &str {
    match kind {
        "return_window" => "return",
        "loan" => "loan_return",
        other => other,
    }
}

async fn deliver(tx: &mut crate::db::Db, config: &Config, r: &Value) -> Result<()> {
    let org = uuid_of(&r["org_id"]).unwrap();
    let asset = uuid_of(&r["asset_id"]);
    // The item's owner, if it has one; otherwise the workspace's admins and managers. Someone who
    // set their own reminder for this item and deadline gets that one instead of this.
    let recipients: Vec<uuid::Uuid> = Q::new(
        "SELECT u.id
           FROM memberships m JOIN users u ON u.id = m.user_id
          WHERE m.org_id = $1 AND m.is_active AND u.is_active
            AND CASE WHEN (SELECT owner_id FROM assets WHERE id = $2) IS NOT NULL
                     THEN m.user_id = (SELECT owner_id FROM assets WHERE id = $2)
                     ELSE m.role IN ('owner', 'admin', 'manager') END
            AND NOT EXISTS (SELECT 1 FROM user_reminders ur
                             WHERE ur.user_id = u.id AND ur.asset_id = $2 AND ur.kind = $3 AND ur.status = 'active')",
    )
    .bind(org)
    .bind(asset)
    .bind(personal_kind(r["kind"].as_str().unwrap_or("")))
    .scalars(&mut *tx)
    .await?;
    let title = r["title"].as_str().unwrap_or("");
    let due = r["due_on"].as_str().unwrap_or("");
    let body = format!("{title} {} ({due}).", when(r["days_left"].as_i64().unwrap_or(0)));
    let tag: Option<String> = match asset {
        Some(a) => {
            Q::new("SELECT asset_tag FROM assets WHERE id = $1").bind(a).scalars(&mut *tx).await?.into_iter().next()
        }
        None => None,
    };
    for user in recipients {
        let notice = Notice {
            asset_id: asset,
            reminder_id: r["id"].as_i64(),
            link: tag.as_ref().map(|t| format!("/?asset={t}")),
            email_kind: "reminder",
            email_dedupe_key: Some(format!("reminder:{}:{user}", r["id"])),
            ..Notice::new(org, user, r["kind"].as_str().unwrap_or(""), title, &body)
        };
        notify::send(&mut *tx, config, notice).await?;
    }
    Ok(())
}

pub fn summary(created: u64, cancelled: u64, fired: usize) -> Value {
    json!({ "remindersCreated": created, "remindersCancelled": cancelled, "remindersFired": fired })
}
