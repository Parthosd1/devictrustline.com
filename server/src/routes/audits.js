import { Router } from 'express';
import { z } from 'zod';
import { query, withTransaction } from '../db/pool.js';
import { HttpError, notFound } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { requirePermission } from '../middleware/auth.js';
import { findByCode, recordEvent } from './assets.js';

const uuid = z.string().uuid();
const PERIODS = ['weekly', 'monthly'];

const startSchema = z.object({
  period: z.enum(PERIODS),
  buildingId: uuid.nullish(),
  locationId: uuid.nullish(),
  name: z.string().trim().min(1).max(160).nullish(),
  dueAt: z.string().datetime({ offset: true }).nullish(),
}).strict();
const listSchema = z.object({
  period: z.enum(PERIODS).optional(),
  status: z.enum(['open', 'closed']).optional(),
  limit: z.coerce.number().int().min(1).max(100).default(25),
});
const scanSchema = z.object({ code: z.string().trim().min(1).max(500), locationId: uuid.nullish() }).strict();
const markSchema = z.object({
  result: z.enum(['verified', 'missing', 'pending']),
  note: z.string().trim().max(500).transform((v) => v || null).nullish(),
}).strict();

const RUN_SELECT = `
  SELECT r.id, r.period, r.name, r.status, r.building_id AS "buildingId", b.name AS "buildingName",
         r.location_id AS "locationId", l.name AS "locationName", r.due_at AS "dueAt",
         r.started_at AS "startedAt", su.name AS "startedBy", r.closed_at AS "closedAt", cu.name AS "closedBy",
         r.summary, r.scheduled,
         (SELECT json_build_object(
            'expected', count(*) FILTER (WHERE i.expected),
            'verified', count(*) FILTER (WHERE i.result = 'verified' AND i.expected),
            'wrongLocation', count(*) FILTER (WHERE i.result = 'wrong_location'),
            'missing', count(*) FILTER (WHERE i.result = 'missing'),
            'pending', count(*) FILTER (WHERE i.result = 'pending'),
            'unexpected', count(*) FILTER (WHERE NOT i.expected))
            FROM audit_items i WHERE i.run_id = r.id) AS counts
    FROM audit_runs r
    LEFT JOIN buildings b ON b.id = r.building_id
    LEFT JOIN locations l ON l.id = r.location_id
    LEFT JOIN users su ON su.id = r.started_by
    LEFT JOIN users cu ON cu.id = r.closed_by`;

const ITEM_SELECT = `
  SELECT i.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name, a.type, a.status AS "assetStatus",
         i.expected, i.result, i.method, i.note, i.checked_at AS "checkedAt", u.name AS "checkedBy",
         el.name AS "expectedLocationName", eb.name AS "expectedBuildingName",
         fl.name AS "foundLocationName", pe.name AS "assignedPersonName"
    FROM audit_items i
    JOIN assets a ON a.id = i.asset_id
    LEFT JOIN locations el ON el.id = i.expected_location_id
    LEFT JOIN buildings eb ON eb.id = i.expected_building_id
    LEFT JOIN locations fl ON fl.id = i.found_location_id
    LEFT JOIN people pe ON pe.id = a.assigned_person_id
    LEFT JOIN users u ON u.id = i.checked_by`;

async function findRun(db, orgId, id, { lock = false } = {}) {
  const { rows: [run] } = await db.query(
    `${RUN_SELECT} WHERE r.org_id = $1 AND r.id = $2${lock ? ' FOR UPDATE OF r' : ''}`, [orgId, id],
  );
  if (!run) throw notFound('Audit');
  return run;
}

async function findItem(db, runId, assetId) {
  const { rows: [item] } = await db.query(`${ITEM_SELECT} WHERE i.run_id = $1 AND i.asset_id = $2`, [runId, assetId]);
  return item;
}

const requireOpen = (run) => {
  if (run.status !== 'open') throw new HttpError(409, 'This audit is closed and can no longer change');
};

