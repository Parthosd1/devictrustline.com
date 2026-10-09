import { Router } from 'express';
import { z } from 'zod';
import { query, withTransaction } from '../db/pool.js';
import { HttpError, notFound } from '../lib/errors.js';
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
  parentId: uuid.nullish(),
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
  personId: uuid.optional(),
  parentId: uuid.optional(),
  limit: z.coerce.number().int().min(1).max(500).default(100),
  offset: z.coerce.number().int().min(0).default(0),
});

// API field name -> column name for every editable field.
const COLUMN = {
  name: 'name', type: 'type', serial: 'serial', manufacturer: 'manufacturer', model: 'model',
  status: 'status', buildingId: 'building_id', locationId: 'location_id', parentId: 'parent_asset_id', notes: 'notes',
  purchaseDate: 'purchase_date', warrantyExpires: 'warranty_expires',
};

const SELECT = `
  SELECT a.id, a.asset_tag AS "assetTag", a.name, a.type, a.serial, a.manufacturer, a.model, a.status,
         a.building_id AS "buildingId", b.name AS "buildingName",
         a.location_id AS "locationId", l.name AS "locationName",
         a.parent_asset_id AS "parentId", pa.asset_tag AS "parentTag", pa.name AS "parentName",
         a.assigned_person_id AS "assignedPersonId", pe.name AS "assignedPersonName", a.assigned_at AS "assignedAt",
         (SELECT count(*)::int FROM assets c WHERE c.parent_asset_id = a.id) AS "childCount",
         a.notes, a.purchase_date AS "purchaseDate", a.warranty_expires AS "warrantyExpires",
         a.created_at AS "createdAt", a.updated_at AS "updatedAt"
    FROM assets a
    LEFT JOIN buildings b ON b.id = a.building_id
    LEFT JOIN locations l ON l.id = a.location_id
    LEFT JOIN assets pa ON pa.id = a.parent_asset_id
    LEFT JOIN people pe ON pe.id = a.assigned_person_id`;

async function findAsset(db, orgId, id) {
  const { rows: [asset] } = await db.query(`${SELECT} WHERE a.org_id = $1 AND a.id = $2`, [orgId, id]);
  if (!asset) throw notFound('Asset');
  return asset;
}

const recordEvent = (db, { orgId, assetId, actorId, type, changes }) => db.query(
  'INSERT INTO asset_events (org_id, asset_id, actor_id, event_type, changes) VALUES ($1, $2, $3, $4, $5)',
  [orgId, assetId, actorId, type, JSON.stringify(changes)],
);

// Every asset below this one in the parent-child tree, nearest first. Rows are locked for update.
async function descendantIds(db, orgId, id) {
  const { rows } = await db.query(
    `WITH RECURSIVE tree AS (
       SELECT id, 1 AS depth FROM assets WHERE org_id = $1 AND parent_asset_id = $2
       UNION ALL
       SELECT a.id, t.depth + 1 FROM assets a JOIN tree t ON a.parent_asset_id = t.id WHERE t.depth < 20
     )
     SELECT a.id FROM tree t JOIN assets a ON a.id = t.id ORDER BY t.depth FOR UPDATE OF a`,
    [orgId, id],
  );
  return rows.map((r) => r.id);
}

async function parentPlacement(db, orgId, parentId) {
  const { rows: [parent] } = await db.query(
    'SELECT building_id AS "buildingId", location_id AS "locationId" FROM assets WHERE org_id = $1 AND id = $2',
    [orgId, parentId],
  );
  if (!parent) throw new HttpError(400, 'Parent asset not found');
  return parent;
}

async function findPerson(db, orgId, id) {
  const { rows: [person] } = await db.query(
    'SELECT id, name, is_active FROM people WHERE org_id = $1 AND id = $2', [orgId, id],
  );
  if (!person) throw new HttpError(400, 'Person not found');
  if (!person.is_active) throw new HttpError(409, `${person.name} is inactive`);
  return person;
}

export const assetsRouter = Router();

