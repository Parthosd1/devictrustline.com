use std::time::Duration;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::{uuid_of, Q};
use crate::error::Result;
use crate::routes::audits::{start_audit, NewAudit};

/// Opens this week's and this month's organization-wide audits for every organization that turned
/// automatic audits on, unless one was already started in the current week or month.
/// Safe to run often and from several servers at once.
pub async fn run_scheduled_audits(pool: &PgPool, log: bool) -> Result<Vec<Value>> {
    let mut db = pool.acquire().await?;
    let orgs = Q::new(
        "SELECT id, settings->'autoAudits' AS auto FROM organizations
          WHERE settings->'autoAudits'->>'weekly' = 'true' OR settings->'autoAudits'->>'monthly' = 'true'",
    )
    .all(&mut db)
    .await?;
    drop(db);
    let mut started = vec![];
    for org in orgs {
        let org_id = uuid_of(&org["id"]).unwrap();
        for (period, unit) in [("weekly", "week"), ("monthly", "month")] {
            if org["auto"][period] != true {
                continue;
            }
            let mut tx = pool.begin().await?;
            Q::new("SELECT pg_advisory_xact_lock(hashtext($1))")
                .bind(format!("audit-schedule:{org_id}:{period}"))
                .run(&mut tx)
                .await?;
            let existing = Q::new(
                "SELECT 1 FROM audit_runs
                  WHERE org_id = $1 AND period = $2 AND building_id IS NULL
                    AND (status = 'open' OR started_at >= date_trunc($3, now()))
                  LIMIT 1",
            )
            .bind(org_id)
            .bind(period)
            .bind(unit)
            .run(&mut tx)
            .await?;
            if existing > 0 {
                continue;
            }
            // Scheduled audits are attributed to the organization's longest-standing active admin.
            let admin: Option<Uuid> = Q::new(
                "SELECT user_id FROM memberships WHERE org_id = $1 AND role = 'admin' AND is_active ORDER BY created_at LIMIT 1",
            )
            .bind(org_id)
            .scalars(&mut tx)
            .await?
            .into_iter()
            .next();
            let Some(admin) = admin else { continue };
            let new = NewAudit {
                org_id,
                user_id: admin,
                period,
                building_id: None,
                location_id: None,
                name: None,
                due_at: None,
                scheduled: true,
            };
            let id = start_audit(&mut tx, new).await?;
            tx.commit().await?;
            if log {
                println!("scheduled {period} audit {id} for organization {org_id}");
            }
            started.push(json!({ "orgId": org_id, "period": period, "id": id }));
        }
    }
    Ok(started)
}

/// Background jobs that run every few seconds on every server: email delivery (and, later,
/// reminders). Each job claims its rows with SKIP LOCKED, so instances never duplicate work.
pub async fn run_jobs(pool: &PgPool, config: &Config) -> Result<Value> {
    let emails = crate::mail::deliver_due(pool, &config.mail).await?;
    Ok(json!({ "emailsSent": emails }))
}

pub fn start(pool: PgPool, config: Config) {
    let jobs_pool = pool.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(10));
        loop {
            every.tick().await;
            if let Err(e) = run_jobs(&jobs_pool, &config).await {
                eprintln!("background jobs failed: {e:?}");
            }
        }
    });
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut every = tokio::time::interval(Duration::from_secs(15 * 60));
        loop {
            every.tick().await;
            if let Err(e) = run_scheduled_audits(&pool, true).await {
                eprintln!("audit scheduler failed: {e:?}");
            }
        }
    });
}
