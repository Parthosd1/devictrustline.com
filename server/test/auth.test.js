import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import request from 'supertest';
import { PASSWORD, app, newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

test('signup creates an organization admin and a session', async () => {
  const { agent, user } = await newOrg('Acme');
  assert.equal(user.role, 'admin');
  assert.equal(user.organization.name, 'Acme');
  assert.ok(user.permissions.includes('users:manage'));
  const me = await agent.get('/api/auth/me').expect(200);
  assert.equal(me.body.user.id, user.id);
});

test('session cookie is httpOnly and SameSite', async () => {
  const res = await request(app).post('/api/auth/signup')
    .send({ organizationName: 'Cookie Co', name: 'A', email: 'cookie@example.com', password: PASSWORD })
    .expect(201);
  const cookie = res.headers['set-cookie'][0];
  assert.match(cookie, /dt_session=/);
  assert.match(cookie, /HttpOnly/);
  assert.match(cookie, /SameSite=Lax/);
  assert.ok(!JSON.stringify(res.body).includes('password'));
});

test('short passwords and duplicate emails are rejected', async () => {
  await request(app).post('/api/auth/signup')
    .send({ organizationName: 'X', name: 'A', email: 'short@example.com', password: 'short' }).expect(400);
  const { email } = await newOrg();
  await request(app).post('/api/auth/signup')
    .send({ organizationName: 'X', name: 'A', email: email.toUpperCase(), password: PASSWORD }).expect(409);
});

test('login works with the right password only', async () => {
  const { email } = await newOrg();
  const bad = await request(app).post('/api/auth/login').send({ email, password: 'wrong password!!' }).expect(401);
  const unknown = await request(app).post('/api/auth/login').send({ email: 'nobody@example.com', password: PASSWORD }).expect(401);
  assert.equal(bad.body.error, unknown.body.error);
  await request(app).post('/api/auth/login').send({ email: email.toUpperCase(), password: PASSWORD }).expect(200);
});

test('protected routes need a session', async () => {
  await request(app).get('/api/assets').expect(401);
  await request(app).get('/api/assets').set('Authorization', 'Bearer not-a-token').expect(401);
  await request(app).get('/api/auth/me').expect(401);
});

test('logout clears the session cookie', async () => {
  const { agent } = await newOrg();
  await agent.post('/api/auth/logout').expect(204);
  await agent.get('/api/auth/me').expect(401);
});

test('deactivated users lose access immediately', async () => {
  const { agent: admin } = await newOrg();
  const tech = await userWithRole(admin, 'technician');
  await tech.agent.get('/api/assets').expect(200);
  await admin.patch(`/api/users/${tech.id}`).send({ isActive: false }).expect(200);
  await tech.agent.get('/api/assets').expect(401);
  await request(app).post('/api/auth/login').send({ email: tech.email, password: PASSWORD }).expect(401);
});

test('changing a password ends other sessions', async () => {
  const { agent, email } = await newOrg();
  const other = request.agent(app);
  await other.post('/api/auth/login').send({ email, password: PASSWORD }).expect(200);
  await agent.post('/api/auth/change-password')
    .send({ currentPassword: 'wrong', newPassword: 'another long password' }).expect(400);
  await agent.post('/api/auth/change-password')
    .send({ currentPassword: PASSWORD, newPassword: 'another long password' }).expect(204);
  await agent.get('/api/auth/me').expect(200);
  await other.get('/api/auth/me').expect(401);
});

test('cross-origin writes are blocked', async () => {
  const { agent } = await newOrg();
  await agent.post('/api/buildings').set('Origin', 'https://evil.example').send({ name: 'HQ' }).expect(403);
  await agent.post('/api/buildings').set('Origin', 'http://localhost:5173').send({ name: 'HQ' }).expect(201);
});
