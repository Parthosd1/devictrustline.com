// Creates an organization and its first administrator, with no sample data.
// Usage: ORG_NAME='Acme' ADMIN_EMAIL=it@acme.com ADMIN_NAME='Pat Lee' ADMIN_PASSWORD='a long password' npm run create-admin
import { hashPassword } from '../lib/auth.js';
import { passwordSchema } from '../routes/auth.js';
import { migrate } from './migrate.js';
import { pool, withTransaction } from './pool.js';

const { ORG_NAME, ADMIN_EMAIL, ADMIN_NAME = 'Administrator', ADMIN_PASSWORD } = process.env;
if (!ORG_NAME || !ADMIN_EMAIL || !ADMIN_PASSWORD) {
  console.error('Set ORG_NAME, ADMIN_EMAIL and ADMIN_PASSWORD (and optionally ADMIN_NAME).');
  process.exit(1);
}
const check = passwordSchema.safeParse(ADMIN_PASSWORD);
if (!check.success) {
  console.error(check.error.issues[0].message);
  process.exit(1);
}

try {
  await migrate();
  const passwordHash = await hashPassword(ADMIN_PASSWORD);
  await withTransaction(async (db) => {
    const { rows: [org] } = await db.query('INSERT INTO organizations (name) VALUES ($1) RETURNING id', [ORG_NAME.trim()]);
    await db.query(
      `INSERT INTO users (org_id, email, name, password_hash, role) VALUES ($1, $2, $3, $4, 'admin')`,
      [org.id, ADMIN_EMAIL.trim(), ADMIN_NAME.trim(), passwordHash],
    );
  });
  console.log(`Created ${ORG_NAME} with administrator ${ADMIN_EMAIL}. Sign in and add your team under Users.`);
} catch (err) {
  console.error(err.code === '23505' ? `${ADMIN_EMAIL} already has an account.` : err.message);
  process.exitCode = 1;
} finally {
  await pool.end();
}
