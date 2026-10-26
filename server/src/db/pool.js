import pg from 'pg';
import { config } from '../config.js';

// Return DATE columns as 'YYYY-MM-DD' strings instead of timezone-shifted Date objects.
pg.types.setTypeParser(1082, (value) => value);

// DATABASE_SSL=true for managed databases that require TLS; 'no-verify' when the provider's certificate
// is not publicly trusted (some hosted Postgres plans). Leave unset on a private network.
const ssl = { true: { rejectUnauthorized: true }, 'no-verify': { rejectUnauthorized: false } }[config.databaseSsl];

export const pool = new pg.Pool({ connectionString: config.databaseUrl, ssl, max: config.databasePoolSize });
pool.on('error', (err) => console.error('idle database connection error', err.message));

export const query = (text, params) => pool.query(text, params);

// Runs fn inside a transaction with a dedicated client.
export async function withTransaction(fn) {
  const client = await pool.connect();
  try {
    await client.query('BEGIN');
    const result = await fn(client);
    await client.query('COMMIT');
    return result;
  } catch (err) {
    await client.query('ROLLBACK');
    throw err;
  } finally {
    client.release();
  }
}
