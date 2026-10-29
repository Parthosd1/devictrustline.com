//! Reminders people set for themselves, and the persisted jobs that deliver them.
//!
//! `sync` keeps `reminder_jobs` in step with each active reminder: one job per "days before"
//! offset, firing at the person's send hour in their time zone, plus one for a snooze. Jobs are
//! keyed by (reminder, due date, offset), so editing a reminder cancels jobs that no longer apply
//! and never recreates one that already fired; a cancelled job comes back if it applies again
//! (a paused reminder resumed). Offsets that fell inside a snooze stay quiet. Reminders that follow an item's date pick up
//! changes to that date, and recurring reminders roll forward once their date has passed.
//! `fire` claims due jobs with `FOR UPDATE SKIP LOCKED`, delivers through `notify::send`, and
//! retries a failed job with backoff before marking it failed. Both are safe on several servers.

use chrono::{Days, Months, NaiveDate};
use serde_json::Value;
use sqlx::PgPool;

use crate::config::Config;
use crate::db::{uuid_of, Q};
use crate::error::Result;
use crate::notify::{self, Notice};

pub const KINDS: &[&str] = &["warranty", "return", "maintenance", "renewal", "inspection", "loan_return", "custom"];
pub const REPEATS: &[&str] = &["none", "daily", "weekly", "monthly", "yearly"];
pub const DATE_SOURCES: &[&str] = &["warranty", "return_deadline", "maintenance_due", "loan_due"];
const MAX_ATTEMPTS: i64 = 5;

/// The item column a reminder can follow.
pub fn source_column(source: &str) -> &'static str {
    match source {
        "warranty" => "warranty_expires",
        "return_deadline" => "return_deadline",
        "maintenance_due" => "maintenance_due",
        _ => "loan_due",
    }
}

/// The `step`-th repetition after `base`.
fn step(base: NaiveDate, repeat: &str, every: u32, n: u32) -> Option<NaiveDate> {
    let k = every.checked_mul(n)?;
    match repeat {
        "daily" => base.checked_add_days(Days::new(k as u64)),
        "weekly" => base.checked_add_days(Days::new(k as u64 * 7)),
        "monthly" => base.checked_add_months(Months::new(k)),
        "yearly" => base.checked_add_months(Months::new(k.checked_mul(12)?)),
        _ => None,
    }
}

/// The first repetition of `base` on or after `from` (strictly after `base`). Counted from `base`
/// each time, so the 31st stays the 31st in months that have one.
pub fn next_due(base: NaiveDate, repeat: &str, every: u32, from: NaiveDate) -> Option<NaiveDate> {
    (1..=100_000).map(|n| step(base, repeat, every, n)).take_while(Option::is_some).flatten().find(|d| *d >= from)
}

