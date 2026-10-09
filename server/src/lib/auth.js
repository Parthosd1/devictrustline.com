import bcrypt from 'bcryptjs';
import jwt from 'jsonwebtoken';
import { config } from '../config.js';

export const SESSION_COOKIE = 'dt_session';
const BCRYPT_ROUNDS = 12;

export const hashPassword = (password) => bcrypt.hash(password, BCRYPT_ROUNDS);
export const verifyPassword = (password, hash) => bcrypt.compare(password, hash);

// Compared against when an email is unknown, so login timing does not reveal which emails exist.
export const DUMMY_HASH = bcrypt.hashSync('not-a-real-password', BCRYPT_ROUNDS);

export function signSession(user) {
  return jwt.sign({ sub: user.id, org: user.org_id, ver: user.session_version }, config.jwtSecret, {
    expiresIn: `${config.sessionHours}h`,
    algorithm: 'HS256',
  });
}

export function verifySession(token) {
  return jwt.verify(token, config.jwtSecret, { algorithms: ['HS256'] });
}

export const sessionCookieOptions = () => ({
  httpOnly: true,
  sameSite: 'lax',
  secure: config.isProduction,
  maxAge: config.sessionHours * 3600 * 1000,
  path: '/',
});
