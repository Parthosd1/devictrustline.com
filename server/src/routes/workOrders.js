import { Router } from 'express';
import { z } from 'zod';
import { query, withTransaction } from '../db/pool.js';
import { HttpError, notFound } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { can } from '../lib/permissions.js';
import { requirePermission } from '../middleware/auth.js';
import { recordEvent } from './assets.js';

const uuid = z.string().uuid();
const PRIORITIES = ['low', 'normal', 'high', 'urgent'];
const ACTIVE = ['open', 'in_progress', 'waiting_parts'];
const optionalText = (max) => z.string().trim().max(max).transform((v) => v || null).nullish();
const money = z.union([z.number(), z.string().trim().regex(/^\d+(\.\d{1,2})?$/).transform(Number)])
  .refine((n) => n >= 0 && n < 1e10, 'Must be a positive amount').nullish();
const date = z.string().regex(/^\d{4}-\d{2}-\d{2}$/, 'Use YYYY-MM-DD').nullish();

const createSchema = z.object({
  assetId: uuid,
  title: z.string().trim().min(1).max(160),
  description: optionalText(4000),
  priority: z.enum(PRIORITIES).default('normal'),
  assigneeId: uuid.nullish(),
  vendor: optionalText(160),
  dueDate: date,
  // Puts the asset into Maintenance status while the work order is open.
  setAssetToMaintenance: z.boolean().default(true),
}).strict();
const updateSchema = z.object({
  title: z.string().trim().min(1).max(160),
  description: optionalText(4000),
  priority: z.enum(PRIORITIES),
  status: z.enum(ACTIVE),
  assigneeId: uuid.nullish(),
  vendor: optionalText(160),
  cost: money,
  dueDate: date,
}).partial().strict();
const noteSchema = z.object({ body: z.string().trim().min(1).max(4000) }).strict();
const resolveSchema = z.object({
  resolution: z.string().trim().min(1).max(4000),
  cost: money,
  // What the asset becomes once fixed; by default Deployed if someone holds it, otherwise Available.
  assetStatus: z.enum(['Available', 'Deployed', 'Retired']).nullish(),
}).strict();
const cancelSchema = z.object({ reason: z.string().trim().min(1).max(4000) }).strict();
const listSchema = z.object({
  status: z.enum([...ACTIVE, 'resolved', 'cancelled', 'active', 'closed']).optional(),
  assetId: uuid.optional(),
  assigneeId: uuid.optional(),
  limit: z.coerce.number().int().min(1).max(500).default(200),
});

const COLUMN = {
  title: 'title', description: 'description', priority: 'priority', status: 'status', assigneeId: 'assignee_id',
  vendor: 'vendor', cost: 'cost', dueDate: 'due_date',
};
const STATUS_LABEL = {
  open: 'Open', in_progress: 'In progress', waiting_parts: 'Waiting on parts', resolved: 'Resolved', cancelled: 'Cancelled',
};

const SELECT = `
  SELECT w.id, w.number, w.title, w.description, w.priority, w.status, w.vendor, w.cost::float AS cost,
         w.due_date AS "dueDate", w.resolution, w.opened_at AS "openedAt", w.closed_at AS "closedAt",
         w.updated_at AS "updatedAt", ou.name AS "openedBy",
         w.assignee_id AS "assigneeId", au.name AS "assigneeName",
         w.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", a.type AS "assetType",
         a.status AS "assetStatus", b.name AS "buildingName", l.name AS "locationName",
         (w.status IN ('open', 'in_progress', 'waiting_parts') AND w.due_date < current_date) AS overdue
    FROM work_orders w
    JOIN assets a ON a.id = w.asset_id
    LEFT JOIN buildings b ON b.id = a.building_id
    LEFT JOIN locations l ON l.id = a.location_id
    LEFT JOIN users ou ON ou.id = w.opened_by
    LEFT JOIN users au ON au.id = w.assignee_id`;

async function findWorkOrder(db, orgId, id, { lock = false } = {}) {
  const { rows: [wo] } = await db.query(`${SELECT} WHERE w.org_id = $1 AND w.id = $2${lock ? ' FOR UPDATE OF w' : ''}`, [orgId, id]);
  if (!wo) throw notFound('Work order');
  return wo;
}

async function checkAssignee(db, orgId, assigneeId) {
  if (!assigneeId) return;
  const { rows: [u] } = await db.query('SELECT role, is_active FROM users WHERE org_id = $1 AND id = $2', [orgId, assigneeId]);
  if (!u || !u.is_active || !can(u.role, 'maintenance:write')) {
    throw new HttpError(400, 'Assign work orders to an active admin, manager or technician');
  }
}