fn date(v: &Value) -> Option<NaiveDate> {
    v.as_str().and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

/// Brings reminder dates and jobs up to date. Returns (jobs created, jobs cancelled).
pub async fn sync(pool: &PgPool) -> Result<(u64, u64)> {
    let mut tx = pool.begin().await?;
    let locked: bool = Q::new("SELECT pg_try_advisory_xact_lock(727276)").scalar(&mut tx).await?;
    if !locked {
        return Ok((0, 0));
    }
    // Reminders that follow an item's date.
    Q::new(
        "UPDATE user_reminders r SET due_on = d.date, updated_at = now()
           FROM (SELECT r2.id, CASE r2.date_source WHEN 'warranty' THEN a.warranty_expires
                                                  WHEN 'return_deadline' THEN a.return_deadline
                                                  WHEN 'maintenance_due' THEN a.maintenance_due
                                                  ELSE a.loan_due END AS date
                   FROM user_reminders r2 JOIN assets a ON a.id = r2.asset_id
                  WHERE r2.date_source IS NOT NULL AND r2.status <> 'completed') d
          WHERE r.id = d.id AND d.date IS NOT NULL AND d.date <> r.due_on",
    )
    .run(&mut tx)
    .await?;
    // Recurring reminders whose date has passed move to their next date.
    let past = Q::new(
        "SELECT r.id, r.due_on, r.repeat, r.repeat_every, r.repeat_until,
                (now() AT TIME ZONE COALESCE(s.timezone, o.timezone))::date AS today
           FROM user_reminders r JOIN organizations o ON o.id = r.org_id
           LEFT JOIN notification_settings s ON s.user_id = r.user_id
          WHERE r.status = 'active' AND r.repeat <> 'none' AND r.date_source IS NULL
            AND r.due_on < (now() AT TIME ZONE COALESCE(s.timezone, o.timezone))::date",
    )
    .all(&mut tx)
    .await?;
    for r in past {
        let (Some(due), Some(today)) = (date(&r["due_on"]), date(&r["today"])) else { continue };
        let every = r["repeat_every"].as_u64().unwrap_or(1) as u32;
        let next = next_due(due, r["repeat"].as_str().unwrap_or(""), every, today);
        let until = date(&r["repeat_until"]);
        match next.filter(|n| until.is_none_or(|u| *n <= u)) {
            Some(n) => Q::new("UPDATE user_reminders SET due_on = $2::date, snoozed_until = NULL, updated_at = now() WHERE id = $1")
                .bind(uuid_of(&r["id"]))
                .bind(n.to_string()),
            None => Q::new(
                "UPDATE user_reminders SET status = 'completed', completed_at = now(), updated_at = now() WHERE id = $1",
            )
            .bind(uuid_of(&r["id"])),
        }
        .run(&mut tx)
        .await?;
    }
    let desired = "WITH r AS (
           SELECT r.id, r.org_id, r.due_on, r.offsets, r.snoozed_until,
                  COALESCE(s.timezone, o.timezone) AS tz, COALESCE(s.send_hour, 9) AS send_hour
             FROM user_reminders r
             JOIN organizations o ON o.id = r.org_id
             JOIN memberships m ON m.org_id = r.org_id AND m.user_id = r.user_id AND m.is_active
             LEFT JOIN notification_settings s ON s.user_id = r.user_id
            WHERE r.status = 'active'
         ),
         offs AS (
           SELECT r.id AS reminder_id, r.org_id, r.due_on, n AS days_before, r.snoozed_until,
                  ((r.due_on - n) + make_time(r.send_hour, 0, 0)) AT TIME ZONE r.tz AS fire_at,
                  r.id || ':' || r.due_on || ':' || n AS dedupe_key,
                  (now() AT TIME ZONE r.tz)::date AS today
             FROM r CROSS JOIN LATERAL unnest(r.offsets) AS n
         ),
         desired AS (
           SELECT reminder_id, org_id, due_on, days_before, fire_at, dedupe_key FROM offs
            WHERE due_on - days_before >= today AND (snoozed_until IS NULL OR fire_at > snoozed_until)
           UNION ALL
           SELECT id, org_id, due_on, NULL, snoozed_until, id || ':snooze:' || extract(epoch FROM snoozed_until)::bigint
             FROM r WHERE snoozed_until > now() - interval '1 day'
         )";
    let created = Q::new(format!(
        "{desired}
         INSERT INTO reminder_jobs (org_id, reminder_id, dedupe_key, due_on, days_before, fire_at)
         SELECT org_id, reminder_id, dedupe_key, due_on, days_before, fire_at FROM desired
         ON CONFLICT (dedupe_key) DO UPDATE SET status = 'pending', fire_at = EXCLUDED.fire_at
          WHERE reminder_jobs.status = 'cancelled'"
    ))
    .run(&mut tx)
    .await?;
    let cancelled = Q::new(format!(
        "{desired}
         UPDATE reminder_jobs j SET status = 'cancelled'
          WHERE j.status = 'pending' AND NOT EXISTS (SELECT 1 FROM desired d WHERE d.dedupe_key = j.dedupe_key)"
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
        -1 => "yesterday".into(),
        n if n < 0 => format!("{} days ago", -n),
        n => format!("in {n} days"),
    }
}

/// Delivers due jobs. Returns how many were delivered.
pub async fn fire(pool: &PgPool, config: &Config) -> Result<usize> {
    let mut fired = 0;
    loop {
        let mut tx = pool.begin().await?;
        let job = Q::new(
            "SELECT j.id, j.attempts, j.due_on, r.id AS reminder_id, r.org_id, r.user_id, r.asset_id, r.kind, r.title, r.notes,
                    r.status, a.asset_tag,
                    j.due_on - (now() AT TIME ZONE COALESCE(s.timezone, o.timezone))::date AS days_left
               FROM reminder_jobs j
               JOIN user_reminders r ON r.id = j.reminder_id
               JOIN organizations o ON o.id = r.org_id
               LEFT JOIN notification_settings s ON s.user_id = r.user_id
               LEFT JOIN assets a ON a.id = r.asset_id
              WHERE j.status = 'pending' AND j.fire_at <= now()
              ORDER BY j.fire_at, j.id LIMIT 1 FOR UPDATE OF j SKIP LOCKED",
        )
        .opt(&mut tx)
        .await?;
        let Some(job) = job else { break };
        let job_id = job["id"].as_i64().unwrap();
        if job["status"] != "active" {
            Q::new("UPDATE reminder_jobs SET status = 'cancelled' WHERE id = $1").bind(job_id).run(&mut tx).await?;
            tx.commit().await?;
            continue;
        }
        let title = job["title"].as_str().unwrap_or("Reminder").to_string();
        let mut body = format!(
            "{title} is due {} ({}).",
            when(job["days_left"].as_i64().unwrap_or(0)),
            job["due_on"].as_str().unwrap_or("")
        );
        if let Some(notes) = job["notes"].as_str() {
            body.push_str(&format!(" {notes}"));
        }
        let link = job["asset_tag"].as_str().map(|t| format!("/?asset={t}"));
        let notice = Notice {
            asset_id: uuid_of(&job["asset_id"]),
            user_reminder_id: uuid_of(&job["reminder_id"]),
            dedupe_key: Some(format!("reminder-job:{job_id}")),
            link,
            email_kind: "reminder",
            ..Notice::new(
                uuid_of(&job["org_id"]).unwrap(),
                uuid_of(&job["user_id"]).unwrap(),
                job["kind"].as_str().unwrap_or("custom"),
                &title,
                &body,
            )
        };
        match notify::send(&mut tx, config, notice).await {
            Ok(notification) => {
                Q::new(
                    "UPDATE reminder_jobs SET status = CASE WHEN $2::bigint IS NULL THEN 'skipped' ELSE 'sent' END,
                            notification_id = $2, fired_at = now() WHERE id = $1",
                )
                .bind(job_id)
                .bind(notification)
                .run(&mut tx)
                .await?;
                tx.commit().await?;
                fired += 1;
            }
            Err(e) => {
                tx.rollback().await?;
                let attempts = job["attempts"].as_i64().unwrap_or(0) + 1;
                let mut db = pool.acquire().await?;
                Q::new(
                    "UPDATE reminder_jobs SET attempts = $2, last_error = $3,
                            status = CASE WHEN $2 >= $4 THEN 'failed' ELSE status END,
                            fire_at = now() + make_interval(mins => $2::int * $2::int)
                      WHERE id = $1",
                )
                .bind(job_id)
                .bind(attempts)
                .bind(format!("{e:?}"))
                .bind(MAX_ATTEMPTS)
                .run(&mut db)
                .await?;
            }
        }
    }
    Ok(fired)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn recurring_dates_roll_forward_without_drifting() {
        assert_eq!(next_due(d("2026-01-31"), "monthly", 1, d("2026-02-01")), Some(d("2026-02-28")));
        assert_eq!(next_due(d("2026-01-31"), "monthly", 1, d("2026-03-01")), Some(d("2026-03-31")));
        assert_eq!(next_due(d("2026-01-01"), "weekly", 2, d("2026-01-10")), Some(d("2026-01-15")));
        assert_eq!(next_due(d("2024-02-29"), "yearly", 1, d("2025-01-01")), Some(d("2025-02-28")));
        assert_eq!(next_due(d("2026-01-01"), "daily", 1, d("2026-01-01")), Some(d("2026-01-02")));
        assert_eq!(next_due(d("2026-01-01"), "none", 1, d("2026-01-02")), None);
    }

    #[test]
    fn days_read_naturally() {
        assert_eq!(when(0), "today");
        assert_eq!(when(1), "tomorrow");
        assert_eq!(when(14), "in 14 days");
        assert_eq!(when(-3), "3 days ago");
    }
}