assetsRouter.get('/', requirePermission('assets:read'), route(async (req, res) => {
  const f = listSchema.parse(req.query);
  const where = ['a.org_id = $1'];
  const params = [req.user.org_id];
  const add = (sql, value) => { params.push(value); where.push(sql.replaceAll('?', `$${params.length}`)); };
  if (f.q) {
    add(`(a.asset_tag ILIKE ? OR a.name ILIKE ? OR a.serial ILIKE ? OR a.type ILIKE ?
          OR a.model ILIKE ? OR b.name ILIKE ? OR l.name ILIKE ? OR pe.name ILIKE ?)`,
    `%${f.q.replace(/[\\%_]/g, '\\$&')}%`);
  }
  if (f.status) add('a.status = ?', f.status);
  if (f.type) add('a.type = ?', f.type);
  if (f.buildingId) add('a.building_id = ?', f.buildingId);
  if (f.locationId) add('a.location_id = ?', f.locationId);
  if (f.personId) add('a.assigned_person_id = ?', f.personId);
  if (f.parentId) add('a.parent_asset_id = ?', f.parentId);
  const whereSql = where.join(' AND ');
  const [{ rows: items }, { rows: [{ total }] }] = await Promise.all([
    query(`${SELECT} WHERE ${whereSql} ORDER BY a.created_at DESC, a.asset_tag DESC
           LIMIT ${f.limit} OFFSET ${f.offset}`, params),
    query(`SELECT count(*)::int AS total FROM assets a
             LEFT JOIN buildings b ON b.id = a.building_id
             LEFT JOIN locations l ON l.id = a.location_id
             LEFT JOIN people pe ON pe.id = a.assigned_person_id WHERE ${whereSql}`, params),
  ]);
  res.json({ items, total, limit: f.limit, offset: f.offset });
}));

// Resolves a scanned code: an asset tag, a serial number, or a DeviceTrustline label URL (…?asset=DT-1001).
export function parseScannedCode(raw) {
  const code = raw.trim();
  try {
    const url = new URL(code);
    const tag = url.searchParams.get('asset');
    if (tag) return tag.trim();
  } catch { /* not a URL */ }
  return code;
}

const lookupSchema = z.object({ code: z.string().trim().min(1).max(500) });

assetsRouter.get('/lookup', requirePermission('assets:read'), route(async (req, res) => {
  const code = parseScannedCode(lookupSchema.parse(req.query).code);
  const { rows } = await query(
    `${SELECT} WHERE a.org_id = $1 AND (lower(a.asset_tag) = lower($2) OR lower(a.serial) = lower($2))
      ORDER BY (lower(a.asset_tag) = lower($2)) DESC LIMIT 1`,
    [req.user.org_id, code],
  );
  if (!rows[0]) throw new HttpError(404, `No asset matches “${code}”`);
  res.json({ ...rows[0], matchedBy: rows[0].assetTag.toLowerCase() === code.toLowerCase() ? 'assetTag' : 'serial' });
}));

assetsRouter.get('/:id', requirePermission('assets:read'), route(async (req, res) => {
  const asset = await findAsset({ query }, req.user.org_id, uuid.parse(req.params.id));
  const { rows: children } = await query(`${SELECT} WHERE a.org_id = $1 AND a.parent_asset_id = $2 ORDER BY a.asset_tag`,
    [req.user.org_id, asset.id]);
  res.json({ ...asset, children });
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
    // A component placed inside a parent sits where the parent is unless told otherwise.
    if (body.parentId && body.buildingId === undefined && body.locationId === undefined) {
      Object.assign(body, await parentPlacement(db, req.user.org_id, body.parentId));
    }
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
  const orgId = req.user.org_id;
  const asset = await withTransaction(async (db) => {
    const { rows: [current] } = await db.query(
      `SELECT name, type, serial, manufacturer, model, status, building_id AS "buildingId",
              location_id AS "locationId", parent_asset_id AS "parentId", notes,
              purchase_date AS "purchaseDate", warranty_expires AS "warrantyExpires"
         FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE`,
      [orgId, id],
    );
    if (!current) throw notFound('Asset');
    const descendants = await descendantIds(db, orgId, id);

    if (body.parentId && body.parentId !== current.parentId) {
      if (body.parentId === id || descendants.includes(body.parentId)) {
        throw new HttpError(400, 'An asset cannot be placed inside itself or one of its own components');
      }
      // Attaching to a new parent moves the asset to the parent's building and location.
      if (body.buildingId === undefined && body.locationId === undefined) {
        Object.assign(body, await parentPlacement(db, orgId, body.parentId));
      }
    }
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
        [orgId, id, ...fields.map((k) => changes[k].to)],
      );
      await recordEvent(db, {
        orgId, assetId: id, actorId: req.user.id, type: changes.status ? 'status_changed' : 'updated', changes,
      });
    }
    // Components travel with their parent.
    if ((changes.buildingId || changes.locationId) && descendants.length) {
      const { buildingId, locationId } = await parentPlacement(db, orgId, id);
      await moveAssets(db, { orgId, actorId: req.user.id, ids: descendants, buildingId, locationId, viaId: id });
    }
    return findAsset(db, orgId, id);
  });
  res.json(asset);
}));

