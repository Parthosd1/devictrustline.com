import request from 'supertest';

process.env.DATABASE_URL = process.env.TEST_DATABASE_URL
  || 'postgres://postgres:postgres@localhost:5432/devicetrustline_test';
process.env.AUTH_RATE_LIMIT ??= '1000';
process.env.ALLOW_SIGNUP = 'true';

const { pool } = await import('../src/db/pool.js');
const { migrate } = await import('../src/db/migrate.js');
const { createApp } = await import('../src/app.js');

export { pool };
export const app = createApp();

export async function resetDatabase() {
  await pool.query('DROP SCHEMA public CASCADE; CREATE SCHEMA public;');
  await migrate({ log: () => {} });
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
