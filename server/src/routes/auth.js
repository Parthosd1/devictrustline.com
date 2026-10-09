import { Router } from 'express';
import rateLimit from 'express-rate-limit';
import { z } from 'zod';
import { config } from '../config.js';
import { query, withTransaction } from '../db/pool.js';
import {
  DUMMY_HASH, SESSION_COOKIE, hashPassword, sessionCookieOptions, signSession, verifyPassword,
} from '../lib/auth.js';
import { HttpError } from '../lib/errors.js';
import { route } from '../lib/http.js';
import { permissionsFor } from '../lib/permissions.js';
import { authenticate } from '../middleware/auth.js';

export const passwordSchema = z.string().min(12, 'Password must be at least 12 characters').max(200);
const email = z.string().trim().toLowerCase().email().max(254);

const signupSchema = z.object({
  organizationName: z.string().trim().min(1).max(120),
  name: z.string().trim().min(1).max(120),
  email,
  password: passwordSchema,
});
const loginSchema = z.object({ email, password: z.string().min(1).max(200) });
const changePasswordSchema = z.object({ currentPassword: z.string().min(1).max(200), newPassword: passwordSchema });

const authLimiter = rateLimit({
  windowMs: 15 * 60 * 1000,
  limit: Number(process.env.AUTH_RATE_LIMIT || 20),
  standardHeaders: 'draft-7',
  legacyHeaders: false,
  message: { error: 'Too many attempts. Try again in a few minutes.' },
});

export function publicUser(u) {
  return {
    id: u.id,
    email: u.email,
    name: u.name,
    role: u.role,
    organization: { id: u.org_id, name: u.org_name },
    permissions: permissionsFor(u.role),
  };
}

function startSession(res, user) {
  res.cookie(SESSION_COOKIE, signSession(user), sessionCookieOptions());
}

export const authRouter = Router();

authRouter.post('/signup', authLimiter, route(async (req, res) => {
  if (!config.allowSignup) throw new HttpError(403, 'Sign-up is disabled. Ask your administrator for an account.');
  const body = signupSchema.parse(req.body);
  const passwordHash = await hashPassword(body.password);
  const user = await withTransaction(async (db) => {
    const { rows: [org] } = await db.query('INSERT INTO organizations (name) VALUES ($1) RETURNING id, name', [body.organizationName]);
    const { rows: [u] } = await db.query(
      `INSERT INTO users (org_id, email, name, password_hash, role)
       VALUES ($1, $2, $3, $4, 'admin') RETURNING id, org_id, email, name, role, session_version`,
      [org.id, body.email, body.name, passwordHash],
    );
    return { ...u, org_name: org.name };
  });
  startSession(res, user);
  res.status(201).json({ user: publicUser(user) });
}));

authRouter.post('/login', authLimiter, route(async (req, res) => {
  const body = loginSchema.parse(req.body);
  const { rows: [u] } = await query(
    `SELECT u.*, o.name AS org_name FROM users u JOIN organizations o ON o.id = u.org_id WHERE u.email = $1`,
    [body.email],
  );
  const ok = await verifyPassword(body.password, u?.password_hash ?? DUMMY_HASH);
  if (!u || !ok || !u.is_active) throw new HttpError(401, 'Incorrect email or password');
  await query('UPDATE users SET last_login_at = now() WHERE id = $1', [u.id]);
  startSession(res, u);
  res.json({ user: publicUser(u) });
}));

authRouter.post('/logout', (_req, res) => {
  const { maxAge, ...opts } = sessionCookieOptions();
  res.clearCookie(SESSION_COOKIE, opts);
  res.status(204).end();
});

authRouter.get('/me', authenticate, (req, res) => {
  res.json({ user: publicUser(req.user) });
});

authRouter.post('/change-password', authLimiter, authenticate, route(async (req, res) => {
  const body = changePasswordSchema.parse(req.body);
  const { rows: [u] } = await query('SELECT password_hash FROM users WHERE id = $1', [req.user.id]);
  if (!(await verifyPassword(body.currentPassword, u.password_hash))) {
    throw new HttpError(400, 'Current password is incorrect');
  }
  const { rows: [updated] } = await query(
    `UPDATE users SET password_hash = $1, session_version = session_version + 1, updated_at = now()
      WHERE id = $2 RETURNING id, org_id, session_version`,
    [await hashPassword(body.newPassword), req.user.id],
  );
  startSession(res, updated);
  res.status(204).end();
}));
