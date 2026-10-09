import { Router } from 'express';
import { z } from 'zod';
import { query, withTransaction } from '../db/pool.js';
import { notFound } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { requirePermission } from '../middleware/auth.js';

export const ASSET_TYPES = [
  'Laptop', 'Desktop', 'Workstation', 'Monitor', 'Scanner', 'Printer', 'Docking Station',
  'Phone', 'Tablet', 'Network', 'Accessory', 'Other',
];
export const ASSET_STATUSES = ['Available', 'Deployed', 'Maintenance', 'Retired'];

const uuid = z.string().uuid();
const optionalText = (max) => z.string().trim().max(max).transform((v) => v || null).nullish();
const optionalDate = z.string().regex(/^\d{4}-\d{2}-\d{2}$/, 'Use YYYY-MM-DD').nullish();

const assetFields = {
  name: z.string().trim().min(1).max(160),
  type: z.enum(ASSET_TYPES),
  serial: optionalText(120),
  manufacturer: optionalText(120),
  model: optionalText(120),
  status: z.enum(ASSET_STATUSES),
  buildingId: uuid.nullish(),
  locationId: uuid.nullish(),
  notes: optionalText(2000),
  purchaseDate: optionalDate,
  warrantyExpires: optionalDate,
};
const createSchema = z.object({ ...assetFields, status: assetFields.status.default('Available') }).strict();
const updateSchema = z.object(assetFields).partial().strict();

const listSchema = z.object({
  q: z.string().trim().max(200).optional(),
  status: z.enum(ASSET_STATUSES).optional(),
  type: z.enum(ASSET_TYPES).optional(),
  buildingId: uuid.optional(),
  locationId: uuid.optional(),
  limit: z.coerce.number().int().min(1).max(500).default(100),
  offset: z.coerce.number().int().min(0).default(0),
});

// API field name -> column name for every editable field.
const COLUMN = {
  name: 'name', type: 'type', serial: 'serial', manufacturer: 'manufacturer', model: 'model',
  status: 'status', buildingId: 'building_id', locationId: 'location_id', notes: 'notes',
  purchaseDate: 'purchase_date', warrantyExpires: 'warranty_expires',
};

const SELECT = `
  SELECT a.id, a.asset_tag AS "assetTag", a.name, a.type, a.serial, a.manufacturer, a.model, a.status,
         a.building_id AS "buildingId", b.name AS "buildingName",
         a.location_id AS "locationId", l.name AS "locationName",
         a.notes, a.purchase_date AS "purchaseDate", a.warranty_expires AS "warrantyExpires",
         a.created_at AS "createdAt", a.updated_at AS "updatedAt"
    FROM assets a
    LEFT JOIN buildings b ON b.id = a.building_id
    LEFT JOIN locations l ON l.id = a.location_id`;

async function findAsset(db, orgId, id) {
  const { rows: [asset] } = await db.query(`${SELECT} WHERE a.org_id = $1 AND a.id = $2`, [orgId, id]);
  if (!asset) throw notFound('Asset');
  return asset;
}

const recordEvent = (db, { orgId, assetId, actorId, type, changes }) => db.query(
  'INSERT INTO asset_events (org_id, asset_id, actor_id, event_type, changes) VALUES ($1, $2, $3, $4, $5)',
  [orgId, assetId, actorId, type, JSON.stringify(changes)],
);

export const assetsRouter = Router();

assetsRouter.get('/', requirePermission('assets:read'), route(async (req, res) => {
  const f = listSchema.parse(req.query);
  const where = ['a.org_id = $1'];
  const params = [req.user.org_id];
  const add = (sql, value) => { params.push(value); where.push(sql.replaceAll('?', `$${params.length}`)); };
  if (f.q) {
    add(`(a.asset_tag ILIKE ? OR a.name ILIKE ? OR a.serial ILIKE ? OR a.type ILIKE ?
          OR a.model ILIKE ? OR b.name ILIKE ? OR l.name ILIKE ?)`,
    `%${f.q.replace(/[\\%_]/g, '\\$&')}%`);
  }
  if (f.status) add('a.status = ?', f.status);
  if (f.type) add('a.type = ?', f.type);
  if (f.buildingId) add('a.building_id = ?', f.buildingId);
  if (f.locationId) add('a.location_id = ?', f.locationId);
  const whereSql = where.join(' AND ');
  const [{ rows: items }, { rows: [{ total }] }] = await Promise.all([
    query(`${SELECT} WHERE ${whereSql} ORDER BY a.created_at DESC, a.asset_tag DESC
           LIMIT ${f.limit} OFFSET ${f.offset}`, params),
    query(`SELECT count(*)::int AS total FROM assets a
             LEFT JOIN buildings b ON b.id = a.building_id
             LEFT JOIN locations l ON l.id = a.location_id WHERE ${whereSql}`, params),
  ]);
  res.json({ items, total, limit: f.limit, offset: f.offset });
}));

