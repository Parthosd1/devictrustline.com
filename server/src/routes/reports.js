import { Router } from 'express';
import { query } from '../db/pool.js';
import { route } from '../lib/http.js';
import { requirePermission } from '../middleware/auth.js';

export const reportsRouter = Router();

reportsRouter.get('/summary', requirePermission('assets:read'), route(async (req, res) => {
  const org = [req.user.org_id];
  const [byStatus, byType, byBuilding, assignment, warranty, maintenance, cost, audits, lastMissing] = await Promise.all([
    query(`SELECT status AS key, count(*)::int AS count FROM assets WHERE org_id = $1 GROUP BY status ORDER BY count DESC`, org),
    query(`SELECT type AS key, count(*)::int AS count FROM assets WHERE org_id = $1 AND status <> 'Retired' GROUP BY type ORDER BY count DESC`, org),
    query(`SELECT COALESCE(b.name, 'No building') AS key, count(*)::int AS count
             FROM assets a LEFT JOIN buildings b ON b.id = a.building_id
            WHERE a.org_id = $1 AND a.status <> 'Retired' GROUP BY 1 ORDER BY count DESC`, org),
    query(`SELECT count(*) FILTER (WHERE assigned_person_id IS NOT NULL)::int AS assigned,
                  count(*) FILTER (WHERE assigned_person_id IS NULL)::int AS unassigned
             FROM assets WHERE org_id = $1 AND status <> 'Retired'`, org),
    query(`SELECT a.id, a.asset_tag AS "assetTag", a.name, a.type, a.warranty_expires AS "warrantyExpires",
                  (a.warranty_expires < current_date) AS expired
             FROM assets a WHERE a.org_id = $1 AND a.status <> 'Retired'
              AND a.warranty_expires IS NOT NULL AND a.warranty_expires < current_date + 90
            ORDER BY a.warranty_expires LIMIT 100`, org),
    query(`SELECT count(*) FILTER (WHERE status IN ('open', 'in_progress', 'waiting_parts'))::int AS active,
                  count(*) FILTER (WHERE status IN ('open', 'in_progress', 'waiting_parts') AND due_date < current_date)::int AS overdue,
                  count(*) FILTER (WHERE status = 'resolved' AND closed_at > now() - interval '30 days')::int AS "resolved30d",
                  round(avg(EXTRACT(epoch FROM closed_at - opened_at) / 86400) FILTER (WHERE status = 'resolved')::numeric, 1)::float AS "avgDaysToResolve"
             FROM work_orders WHERE org_id = $1`, org),
    query(`SELECT to_char(m, 'YYYY-MM') AS month, COALESCE(sum(w.cost), 0)::float AS cost, count(w.id)::int AS resolved
             FROM generate_series(date_trunc('month', now()) - interval '11 months', date_trunc('month', now()), interval '1 month') m
             LEFT JOIN work_orders w ON w.org_id = $1 AND w.status = 'resolved' AND date_trunc('month', w.closed_at) = m
            GROUP BY m ORDER BY m`, org),
    query(`SELECT id, period, name, closed_at AS "closedAt", summary FROM audit_runs
            WHERE org_id = $1 AND status = 'closed' ORDER BY closed_at DESC LIMIT 12`, org),
    query(`SELECT DISTINCT ON (i.asset_id) a.id, a.asset_tag AS "assetTag", a.name, r.name AS audit, r.closed_at AS "closedAt"
             FROM audit_items i JOIN audit_runs r ON r.id = i.run_id JOIN assets a ON a.id = i.asset_id
            WHERE i.org_id = $1 AND r.status = 'closed' AND i.result = 'missing'
              AND NOT EXISTS (SELECT 1 FROM audit_items j JOIN audit_runs r2 ON r2.id = j.run_id
                               WHERE j.asset_id = i.asset_id AND j.result IN ('verified', 'wrong_location')
                                 AND r2.status = 'closed' AND r2.closed_at > r.closed_at)
            ORDER BY i.asset_id, r.closed_at DESC`, org),
  ]);
  res.json({
    byStatus: byStatus.rows, byType: byType.rows, byBuilding: byBuilding.rows, assignment: assignment.rows[0],
    warranty: warranty.rows, maintenance: maintenance.rows[0], maintenanceCostByMonth: cost.rows,
    recentAudits: audits.rows, stillMissing: lastMissing.rows,
  });
}));