async function moveAssets(db, { orgId, actorId, ids, buildingId, locationId, viaId }) {
  const { rows } = await db.query(
    `SELECT id, building_id AS "buildingId", location_id AS "locationId" FROM assets
      WHERE org_id = $1 AND id = ANY($2) AND (building_id IS DISTINCT FROM $3 OR location_id IS DISTINCT FROM $4)`,
    [orgId, ids, buildingId, locationId],
  );
  for (const row of rows) {
    await db.query('UPDATE assets SET building_id = $3, location_id = $4, updated_at = now() WHERE org_id = $1 AND id = $2',
      [orgId, row.id, buildingId, locationId]);
    const changes = { via: { from: null, to: viaId } };
    if (row.buildingId !== buildingId) changes.buildingId = { from: row.buildingId, to: buildingId };
    if (row.locationId !== locationId) changes.locationId = { from: row.locationId, to: locationId };
    await recordEvent(db, { orgId, assetId: row.id, actorId, type: 'moved_with_parent', changes });
  }
}

const assignSchema = z.object({
  personId: uuid,
  includeComponents: z.boolean().default(true),
  note: optionalText(500),
}).strict();
const returnSchema = z.object({
  status: z.enum(['Available', 'Maintenance']).default('Available'),
  includeComponents: z.boolean().default(true),
  note: optionalText(500),
}).strict();

// Assigns or returns an asset (and by default its components), recording one event per asset.
async function setAssignment(db, { orgId, actorId, id, person, status, includeComponents, note }) {
  const { rows: [root] } = await db.query('SELECT id, status FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE', [orgId, id]);
  if (!root) throw notFound('Asset');
  if (root.status === 'Retired') throw new HttpError(409, 'Retired assets cannot be assigned or returned');
  const ids = [id, ...(includeComponents ? await descendantIds(db, orgId, id) : [])];
  const { rows } = await db.query(
    `SELECT a.id, a.status, a.assigned_person_id AS "personId", p.name AS "personName"
       FROM assets a LEFT JOIN people p ON p.id = a.assigned_person_id
      WHERE a.org_id = $1 AND a.id = ANY($2) AND a.status <> 'Retired'`,
    [orgId, ids],
  );
  for (const row of rows) {
    const nextStatus = person ? (row.status === 'Available' ? 'Deployed' : row.status) : status;
    if (row.personId === (person?.id ?? null) && row.status === nextStatus) continue;
    await db.query(
      `UPDATE assets SET assigned_person_id = $3, assigned_at = CASE WHEN $3::uuid IS NULL THEN NULL ELSE now() END,
              status = $4, updated_at = now() WHERE org_id = $1 AND id = $2`,
      [orgId, row.id, person?.id ?? null, nextStatus],
    );
    const changes = {
      assignedTo: {
        from: row.personId && { id: row.personId, name: row.personName },
        to: person && { id: person.id, name: person.name },
      },
    };
    if (nextStatus !== row.status) changes.status = { from: row.status, to: nextStatus };
    if (row.id !== id) changes.via = { from: null, to: id };
    if (note) changes.note = { from: null, to: note };
    await recordEvent(db, { orgId, assetId: row.id, actorId, type: person ? 'assigned' : 'returned', changes });
  }
}

assetsRouter.post('/:id/assign', requirePermission('assets:write'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = assignSchema.parse(req.body);
  const asset = await withTransaction(async (db) => {
    const person = await findPerson(db, req.user.org_id, body.personId);
    await setAssignment(db, { orgId: req.user.org_id, actorId: req.user.id, id, person, ...body });
    return findAsset(db, req.user.org_id, id);
  });
  res.json(asset);
}));

assetsRouter.post('/:id/return', requirePermission('assets:write'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = returnSchema.parse(req.body);
  const asset = await withTransaction(async (db) => {
    await setAssignment(db, { orgId: req.user.org_id, actorId: req.user.id, id, person: null, ...body });
    return findAsset(db, req.user.org_id, id);
  });
  res.json(asset);
}));