assetsRouter.get('/:id', requirePermission('assets:read'), route(async (req, res) => {
  res.json(await findAsset({ query }, req.user.org_id, uuid.parse(req.params.id)));
}));

assetsRouter.get('/:id/events', requirePermission('assets:read'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  await findAsset({ query }, req.user.org_id, id);
  const { rows } = await query(
    `SELECT e.id, e.event_type AS "type", e.changes, e.created_at AS "createdAt",
            CASE WHEN u.id IS NULL THEN NULL ELSE json_build_object('id', u.id, 'name', u.name) END AS actor
       FROM asset_events e LEFT JOIN users u ON u.id = e.actor_id
      WHERE e.org_id = $1 AND e.asset_id = $2 ORDER BY e.id DESC`,
    [req.user.org_id, id],
  );
  res.json({ items: rows });
}));

assetsRouter.post('/', requirePermission('assets:write'), route(async (req, res) => {
  const body = createSchema.parse(req.body);
  const asset = await withTransaction(async (db) => {
    const { rows: [{ n }] } = await db.query(
      'UPDATE organizations SET next_asset_number = next_asset_number + 1 WHERE id = $1 RETURNING next_asset_number - 1 AS n',
      [req.user.org_id],
    );
    const fields = Object.keys(COLUMN).filter((k) => body[k] !== undefined);
    const cols = ['org_id', 'asset_tag', 'created_by', ...fields.map((k) => COLUMN[k])];
    const values = [req.user.org_id, `DT-${n}`, req.user.id, ...fields.map((k) => body[k])];
    const { rows: [{ id }] } = await db.query(
      `INSERT INTO assets (${cols.join(', ')}) VALUES (${cols.map((_, i) => `$${i + 1}`).join(', ')}) RETURNING id`,
      values,
    );
    const created = await findAsset(db, req.user.org_id, id);
    await recordEvent(db, {
      orgId: req.user.org_id, assetId: id, actorId: req.user.id, type: 'created',
      changes: Object.fromEntries(fields.map((k) => [k, { from: null, to: body[k] }])),
    });
    return created;
  });
  res.status(201).json(asset);
}));

assetsRouter.patch('/:id', requirePermission('assets:write'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = updateSchema.parse(req.body);
  const asset = await withTransaction(async (db) => {
    const { rows: [current] } = await db.query(
      `SELECT name, type, serial, manufacturer, model, status, building_id AS "buildingId",
              location_id AS "locationId", notes, purchase_date AS "purchaseDate", warranty_expires AS "warrantyExpires"
         FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE`,
      [req.user.org_id, id],
    );
    if (!current) throw notFound('Asset');
    // Moving to another building without naming a location clears the old location.
    if (body.buildingId !== undefined && body.buildingId !== current.buildingId && body.locationId === undefined) {
      body.locationId = null;
    }
    const changes = {};
    for (const [k, v] of Object.entries(body)) {
      if (v !== undefined && (v ?? null) !== current[k]) changes[k] = { from: current[k], to: v ?? null };
    }
    const fields = Object.keys(changes);
    if (fields.length) {
      await db.query(
        `UPDATE assets SET ${fields.map((k, i) => `${COLUMN[k]} = $${i + 3}`).join(', ')}, updated_at = now()
          WHERE org_id = $1 AND id = $2`,
        [req.user.org_id, id, ...fields.map((k) => changes[k].to)],
      );
      await recordEvent(db, {
        orgId: req.user.org_id, assetId: id, actorId: req.user.id,
        type: changes.status ? 'status_changed' : 'updated', changes,
      });
    }
    return findAsset(db, req.user.org_id, id);
  });
  res.json(asset);
}));
