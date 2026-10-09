const env = process.env;
const isProduction = env.NODE_ENV === 'production';

function required(name, fallback) {
  const value = env[name] ?? (isProduction ? undefined : fallback);
  if (!value) throw new Error(`Missing required environment variable ${name}`);
  return value;
}

const jwtSecret = required('JWT_SECRET', 'dev-only-secret-change-me-dev-only-secret');
if (isProduction && jwtSecret.length < 32) {
  throw new Error('JWT_SECRET must be at least 32 characters in production');
}

export const config = {
  isProduction,
  port: Number(env.PORT || 4000),
  databaseUrl: required('DATABASE_URL', 'postgres://postgres:postgres@localhost:5432/devicetrustline'),
  databaseSsl: env.DATABASE_SSL || null,
  databasePoolSize: Number(env.DATABASE_POOL_SIZE || 10),
  jwtSecret,
  sessionHours: Number(env.SESSION_HOURS || 8),
  // Browser origins allowed to make state-changing requests with the session cookie.
  appOrigins: (env.APP_ORIGIN || 'http://localhost:5173').split(',').map((o) => o.trim()).filter(Boolean),
  // Self-service workspace sign-up. Turn off once your organization is set up.
  allowSignup: (env.ALLOW_SIGNUP ?? (isProduction ? 'false' : 'true')) === 'true',
  trustProxy: env.TRUST_PROXY === 'true',
  // Directory holding the built web app (vite build output). When set, the API also serves the app.
  staticDir: env.STATIC_DIR || null,
  logRequests: (env.LOG_REQUESTS ?? (isProduction ? 'true' : 'false')) === 'true',
};
