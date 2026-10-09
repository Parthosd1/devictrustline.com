import { Router } from 'express';
import { z } from 'zod';
import { query } from '../db/pool.js';
import { HttpError, notFound } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { requirePermission } from '../middleware/auth.js';

const uuid = z.string().uuid();
const buildingSchema = z.object({
  name: z.string().trim().min(1).max(120),
  address: z.string().trim().max(300).nullish(),
});
const locationSchema = z.object({ name: z.string().trim().min(1).max(120) });

// Deleting a building or location that assets still reference is refused with a clear message.
async function deleteOrExplain(sql, params, what) {
  try {
    const { rowCount } = await query(sql, params);
    if (!rowCount) throw notFound(what);
  } catch (err) {
    if (err.code === '23503') throw new HttpError(409, `${what} still has assets. Move them first.`);
    throw err;
  }
}

export const sitesRouter = Router();

sitesRouter.get('/buildings', requirePermission('sites:read'), route(async (req, res) => {
  const { rows } = await query(
    `SELECT b.id, b.name, b.address,
            (SELECT count(*)::int FROM assets a WHERE a.building_id = b.id AND a.status <> 'Retired') AS "assetCount",
            COALESCE((SELECT json_agg(json_build_object(
                        'id', l.id, 'name', l.name,
                        'assetCount', (SELECT count(*)::int FROM assets a WHERE a.location_id = l.id AND a.status <> 'Retired'))
                      ORDER BY l.name)
                      FROM locations l WHERE l.building_id = b.id), '[]') AS locations
       FROM buildings b WHERE b.org_id = $1 ORDER BY b.name`,
    [req.user.org_id],
  );
  res.json({ items: rows });
}));

sitesRouter.post('/buildings', requirePermission('sites:write'), route(async (req, res) => {
  const body = buildingSchema.parse(req.body);
  const { rows: [b] } = await query(
    'INSERT INTO buildings (org_id, name, address) VALUES ($1, $2, $3) RETURNING id, name, address',
    [req.user.org_id, body.name, body.address ?? null],
  );
  res.status(201).json({ ...b, assetCount: 0, locations: [] });
}));

sitesRouter.patch('/buildings/:id', requirePermission('sites:write'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = buildingSchema.partial().strict().parse(req.body);
  const { rows: [b] } = await query(
    `UPDATE buildings SET name = COALESCE($3, name),
            address = CASE WHEN $4::boolean THEN $5 ELSE address END
      WHERE id = $1 AND org_id = $2 RETURNING id, name, address`,
    [id, req.user.org_id, body.name ?? null, 'address' in body, body.address ?? null],
  );
  if (!b) throw notFound('Building');
  res.json(b);
}));

sitesRouter.delete('/buildings/:id', requirePermission('sites:write'), route(async (req, res) => {
  await deleteOrExplain('DELETE FROM buildings WHERE id = $1 AND org_id = $2',
    [uuid.parse(req.params.id), req.user.org_id], 'Building');
  res.status(204).end();
}));

sitesRouter.post('/buildings/:id/locations', requirePermission('sites:write'), route(async (req, res) => {
  const buildingId = uuid.parse(req.params.id);
  const body = locationSchema.parse(req.body);
  const { rows: [b] } = await query('SELECT id FROM buildings WHERE id = $1 AND org_id = $2', [buildingId, req.user.org_id]);
  if (!b) throw notFound('Building');
  const { rows: [l] } = await query(
    'INSERT INTO locations (org_id, building_id, name) VALUES ($1, $2, $3) RETURNING id, name, building_id AS "buildingId"',
    [req.user.org_id, buildingId, body.name],
  );
  res.status(201).json({ ...l, assetCount: 0 });
}));

sitesRouter.patch('/locations/:id', requirePermission('sites:write'), route(async (req, res) => {
  const body = locationSchema.parse(req.body);
  const { rows: [l] } = await query(
    'UPDATE locations SET name = $3 WHERE id = $1 AND org_id = $2 RETURNING id, name, building_id AS "buildingId"',
    [uuid.parse(req.params.id), req.user.org_id, body.name],
  );
  if (!l) throw notFound('Location');
  res.json(l);
}));

sitesRouter.delete('/locations/:id', requirePermission('sites:write'), route(async (req, res) => {
  await deleteOrExplain('DELETE FROM locations WHERE id = $1 AND org_id = $2',
    [uuid.parse(req.params.id), req.user.org_id], 'Location');
  res.status(204).end();
}));
