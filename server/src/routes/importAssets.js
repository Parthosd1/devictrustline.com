import { Router } from 'express';
import { z } from 'zod';
import { withTransaction } from '../db/pool.js';
import { HttpError } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { requirePermission } from '../middleware/auth.js';
import { ASSET_STATUSES, ASSET_TYPES, recordEvent } from './assets.js';

const MAX_ROWS = 2000;
const text = (max) => z.string().trim().max(max).optional().transform((v) => v || null);
const date = z.string().trim().optional().transform((v) => v || null)
  .refine((v) => v === null || /^\d{4}-\d{2}-\d{2}$/.test(v), 'Use YYYY-MM-DD');
const pick = (list) => z.string().trim().optional().transform((v, ctx) => {
  if (!v) return null;
  const match = list.find((x) => x.toLowerCase() === v.toLowerCase());
  if (!match) {
    ctx.addIssue({ code: 'custom', message: `"${v}" is not one of: ${list.join(', ')}` });
    return z.NEVER;
  }
  return match;
});

const rowSchema = z.object({
  name: z.string({ required_error: 'Required' }).trim().min(1, 'Required').max(160),
  type: pick(ASSET_TYPES).refine((v) => v !== null, 'Required'),
  serial: text(120),
  manufacturer: text(120),
  model: text(120),
  status: pick(ASSET_STATUSES),
  building: text(120),
  location: text(120),
  purchaseDate: date,
  warrantyExpires: date,
  notes: text(2000),
}).strip();

const bodySchema = z.object({
  rows: z.array(z.record(z.string(), z.any())).min(1, 'The file has no rows').max(MAX_ROWS, `Import at most ${MAX_ROWS} rows at a time`),
  createSites: z.boolean().default(false),
  dryRun: z.boolean().default(true),
}).strict();

export const importRouter = Router();

// Validates every row first; nothing is written unless all rows are valid, and then all are written together.
importRouter.post('/', requirePermission('assets:write'), route(async (req, res) => {
  const body = bodySchema.parse(req.body);
  const orgId = req.user.org_id;
  const result = await withTransaction(async (db) => {
    const errors = [];
    const parsed = body.rows.map((raw, i) => {
      const r = rowSchema.safeParse(Object.fromEntries(Object.entries(raw).map(([k, v]) => [k, v == null ? undefined : String(v)])));
      if (!r.success) {
        errors.push({ row: i + 1, messages: r.error.issues.map((x) => `${x.path.join('.') || 'row'}: ${x.message}`) });
        return null;
      }
      if (r.data.location && !r.data.building) errors.push({ row: i + 1, messages: ['location: needs a building'] });
      return r.data;
    });

    const { rows: sites } = await db.query(
      `SELECT b.id AS "buildingId", lower(b.name) AS building, l.id AS "locationId", lower(l.name) AS location
         FROM buildings b LEFT JOIN locations l ON l.building_id = b.id WHERE b.org_id = $1`,
      [orgId],
    );
    const buildings = new Map(sites.map((s) => [s.building, s.buildingId]));
    const locations = new Map(sites.filter((s) => s.locationId).map((s) => [`${s.building}/${s.location}`, s.locationId]));
    const { rows: serialRows } = await db.query('SELECT lower(serial) AS s FROM assets WHERE org_id = $1 AND serial IS NOT NULL', [orgId]);
    const serials = new Set(serialRows.map((r) => r.s));
    const newSites = { buildings: new Set(), locations: new Set() };

    parsed.forEach((row, i) => {
      if (!row) return;
      const messages = [];
      if (row.serial) {
        const key = row.serial.toLowerCase();
        if (serials.has(key)) messages.push(`serial: ${row.serial} already exists`);
        serials.add(key);
      }
      const b = row.building?.toLowerCase();
      const l = row.location?.toLowerCase();
      if (b && !buildings.has(b)) {
        if (body.createSites) newSites.buildings.add(row.building); else messages.push(`building: "${row.building}" does not exist`);
      }
      if (b && l && !locations.has(`${b}/${l}`)) {
        if (body.createSites) newSites.locations.add(`${row.building}\u0000${row.location}`);
        else messages.push(`location: "${row.location}" does not exist in ${row.building}`);
      }
      if (messages.length) errors.push({ row: i + 1, messages });
    });
    errors.sort((a, b) => a.row - b.row);

    const summary = {
      rows: body.rows.length, valid: body.rows.length - new Set(errors.map((e) => e.row)).size, errors,
      newBuildings: [...newSites.buildings], newLocations: [...newSites.locations].map((k) => k.replace('\u0000', ' / ')),
      created: 0,
    };
    if (body.dryRun || errors.length) return summary;

    for (const name of newSites.buildings) {
      const { rows: [row] } = await db.query('INSERT INTO buildings (org_id, name) VALUES ($1, $2) RETURNING id', [orgId, name]);
      buildings.set(name.toLowerCase(), row.id);
    }
    for (const key of newSites.locations) {
      const [building, location] = key.split('\u0000');
      const { rows: [row] } = await db.query(
        'INSERT INTO locations (org_id, building_id, name) VALUES ($1, $2, $3) RETURNING id',
        [orgId, buildings.get(building.toLowerCase()), location],
      );
      locations.set(`${building.toLowerCase()}/${location.toLowerCase()}`, row.id);
    }
    const { rows: [{ first }] } = await db.query(
      `UPDATE organizations SET next_asset_number = next_asset_number + $2 WHERE id = $1
       RETURNING next_asset_number - $2 AS first`,
      [orgId, parsed.length],
    );
    for (const [i, row] of parsed.entries()) {
      const buildingId = row.building ? buildings.get(row.building.toLowerCase()) : null;
      const locationId = row.location ? locations.get(`${row.building.toLowerCase()}/${row.location.toLowerCase()}`) : null;
      const { rows: [{ id }] } = await db.query(
        `INSERT INTO assets (org_id, asset_tag, name, type, serial, manufacturer, model, status, building_id, location_id,
                             purchase_date, warranty_expires, notes, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) RETURNING id`,
        [orgId, `DT-${first + i}`, row.name, row.type, row.serial, row.manufacturer, row.model, row.status ?? 'Available',
          buildingId, locationId, row.purchaseDate, row.warrantyExpires, row.notes, req.user.id],
      );
      await recordEvent(db, { orgId, assetId: id, actorId: req.user.id, type: 'created', changes: { source: { from: null, to: 'csv import' } } });
    }
    return { ...summary, created: parsed.length };
  });
  if (!body.dryRun && result.errors.length) {
    throw new HttpError(400, `${result.errors.length} row(s) need fixing before anything is imported`, result.errors);
  }
  res.status(body.dryRun ? 200 : 201).json(result);
}));