function defaultName(period, scopeName) {
  const date = new Date().toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' });
  return `${period === 'weekly' ? 'Weekly' : 'Monthly'} audit · ${date}${scopeName ? ` · ${scopeName}` : ''}`;
}

export const auditsRouter = Router();

auditsRouter.get('/', requirePermission('audits:read'), route(async (req, res) => {
  const f = listSchema.parse(req.query);
  const params = [req.user.org_id];
  const where = ['r.org_id = $1'];
  if (f.period) { params.push(f.period); where.push(`r.period = $${params.length}`); }
  if (f.status) { params.push(f.status); where.push(`r.status = $${params.length}`); }
  const { rows } = await query(
    `${RUN_SELECT} WHERE ${where.join(' AND ')} ORDER BY r.status = 'open' DESC, r.started_at DESC LIMIT ${f.limit}`, params,
  );
  res.json({ items: rows });
}));

// Opens an audit and snapshots every active asset in scope. Used by people and by the scheduler.
export async function startAudit(db, { orgId, userId, period, buildingId = null, locationId = null, name = null, dueAt = null, scheduled = false }) {
  if (locationId && !buildingId) throw new HttpError(400, 'Choose the building for that location');
  let scopeName = null;
  if (buildingId) {
    const { rows: [scope] } = await db.query(
      `SELECT b.name AS building, l.name AS location FROM buildings b
         LEFT JOIN locations l ON l.id = $3 AND l.building_id = b.id
        WHERE b.org_id = $1 AND b.id = $2`,
      [orgId, buildingId, locationId],
    );
    if (!scope || (locationId && !scope.location)) throw new HttpError(400, 'Building or location not found');
    scopeName = scope.location ? `${scope.building} / ${scope.location}` : scope.building;
  }
  const { rows: [{ id }] } = await db.query(
    `INSERT INTO audit_runs (org_id, period, name, building_id, location_id, due_at, started_by, scheduled)
     VALUES ($1, $2, $3, $4, $5,
             COALESCE($6::timestamptz, CASE WHEN $2 = 'weekly' THEN now() + interval '7 days'
                                            ELSE date_trunc('month', now()) + interval '1 month' END),
             $7, $8)
     RETURNING id`,
    [orgId, period, name ?? defaultName(period, scopeName), buildingId, locationId, dueAt, userId, scheduled],
  ).catch((err) => {
    if (err.constraint === 'audit_runs_one_open') {
      throw new HttpError(409, `A ${period} audit is already open for this scope. Close it before starting another.`);
    }
    throw err;
  });
  await db.query(
    `INSERT INTO audit_items (run_id, org_id, asset_id, expected_building_id, expected_location_id)
     SELECT $1, org_id, id, building_id, location_id FROM assets
      WHERE org_id = $2 AND status <> 'Retired'
        AND ($3::uuid IS NULL OR building_id = $3) AND ($4::uuid IS NULL OR location_id = $4)`,
    [id, orgId, buildingId, locationId],
  );
  return id;
}

auditsRouter.post('/', requirePermission('audits:perform'), route(async (req, res) => {
  const body = startSchema.parse(req.body);
  const run = await withTransaction(async (db) => {
    const id = await startAudit(db, {
      orgId: req.user.org_id, userId: req.user.id, period: body.period, buildingId: body.buildingId ?? null,
      locationId: body.locationId ?? null, name: body.name ?? null, dueAt: body.dueAt ?? null,
    });
    return findRun(db, req.user.org_id, id);
  });
  res.status(201).json(run);
}));

auditsRouter.get('/:id', requirePermission('audits:read'), route(async (req, res) => {
  const run = await findRun({ query }, req.user.org_id, uuid.parse(req.params.id));
  const { rows: items } = await query(
    `${ITEM_SELECT} WHERE i.run_id = $1
      ORDER BY CASE i.result WHEN 'pending' THEN 0 WHEN 'wrong_location' THEN 1 WHEN 'missing' THEN 2 ELSE 3 END,
               a.asset_tag`,
    [run.id],
  );
  res.json({ ...run, items });
}));

