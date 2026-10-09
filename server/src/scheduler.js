import { query, withTransaction } from './db/pool.js';
import { startAudit } from './routes/audits.js';

const PERIOD_UNIT = { weekly: 'week', monthly: 'month' };

// Opens this week's and this month's organization-wide audits for every organization that turned
// automatic audits on, unless one was already started in the current week or month.
// Safe to run often and from several servers at once.
export async function runScheduledAudits({ log = console.log } = {}) {
  const { rows: orgs } = await query(
    `SELECT id, settings->'autoAudits' AS auto FROM organizations
      WHERE settings->'autoAudits'->>'weekly' = 'true' OR settings->'autoAudits'->>'monthly' = 'true'`,
  );
  const started = [];
  for (const org of orgs) {
    for (const period of ['weekly', 'monthly']) {
      if (org.auto?.[period] !== true) continue;
      const id = await withTransaction(async (db) => {
        await db.query('SELECT pg_advisory_xact_lock(hashtext($1))', [`audit-schedule:${org.id}:${period}`]);
        const { rows: [existing] } = await db.query(
          `SELECT 1 FROM audit_runs
            WHERE org_id = $1 AND period = $2 AND building_id IS NULL
              AND (status = 'open' OR started_at >= date_trunc($3, now()))
            LIMIT 1`,
          [org.id, period, PERIOD_UNIT[period]],
        );
        if (existing) return null;
        // Scheduled audits are attributed to the organization's longest-standing active admin.
        const { rows: [admin] } = await db.query(
          `SELECT id FROM users WHERE org_id = $1 AND role = 'admin' AND is_active ORDER BY created_at LIMIT 1`, [org.id],
        );
        if (!admin) return null;
        return startAudit(db, { orgId: org.id, userId: admin.id, period, scheduled: true });
      });
      if (id) {
        started.push({ orgId: org.id, period, id });
        log(`scheduled ${period} audit ${id} for organization ${org.id}`);
      }
    }
  }
  return started;
}

export function startScheduler({ intervalMs = 15 * 60 * 1000 } = {}) {
  const tick = () => runScheduledAudits().catch((err) => console.error('audit scheduler failed:', err.message));
  const first = setTimeout(tick, 5000);
  const timer = setInterval(tick, intervalMs);
  timer.unref();
  first.unref();
  return () => { clearTimeout(first); clearInterval(timer); };
}