const addNote = (db, { orgId, id, authorId, kind = 'note', body }) => db.query(
  'INSERT INTO work_order_notes (org_id, work_order_id, author_id, kind, body) VALUES ($1, $2, $3, $4, $5)',
  [orgId, id, authorId, kind, body],
);

async function setAssetStatus(db, { orgId, assetId, actorId, status, workOrder }) {
  const { rows: [asset] } = await db.query('SELECT status FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE', [orgId, assetId]);
  if (asset.status === status) return;
  await db.query('UPDATE assets SET status = $3, updated_at = now() WHERE org_id = $1 AND id = $2', [orgId, assetId, status]);
  await recordEvent(db, {
    orgId, assetId, actorId, type: 'status_changed',
    changes: { status: { from: asset.status, to: status }, workOrder: { from: null, to: workOrder } },
  });
}

export const workOrdersRouter = Router();

workOrdersRouter.get('/', requirePermission('maintenance:read'), route(async (req, res) => {
  const f = listSchema.parse(req.query);
  const params = [req.user.org_id];
  const where = ['w.org_id = $1'];
  const add = (sql, v) => { params.push(v); where.push(sql.replace('?', `$${params.length}`)); };
  if (f.status === 'active') where.push(`w.status IN ('open', 'in_progress', 'waiting_parts')`);
  else if (f.status === 'closed') where.push(`w.status IN ('resolved', 'cancelled')`);
  else if (f.status) add('w.status = ?', f.status);
  if (f.assetId) add('w.asset_id = ?', f.assetId);
  if (f.assigneeId) add('w.assignee_id = ?', f.assigneeId);
  const { rows } = await query(
    `${SELECT} WHERE ${where.join(' AND ')}
      ORDER BY (w.status IN ('resolved', 'cancelled')), array_position(ARRAY['urgent','high','normal','low'], w.priority),
               w.opened_at DESC LIMIT ${f.limit}`,
    params,
  );
  res.json({ items: rows });
}));

// People who can be given work orders.
workOrdersRouter.get('/assignees', requirePermission('maintenance:read'), route(async (req, res) => {
  const { rows } = await query(
    `SELECT id, name, role FROM users WHERE org_id = $1 AND is_active AND role IN ('admin', 'manager', 'technician') ORDER BY name`,
    [req.user.org_id],
  );
  res.json({ items: rows });
}));

workOrdersRouter.post('/', requirePermission('maintenance:write'), route(async (req, res) => {
  const body = createSchema.parse(req.body);
  const orgId = req.user.org_id;
  const wo = await withTransaction(async (db) => {
    const { rows: [asset] } = await db.query('SELECT status FROM assets WHERE org_id = $1 AND id = $2', [orgId, body.assetId]);
    if (!asset) throw new HttpError(400, 'Asset not found');
    if (asset.status === 'Retired') throw new HttpError(409, 'Retired assets cannot get work orders');
    await checkAssignee(db, orgId, body.assigneeId);
    const { rows: [{ n }] } = await db.query(
      'UPDATE organizations SET next_work_order_number = next_work_order_number + 1 WHERE id = $1 RETURNING next_work_order_number - 1 AS n',
      [orgId],
    );
    const { rows: [{ id }] } = await db.query(
      `INSERT INTO work_orders (org_id, number, asset_id, title, description, priority, assignee_id, vendor, due_date, opened_by)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id`,
      [orgId, `WO-${n}`, body.assetId, body.title, body.description ?? null, body.priority, body.assigneeId ?? null,
        body.vendor ?? null, body.dueDate ?? null, req.user.id],
    );
    await addNote(db, { orgId, id, authorId: req.user.id, kind: 'status', body: 'Opened' });
    await recordEvent(db, {
      orgId, assetId: body.assetId, actorId: req.user.id, type: 'maintenance_opened',
      changes: { workOrder: { from: null, to: { id, number: `WO-${n}`, title: body.title } } },
    });
    if (body.setAssetToMaintenance) {
      await setAssetStatus(db, { orgId, assetId: body.assetId, actorId: req.user.id, status: 'Maintenance', workOrder: `WO-${n}` });
    }
    return findWorkOrder(db, orgId, id);
  });
  res.status(201).json(wo);
}));

