use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::User;
use crate::db::Q;
use crate::error::Result;
use crate::AppState;

pub async fn summary(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    crate::routes::plans::require_feature(&mut db, user.org_id, "reports").await?;
    let org = user.org_id;
    let q = |sql: &str| Q::new(sql.to_string()).bind(org);
    let by_status = q("SELECT status AS key, count(*)::int AS count FROM assets WHERE org_id = $1 GROUP BY status ORDER BY count DESC")
        .all(&mut db).await?;
    let by_type = q("SELECT type AS key, count(*)::int AS count FROM assets WHERE org_id = $1 AND status <> 'Retired' GROUP BY type ORDER BY count DESC")
        .all(&mut db).await?;
    let by_building = q("SELECT COALESCE(b.name, 'No building') AS key, count(*)::int AS count
                           FROM assets a LEFT JOIN buildings b ON b.id = a.building_id
                          WHERE a.org_id = $1 AND a.status <> 'Retired' GROUP BY 1 ORDER BY count DESC")
    .all(&mut db)
    .await?;
    let assignment = q("SELECT count(*) FILTER (WHERE assigned_person_id IS NOT NULL)::int AS assigned,
                               count(*) FILTER (WHERE assigned_person_id IS NULL)::int AS unassigned
                          FROM assets WHERE org_id = $1 AND status <> 'Retired'")
    .one(&mut db)
    .await?;
    let warranty =
        q(r#"SELECT a.id, a.asset_tag AS "assetTag", a.name, a.type, a.warranty_expires AS "warrantyExpires",
                               (a.warranty_expires < current_date) AS expired
                          FROM assets a WHERE a.org_id = $1 AND a.status <> 'Retired'
                           AND a.warranty_expires IS NOT NULL AND a.warranty_expires < current_date + 90
                         ORDER BY a.warranty_expires LIMIT 100"#)
        .all(&mut db)
        .await?;
    let maintenance = q(r#"SELECT count(*) FILTER (WHERE status IN ('open', 'in_progress', 'waiting_parts'))::int AS active,
                                  count(*) FILTER (WHERE status IN ('open', 'in_progress', 'waiting_parts') AND due_date < current_date)::int AS overdue,
                                  count(*) FILTER (WHERE status = 'resolved' AND closed_at > now() - interval '30 days')::int AS "resolved30d",
                                  round(avg(EXTRACT(epoch FROM closed_at - opened_at) / 86400) FILTER (WHERE status = 'resolved')::numeric, 1)::float AS "avgDaysToResolve"
                             FROM work_orders WHERE org_id = $1"#)
        .one(&mut db).await?;
    let cost = q("SELECT to_char(m, 'YYYY-MM') AS month, COALESCE(sum(w.cost), 0)::float AS cost, count(w.id)::int AS resolved
                    FROM generate_series(date_trunc('month', now()) - interval '11 months', date_trunc('month', now()), interval '1 month') m
                    LEFT JOIN work_orders w ON w.org_id = $1 AND w.status = 'resolved' AND date_trunc('month', w.closed_at) = m
                   GROUP BY m ORDER BY m")
        .all(&mut db).await?;
    let audits = q(r#"SELECT id, period, name, closed_at AS "closedAt", summary FROM audit_runs
                       WHERE org_id = $1 AND status IN ('closed', 'approved') ORDER BY closed_at DESC LIMIT 12"#)
    .all(&mut db)
    .await?;
    let still_missing = q(r#"SELECT DISTINCT ON (i.asset_id) a.id, a.asset_tag AS "assetTag", a.name, r.name AS audit, r.closed_at AS "closedAt"
                               FROM audit_items i JOIN audit_runs r ON r.id = i.run_id JOIN assets a ON a.id = i.asset_id
                              WHERE i.org_id = $1 AND r.status IN ('closed', 'approved') AND i.result = 'missing'
                                AND NOT EXISTS (SELECT 1 FROM audit_items j JOIN audit_runs r2 ON r2.id = j.run_id
                                                 WHERE j.asset_id = i.asset_id AND j.result IN ('verified', 'wrong_location')
                                                   AND r2.status IN ('closed', 'approved') AND r2.closed_at > r.closed_at)
                              ORDER BY i.asset_id, r.closed_at DESC"#)
        .all(&mut db).await?;
    Ok(Json(json!({
        "byStatus": by_status, "byType": by_type, "byBuilding": by_building, "assignment": assignment,
        "warranty": warranty, "maintenance": maintenance, "maintenanceCostByMonth": cost,
        "recentAudits": audits, "stillMissing": still_missing,
    })))
}
