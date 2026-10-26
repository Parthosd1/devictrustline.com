import { Router } from 'express';
import { z } from 'zod';
import { query, withTransaction } from '../db/pool.js';
import { hashPassword } from '../lib/auth.js';
import { HttpError, notFound } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { ROLES } from '../lib/permissions.js';
import { requirePermission } from '../middleware/auth.js';
import { passwordSchema } from './auth.js';

const uuid = z.string().uuid();
const createSchema = z.object({
  name: z.string().trim().min(1).max(120),
  email: z.string().trim().toLowerCase().email().max(254),
  role: z.enum(ROLES),
  // Temporary password the admin shares with the new user; invitations by email come later.
  password: passwordSchema,
});
const updateSchema = z.object({
  name: z.string().trim().min(1).max(120).optional(),
  role: z.enum(ROLES).optional(),
  isActive: z.boolean().optional(),
  password: passwordSchema.optional(),
}).strict();

const COLUMNS = 'id, email, name, role, is_active AS "isActive", last_login_at AS "lastLoginAt", created_at AS "createdAt"';

export const usersRouter = Router();

usersRouter.get('/', requirePermission('users:read'), route(async (req, res) => {
  const { rows } = await query(`SELECT ${COLUMNS} FROM users WHERE org_id = $1 ORDER BY name`, [req.user.org_id]);
  res.json({ items: rows });
}));

usersRouter.post('/', requirePermission('users:manage'), route(async (req, res) => {
  const body = createSchema.parse(req.body);
  const { rows: [user] } = await query(
    `INSERT INTO users (org_id, email, name, password_hash, role) VALUES ($1, $2, $3, $4, $5) RETURNING ${COLUMNS}`,
    [req.user.org_id, body.email, body.name, await hashPassword(body.password), body.role],
  );
  res.status(201).json(user);
}));

usersRouter.patch('/:id', requirePermission('users:manage'), route(async (req, res) => {
  const id = uuid.parse(req.params.id);
  const body = updateSchema.parse(req.body);
  const passwordHash = body.password && await hashPassword(body.password);
  const user = await withTransaction(async (db) => {
    // Lock the org's admins so two concurrent edits cannot remove the last one.
    const { rows: admins } = await db.query(
      `SELECT id FROM users WHERE org_id = $1 AND role = 'admin' AND is_active FOR UPDATE`, [req.user.org_id],
    );
    const { rows: [current] } = await db.query('SELECT * FROM users WHERE id = $1 AND org_id = $2', [id, req.user.org_id]);
    if (!current) throw notFound('User');
    const stillAdmin = (body.role ?? current.role) === 'admin' && (body.isActive ?? current.is_active);
    if (current.role === 'admin' && current.is_active && !stillAdmin && admins.length <= 1) {
      throw new HttpError(409, 'An organization needs at least one active admin');
    }
    const { rows: [updated] } = await db.query(
      `UPDATE users SET
         name = COALESCE($3, name),
         role = COALESCE($4, role),
         is_active = COALESCE($5, is_active),
         password_hash = COALESCE($6, password_hash),
         session_version = session_version + CASE WHEN $6::text IS NULL THEN 0 ELSE 1 END,
         updated_at = now()
       WHERE id = $1 AND org_id = $2 RETURNING ${COLUMNS}`,
      [id, req.user.org_id, body.name ?? null, body.role ?? null, body.isActive ?? null, passwordHash ?? null],
    );
    return updated;
  });
  res.json(user);
}));
