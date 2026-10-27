// Black-box tests: every test talks over HTTP to a running API server, started by
// api/scripts/test-http.sh, which also sets the variables read here.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import pg from 'pg';
import request from 'supertest';

const run = promisify(execFile);
const databaseUrl = process.env.TEST_DATABASE_URL
  || 'postgres://postgres:postgres@localhost:5432/devicetrustline_test';

export const app = process.env.TEST_API_URL;
if (!app) throw new Error('Run these tests with api/scripts/test-http.sh');

// Direct database access, for checks the API itself would never allow.
export const pool = new pg.Pool({ connectionString: databaseUrl });

const server = (...args) => run(process.env.TEST_API_BIN, args, { env: { ...process.env, DATABASE_URL: databaseUrl, MAIL_MODE: 'capture' } });

export async function resetDatabase() {
  await pool.query('DROP SCHEMA public CASCADE; CREATE SCHEMA public;');
  await server('migrate');
}

// Opens due automatic audits, as the server's scheduler does every 15 minutes.
export async function runScheduledAudits() {
  const { stdout } = await server('run-scheduled-audits');
  return JSON.parse(stdout);
}

let counter = 0;
export const PASSWORD = 'correct horse battery staple';

// Signs up a fresh organization and returns a cookie-carrying agent for its admin.
export async function newOrg(orgName = `Org ${++counter}`) {
  const agent = request.agent(app);
  const email = `admin${++counter}-${Date.now()}@example.com`;
  const res = await agent.post('/api/auth/signup')
    .send({ organizationName: orgName, name: 'Admin', email, password: PASSWORD });
  if (res.status !== 201) throw new Error(`signup failed: ${res.status} ${JSON.stringify(res.body)}`);
  return { agent, email, user: res.body.user };
}

// Creates a user with the given role in the admin's org and returns a signed-in agent for them.
export async function userWithRole(adminAgent, role) {
  const email = `${role}${++counter}-${Date.now()}@example.com`;
  const res = await adminAgent.post('/api/users').send({ name: role, email, role, password: PASSWORD });
  if (res.status !== 201) throw new Error(`create user failed: ${res.status} ${JSON.stringify(res.body)}`);
  const agent = request.agent(app);
  await agent.post('/api/auth/login').send({ email, password: PASSWORD }).expect(200);
  return { agent, email, id: res.body.id };
}

// Runs background jobs once (email delivery, reminders), as the server does every few seconds.
export async function runJobs() {
  const { stdout } = await server('run-jobs');
  return JSON.parse(stdout);
}

// The latest captured email to an address (MAIL_MODE=capture), after delivering due mail.
export async function lastEmail(to) {
  await runJobs();
  const { rows } = await pool.query(
    'SELECT subject, body_text AS body, status, kind FROM email_outbox WHERE to_address = $1 ORDER BY id DESC LIMIT 1', [to]);
  return rows[0];
}

export const tokenFrom = (body, param) => new URL(body.match(/https?:\/\/\S+/)[0]).searchParams.get(param);

// Signs up a personal account and returns its agent.
export async function newPersonal(name = 'Pat') {
  const agent = request.agent(app);
  const email = `person${++counter}-${Date.now()}@example.com`;
  const res = await agent.post('/api/auth/signup').send({ accountType: 'personal', name, email, password: PASSWORD });
  if (res.status !== 201) throw new Error(`signup failed: ${res.status} ${JSON.stringify(res.body)}`);
  return { agent, email, user: res.body.user };
}

export const day = (n) => new Date(Date.now() + n * 86400000).toISOString().slice(0, 10);
