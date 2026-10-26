// Creates a demo organization with the same sample inventory the frontend MVP ships with.
// Usage: SEED_ADMIN_EMAIL=you@example.com SEED_ADMIN_PASSWORD='a long password' npm run seed
import { fileURLToPath } from 'node:url';
import { hashPassword } from '../lib/auth.js';
import { migrate } from './migrate.js';
import { pool, withTransaction } from './pool.js';

const SAMPLE = [
  ['Dell Latitude 5440', 'Laptop', 'DL5440-9921', 'Building A', 'IT Storage', 'Available'],
  ['Zebra DS3678', 'Scanner', 'ZB-3678-112', 'Building A', 'Station 214-02', 'Deployed'],
  ['HP EliteDesk 800', 'Desktop', 'HP800-4218', 'Building A', 'Station 214-02', 'Deployed'],
  ['Dell P2422H', 'Monitor', 'DP24-9922', 'Building A', 'Station 214-02', 'Deployed'],
  ['Honeywell 1950g', 'Scanner', 'HW-1950-555', 'Building B', 'Station 4151', 'Maintenance'],
  ['Lenovo ThinkPad T14', 'Laptop', 'LT14-2180', 'Building B', 'Loaner Storage', 'Available'],
  ['Logitech C920', 'Accessory', 'LC920-001', 'Building A', 'Station 214-02', 'Deployed'],
  ['Zebra TC52', 'Scanner', 'TC52-8290', 'Building B', 'Station 4151', 'Deployed'],
];

export async function seed({ email, password, orgName = 'Demo Workspace', name = 'Demo Admin' }) {
  return withTransaction(async (db) => {
    const { rows: [org] } = await db.query('INSERT INTO organizations (name) VALUES ($1) RETURNING id', [orgName]);
    const { rows: [admin] } = await db.query(
      `INSERT INTO users (org_id, email, name, password_hash, role) VALUES ($1, $2, $3, $4, 'admin') RETURNING id`,
      [org.id, email, name, await hashPassword(password)],
    );
    const buildings = new Map();
    const locations = new Map();
    for (const [assetName, type, serial, building, location, status] of SAMPLE) {
      if (!buildings.has(building)) {
        const { rows: [b] } = await db.query('INSERT INTO buildings (org_id, name) VALUES ($1, $2) RETURNING id', [org.id, building]);
        buildings.set(building, b.id);
      }
      const key = `${building}/${location}`;
      if (!locations.has(key)) {
        const { rows: [l] } = await db.query(
          'INSERT INTO locations (org_id, building_id, name) VALUES ($1, $2, $3) RETURNING id',
          [org.id, buildings.get(building), location],
        );
        locations.set(key, l.id);
      }
      const { rows: [{ n }] } = await db.query(
        'UPDATE organizations SET next_asset_number = next_asset_number + 1 WHERE id = $1 RETURNING next_asset_number - 1 AS n',
        [org.id],
      );
      const { rows: [a] } = await db.query(
        `INSERT INTO assets (org_id, asset_tag, name, type, serial, status, building_id, location_id, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id`,
        [org.id, `DT-${n}`, assetName, type, serial, status, buildings.get(building), locations.get(key), admin.id],
      );
      await db.query(
        `INSERT INTO asset_events (org_id, asset_id, actor_id, event_type, changes) VALUES ($1, $2, $3, 'created', '{"source":{"from":null,"to":"seed"}}')`,
        [org.id, a.id, admin.id],
      );
    }
    return { orgId: org.id, adminId: admin.id };
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const email = process.env.SEED_ADMIN_EMAIL;
  const password = process.env.SEED_ADMIN_PASSWORD;
  if (!email || !password || password.length < 12) {
    console.error('Set SEED_ADMIN_EMAIL and SEED_ADMIN_PASSWORD (12+ characters).');
    process.exit(1);
  }
  migrate()
    .then(() => seed({ email: email.toLowerCase(), password }))
    .then(() => console.log(`Seeded demo workspace. Sign in as ${email}.`))
    .catch((err) => { console.error(err.message); process.exitCode = 1; })
    .finally(() => pool.end());
}
