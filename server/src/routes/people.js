import { Router } from 'express';
import { z } from 'zod';
import { query } from '../db/pool.js';
import { notFound } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { requirePermission } from '../middleware/auth.js';

const uuid = z.string().uuid();
const optionalText = (max) => z.string().trim().max(max).transform((v) => v || null).nullish();
const fields = {
  name: z.string().trim().min(1).max(120),
  email: z.string().trim().toLowerCase().email().max(254).or(z.literal('').transform(() => null)).nullish(),
  employeeId: optionalText(60),
  department: optionalText(120),
};
const createSchema = z.object(fields).strict();
const updateSchema = z.object({ ...fields, isActive: z.boolean() }).partial().strict();
const COLUMN = { name: 'name', email: 'email', employeeId: 'employee_id', department: 'department', isActive: 'is_active' };

const SELECT = `
  SELECT p.id, p.name, p.email, p.employee_id AS "employeeId", p.department, p.is_active AS "isActive",
         (SELECT count(*)::int FROM assets a WHERE a.assigned_person_id = p.id) AS "assetCount"
    FROM people p`;

export const peopleRouter = Router();

peopleRouter.get('/', requirePermission('assets:read'), route(async (req, res) => {
  const { rows } = await query(`${SELECT} WHERE p.org_id = $1 ORDER BY p.is_active DESC, p.name`, [req.user.org_id]);
  res.json({ items: rows });
}));

peopleRouter.post('/', requirePermission('people:write'), route(async (req, res) => {
  const body = createSchema.parse(req.body);
  const { rows: [{ id }] } = await query(
    'INSERT INTO people (org_id, name, email, employee_id, department) VALUES ($1, $2, $3, $4, $5) RETURNING id',
    [req.user.org_id, body.name, body.email ?? null, body.employeeId ?? null, body.department ?? null],
  );
  const { rows: [person] } = await query(`${SELECT} WHERE p.id = $1`, [id]);
  res.status(201).json(person);
}));

peopleRouter.patch('/:id', requirePermission('people:write'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = updateSchema.parse(req.body);
  const keys = Object.keys(body);
  if (keys.length) {
    const { rowCount } = await query(
      `UPDATE people SET ${keys.map((k, i) => `${COLUMN[k]} = $${i + 3}`).join(', ')}, updated_at = now()
        WHERE id = $1 AND org_id = $2`,
      [id, req.user.org_id, ...keys.map((k) => body[k] ?? null)],
    );
    if (!rowCount) throw notFound('Person');
  }
  const { rows: [person] } = await query(`${SELECT} WHERE p.id = $1 AND p.org_id = $2`, [id, req.user.org_id]);
  if (!person) throw notFound('Person');
  res.json(person);
}));