auditsRouter.post('/:id/scan', requirePermission('audits:perform'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = scanSchema.parse(req.body);
  const orgId = req.user.org_id;
  const item = await withTransaction(async (db) => {
    const run = await findRun(db, orgId, id, { lock: true });
    requireOpen(run);
    const asset = await findByCode(db, orgId, body.code);
    const foundLocationId = body.locationId ?? run.locationId ?? null;
    if (body.locationId) {
      const { rowCount } = await db.query('SELECT 1 FROM locations WHERE org_id = $1 AND id = $2', [orgId, body.locationId]);
      if (!rowCount) throw new HttpError(400, 'Location not found');
    }
    // An asset counts as misplaced when the auditor says where they are and it is not where it was expected.
    await db.query(
      `INSERT INTO audit_items (run_id, org_id, asset_id, expected, result, found_location_id, method, checked_by, checked_at)
       VALUES ($1, $2, $3, false, 'verified', $4, 'scan', $5, now())
       ON CONFLICT (run_id, asset_id) DO UPDATE SET
         result = CASE WHEN EXCLUDED.found_location_id IS NOT NULL AND audit_items.expected_location_id IS NOT NULL
                            AND EXCLUDED.found_location_id <> audit_items.expected_location_id
                       THEN 'wrong_location' ELSE 'verified' END,
         found_location_id = EXCLUDED.found_location_id, method = 'scan',
         checked_by = EXCLUDED.checked_by, checked_at = now()`,
      [id, orgId, asset.id, foundLocationId, req.user.id],
    );
    return findItem(db, id, asset.id);
  });
  res.json(item);
}));

auditsRouter.patch('/:id/items/:assetId', requirePermission('audits:perform'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const assetId = uuid.parse(req.params.assetId);
  const body = markSchema.parse(req.body);
  const item = await withTransaction(async (db) => {
    requireOpen(await findRun(db, req.user.org_id, id, { lock: true }));
    const pending = body.result === 'pending';
    const { rowCount } = await db.query(
      `UPDATE audit_items SET result = $3, method = $4, note = COALESCE($5, note),
              checked_by = $6, checked_at = $7, found_location_id = NULL
        WHERE run_id = $1 AND asset_id = $2`,
      [id, assetId, body.result, pending ? null : 'manual', body.note ?? null,
        pending ? null : req.user.id, pending ? null : new Date()],
    );
    if (!rowCount) throw notFound('Audit item');
    return findItem(db, id, assetId);
  });
  res.json(item);
}));

auditsRouter.post('/:id/close', requirePermission('audits:perform'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const orgId = req.user.org_id;
  const run = await withTransaction(async (db) => {
    const current = await findRun(db, orgId, id, { lock: true });
    requireOpen(current);
    // Anything not found by the time the audit closes is recorded as missing.
    await db.query(
      `UPDATE audit_items SET result = 'missing', checked_by = $2, checked_at = now()
        WHERE run_id = $1 AND result = 'pending'`,
      [id, req.user.id],
    );
    const { rows: items } = await db.query('SELECT asset_id, expected, result FROM audit_items WHERE run_id = $1', [id]);
    for (const item of items) {
      await recordEvent(db, {
        orgId, assetId: item.asset_id, actorId: req.user.id, type: 'audited',
        changes: { audit: { from: null, to: { id, name: current.name, period: current.period, result: item.result, expected: item.expected } } },
      });
    }
    const { counts } = await findRun(db, orgId, id);
    await db.query(
      `UPDATE audit_runs SET status = 'closed', closed_by = $2, closed_at = now(), summary = $3 WHERE id = $1`,
      [id, req.user.id, JSON.stringify(counts)],
    );
    return findRun(db, orgId, id);
  });
  res.json(run);
}));
