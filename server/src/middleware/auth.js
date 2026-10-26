import { query } from '../db/pool.js';
import { SESSION_COOKIE, verifySession } from '../lib/auth.js';
import { HttpError } from '../lib/errors.js';
import { can } from '../lib/permissions.js';

function readToken(req) {
  const header = req.get('authorization');
  if (header?.startsWith('Bearer ')) return header.slice(7);
  return req.cookies?.[SESSION_COOKIE];
}

// Resolves the signed-in user on every request, so deactivation and role changes apply immediately.
export async function authenticate(req, _res, next) {
  try {
    const token = readToken(req);
    if (!token) throw new HttpError(401, 'Sign in required');
    let claims;
    try {
      claims = verifySession(token);
    } catch {
      throw new HttpError(401, 'Session expired or invalid');
    }
    const { rows } = await query(
      `SELECT u.id, u.org_id, u.email, u.name, u.role, o.name AS org_name
         FROM users u JOIN organizations o ON o.id = u.org_id
        WHERE u.id = $1 AND u.org_id = $2 AND u.session_version = $3 AND u.is_active`,
      [claims.sub, claims.org, claims.ver],
    );
    if (!rows[0]) throw new HttpError(401, 'Session expired or invalid');
    req.user = rows[0];
    next();
  } catch (err) {
    next(err);
  }
}

export const requirePermission = (permission) => (req, _res, next) => {
  if (!can(req.user?.role, permission)) return next(new HttpError(403, 'You do not have permission to do that'));
  next();
};

// Cookie sessions are CSRF-protected by SameSite=Lax, JSON-only bodies, and this origin check.
export const checkOrigin = (allowedOrigins) => (req, _res, next) => {
  if (['GET', 'HEAD', 'OPTIONS'].includes(req.method)) return next();
  const origin = req.get('origin');
  if (!origin) return next();
  const self = `${req.protocol}://${req.get('host')}`;
  if (origin === self || allowedOrigins.includes(origin)) return next();
  next(new HttpError(403, 'Cross-origin request blocked'));
};