workOrdersRouter.get('/:id', requirePermission('maintenance:read'), route(async (req, res) => {
  const wo = await findWorkOrder({ query }, req.user.org_id, uuid.parse(req.params.id));
  const { rows: notes } = await query(
    `SELECT n.id, n.kind, n.body, n.created_at AS "createdAt", u.name AS author
       FROM work_order_notes n LEFT JOIN users u ON u.id = n.author_id
      WHERE n.work_order_id = $1 ORDER BY n.id`,
    [wo.id],
  );
  res.json({ ...wo, notes });
}));

workOrdersRouter.patch('/:id', requirePermission('maintenance:write'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = updateSchema.parse(req.body);
  const orgId = req.user.org_id;
  const wo = await withTransaction(async (db) => {
    const current = await findWorkOrder(db, orgId, id, { lock: true });
    if (!ACTIVE.includes(current.status)) throw new HttpError(409, 'This work order is closed');
    if (body.assigneeId) await checkAssignee(db, orgId, body.assigneeId);
    const keys = Object.keys(body).filter((k) => body[k] !== undefined);
    if (keys.length) {
      await db.query(
        `UPDATE work_orders SET ${keys.map((k, i) => `${COLUMN[k]} = $${i + 3}`).join(', ')}, updated_at = now()
          WHERE org_id = $1 AND id = $2`,
        [orgId, id, ...keys.map((k) => body[k] ?? null)],
      );
    }
    if (body.status && body.status !== current.status) {
      await addNote(db, { orgId, id, authorId: req.user.id, kind: 'status', body: `Status: ${STATUS_LABEL[body.status]}` });
    }
    if (body.assigneeId !== undefined && (body.assigneeId ?? null) !== current.assigneeId) {
      const { rows: [u] } = body.assigneeId
        ? await db.query('SELECT name FROM users WHERE id = $1', [body.assigneeId]) : { rows: [null] };
      await addNote(db, { orgId, id, authorId: req.user.id, kind: 'status', body: u ? `Assigned to ${u.name}` : 'Unassigned' });
    }
    return findWorkOrder(db, orgId, id);
  });
  res.json(wo);
}));

workOrdersRouter.post('/:id/notes', requirePermission('maintenance:write'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const { body } = noteSchema.parse(req.body);
  await findWorkOrder({ query }, req.user.org_id, id);
  await addNote({ query }, { orgId: req.user.org_id, id, authorId: req.user.id, body });
  res.status(201).json({ ok: true });
}));

async function close(req, { status, text, cost, assetStatus }) {
  const id = uuid.parse(req.params.id);
  const orgId = req.user.org_id;
  return withTransaction(async (db) => {
    const current = await findWorkOrder(db, orgId, id, { lock: true });
    if (!ACTIVE.includes(current.status)) throw new HttpError(409, 'This work order is already closed');
    await db.query(
      `UPDATE work_orders SET status = $3, resolution = $4, cost = COALESCE($5, cost), closed_at = now(), updated_at = now()
        WHERE org_id = $1 AND id = $2`,
      [orgId, id, status, text, cost ?? null],
    );
    await addNote(db, { orgId, id, authorId: req.user.id, kind: 'status', body: `${STATUS_LABEL[status]}: ${text}` });
    await recordEvent(db, {
      orgId, assetId: current.assetId, actorId: req.user.id, type: 'maintenance_closed',
      changes: { workOrder: { from: null, to: { id, number: current.number, title: current.title, status, resolution: text } } },
    });
    // Release the asset from Maintenance once no other work order is still active on it.
    const { rows: [{ others }] } = await db.query(
      `SELECT count(*)::int AS others FROM work_orders
        WHERE asset_id = $1 AND id <> $2 AND status IN ('open', 'in_progress', 'waiting_parts')`,
      [current.assetId, id],
    );
    if (!others && (current.assetStatus === 'Maintenance' || assetStatus)) {
      const { rows: [a] } = await db.query('SELECT assigned_person_id FROM assets WHERE id = $1', [current.assetId]);
      const next = assetStatus ?? (a.assigned_person_id ? 'Deployed' : 'Available');
      await setAssetStatus(db, { orgId, assetId: current.assetId, actorId: req.user.id, status: next, workOrder: current.number });
    }
    return findWorkOrder(db, orgId, id);
  });
}

workOrdersRouter.post('/:id/resolve', requirePermission('maintenance:write'), route(async (req, res) => {
  const body = resolveSchema.parse(req.body);
  res.json(await close(req, { status: 'resolved', text: body.resolution, cost: body.cost, assetStatus: body.assetStatus }));
}));

workOrdersRouter.post('/:id/cancel', requirePermission('maintenance:write'), route(async (req, res) => {
  const body = cancelSchema.parse(req.body);
  res.json(await close(req, { status: 'cancelled', text: body.reason }));
}));
