import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import request from 'supertest';
import { PASSWORD, app, lastEmail, newOrg, pool, resetDatabase, runJobs, tokenFrom, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

let n = 0;
const email = (p = 'person') => `${p}${++n}-${Date.now()}@example.com`;

test('auth config tells the app what is available', async () => {
  const res = await request(app).get('/api/auth/config').expect(200);
  assert.equal(res.body.signup, true);
  assert.equal(res.body.google, true, "the test server runs with a stand-in Google");
  assert.equal(res.body.email, true);
});

test('personal sign-up needs no organization name and creates a personal workspace', async () => {
  const agent = request.agent(app);
  const res = await agent.post('/api/auth/signup')
    .send({ accountType: 'personal', name: 'Sam', email: email(), password: PASSWORD }).expect(201);
  assert.equal(res.body.user.organization.kind, 'personal');
  assert.equal(res.body.user.organization.name, "Sam's things");
  assert.equal(res.body.user.role, 'admin');
  // Business sign-up still needs a name.
  await request(app).post('/api/auth/signup')
    .send({ accountType: 'business', name: 'B', email: email(), password: PASSWORD }).expect(400);
});

test('email verification links work once', async () => {
  const address = email();
  const agent = request.agent(app);
  const res = await agent.post('/api/auth/signup').send({ accountType: 'personal', name: 'V', email: address, password: PASSWORD });
  assert.equal(res.body.user.emailVerified, false);
  const mail = await lastEmail(address);
  assert.equal(mail.kind, 'verify_email');
  assert.equal(mail.status, 'sent');
  const token = tokenFrom(mail.body, 'verify');
  await request(app).post('/api/auth/verify-email').send({ token }).expect(200);
  const again = await request(app).post('/api/auth/verify-email').send({ token }).expect(400);
  assert.equal(again.body.code, 'token_invalid');
  const me = await agent.get('/api/auth/me').expect(200);
  assert.equal(me.body.user.emailVerified, true);
});

test('password reset: same answer for unknown accounts, single-use, expires, signs out old sessions', async () => {
  const { agent, email: address } = await newOrg();
  const unknown = await request(app).post('/api/auth/password-reset/request').send({ email: 'nobody@example.com' }).expect(202);
  const known = await request(app).post('/api/auth/password-reset/request').send({ email: address }).expect(202);
  assert.deepEqual(unknown.body, known.body);
  const token = tokenFrom((await lastEmail(address)).body, 'reset');
  await request(app).post('/api/auth/password-reset/confirm').send({ token, password: 'short' }).expect(400);
  await request(app).post('/api/auth/password-reset/confirm').send({ token, password: 'a brand new password' }).expect(204);
  await request(app).post('/api/auth/password-reset/confirm').send({ token, password: 'another new password' }).expect(400);
  await agent.get('/api/auth/me').expect(401);
  await request(app).post('/api/auth/login').send({ email: address, password: 'a brand new password' }).expect(200);

  await request(app).post('/api/auth/password-reset/request').send({ email: address }).expect(202);
  const expired = tokenFrom((await lastEmail(address)).body, 'reset');
  await pool.query("UPDATE auth_tokens SET expires_at = now() - interval '1 minute' WHERE purpose = 'reset_password' AND used_at IS NULL");
  await request(app).post('/api/auth/password-reset/confirm').send({ token: expired, password: 'a third new password' }).expect(400);
});

test('a person can belong to several workspaces and switch between them', async () => {
  const { agent, user } = await newOrg('First Co');
  const created = await agent.post('/api/workspaces').send({ name: 'Home', kind: 'personal' }).expect(201);
  const list = await agent.get('/api/workspaces').expect(200);
  assert.equal(list.body.items.length, 2);
  assert.equal(list.body.items.find((w) => w.current).id, user.organization.id);

  await agent.post('/api/assets').send({ name: 'Work laptop', type: 'Laptop' }).expect(201);
  const switched = await agent.post('/api/auth/switch').send({ workspaceId: created.body.id }).expect(200);
  assert.equal(switched.body.user.organization.kind, 'personal');
  const assets = await agent.get('/api/assets').expect(200);
  assert.equal(assets.body.items.length, 0, 'items never leak between workspaces');

  // Signing in again returns to the workspace used last.
  await agent.post('/api/auth/logout').expect(204);
  const login = await agent.post('/api/auth/login').send({ email: user.email, password: PASSWORD }).expect(200);
  assert.equal(login.body.user.organization.id, created.body.id);
});

test('nobody can switch into a workspace they do not belong to', async () => {
  const { agent } = await newOrg();
  const other = await newOrg();
  await agent.post('/api/auth/switch').send({ workspaceId: other.user.organization.id }).expect(404);
});

test('invitations: new accounts join with the invited role, links work once', async () => {
  const { agent: admin } = await newOrg('Inviters');
  const address = email('invitee');
  const inv = await admin.post('/api/invitations').send({ email: address, role: 'technician' }).expect(201);
  assert.equal(inv.body.emailed, true);
  assert.equal(inv.body.link, undefined, 'links only go to the inbox when email works');
  const list = await admin.get('/api/invitations').expect(200);
  assert.equal(list.body.items.length, 1);

  const token = tokenFrom((await lastEmail(address)).body, 'invite');
  const lookup = await request(app).get(`/api/invitations/lookup?token=${token}`).expect(200);
  assert.equal(lookup.body.workspace, 'Inviters');
  assert.equal(lookup.body.accountExists, false);

  const agent = request.agent(app);
  const accepted = await agent.post('/api/invitations/accept').send({ token, name: 'Tess', password: PASSWORD }).expect(200);
  assert.equal(accepted.body.user.role, 'technician');
  assert.equal(accepted.body.user.emailVerified, true);
  await request(app).post('/api/invitations/accept').send({ token, name: 'X', password: PASSWORD }).expect(400);
  assert.equal((await admin.get('/api/invitations')).body.items.length, 0);
});

test('invitations to existing accounts need that account signed in', async () => {
  const { agent: admin } = await newOrg('Second Home');
  const existing = await newOrg('Their Own Co');
  await admin.post('/api/invitations').send({ email: existing.email, role: 'viewer' }).expect(201);
  const token = tokenFrom((await lastEmail(existing.email)).body, 'invite');

  // Someone else (not signed in, or signed in as another account) cannot use it.
  const anon = await request(app).post('/api/invitations/accept').send({ token, name: 'Mallory', password: PASSWORD }).expect(409);
  assert.equal(anon.body.code, 'sign_in_required');
  const stranger = await newOrg();
  await admin.post('/api/invitations').send({ email: existing.email, role: 'viewer' }).expect(201);
  const token2 = tokenFrom((await lastEmail(existing.email)).body, 'invite');
  await stranger.agent.post('/api/invitations/accept').send({ token: token2 }).expect(409);

  await admin.post('/api/invitations').send({ email: existing.email, role: 'viewer' }).expect(201);
  const token3 = tokenFrom((await lastEmail(existing.email)).body, 'invite');
  const res = await existing.agent.post('/api/invitations/accept').send({ token: token3 }).expect(200);
  assert.equal(res.body.user.organization.name, 'Second Home');
  assert.equal(res.body.user.role, 'viewer');
  const spaces = await existing.agent.get('/api/workspaces').expect(200);
  assert.equal(spaces.body.items.length, 2);
});

test('a new invitation replaces the old one, and admins can revoke them', async () => {
  const { agent: admin } = await newOrg();
  const address = email();
  await admin.post('/api/invitations').send({ email: address, role: 'viewer' }).expect(201);
  const first = tokenFrom((await lastEmail(address)).body, 'invite');
  await admin.post('/api/invitations').send({ email: address, role: 'auditor' }).expect(201);
  await request(app).get(`/api/invitations/lookup?token=${first}`).expect(404);
  const [pending] = (await admin.get('/api/invitations')).body.items;
  assert.equal(pending.role, 'auditor');
  await admin.delete(`/api/invitations/${pending.id}`).expect(204);
  assert.equal((await admin.get('/api/invitations')).body.items.length, 0);
});

test('only admins invite, and invitations stay in their workspace', async () => {
  const { agent: admin } = await newOrg();
  const manager = await userWithRole(admin, 'manager');
  await manager.agent.post('/api/invitations').send({ email: email(), role: 'viewer' }).expect(403);
  await admin.post('/api/invitations').send({ email: email(), role: 'viewer' }).expect(201);
  const other = await newOrg();
  const [pending] = (await admin.get('/api/invitations')).body.items;
  await other.agent.delete(`/api/invitations/${pending.id}`).expect(404);
});

test('adding an existing account as a user points the admin to invitations', async () => {
  const { agent: admin } = await newOrg();
  const existing = await newOrg();
  const res = await admin.post('/api/users').send({ name: 'X', email: existing.email, role: 'viewer', password: PASSWORD }).expect(409);
  assert.equal(res.body.code, 'account_exists');
});

test('admins cannot change the password of someone who also uses other workspaces', async () => {
  const { agent: admin } = await newOrg('Shared');
  const other = await newOrg('Theirs');
  await admin.post('/api/invitations').send({ email: other.email, role: 'viewer' }).expect(201);
  const token = tokenFrom((await lastEmail(other.email)).body, 'invite');
  await other.agent.post('/api/invitations/accept').send({ token }).expect(200);
  await admin.patch(`/api/users/${other.user.id}`).send({ password: 'taken over password' }).expect(403);
  // Deactivating them here only affects this workspace.
  await admin.patch(`/api/users/${other.user.id}`).send({ isActive: false }).expect(200);
  await other.agent.get('/api/assets').expect(401);
  const login = await request(app).post('/api/auth/login').send({ email: other.email, password: PASSWORD }).expect(200);
  assert.equal(login.body.user.organization.name, 'Theirs');
});

test('email delivery retries rejected messages and never marks them sent', async () => {
  const { agent: admin } = await newOrg();
  await admin.post('/api/invitations').send({ email: 'bounce@example.com', role: 'viewer' }).expect(201);
  await runJobs();
  const { rows: [row] } = await pool.query("SELECT status, attempts, last_error, sent_at FROM email_outbox WHERE to_address = 'bounce@example.com'");
  assert.equal(row.status, 'queued');
  assert.equal(row.attempts, 1);
  assert.equal(row.sent_at, null);
  assert.ok(row.last_error);
  await pool.query("UPDATE email_outbox SET attempts = 5, next_attempt_at = now() WHERE to_address = 'bounce@example.com'");
  await runJobs();
  const { rows: [after6] } = await pool.query("SELECT status FROM email_outbox WHERE to_address = 'bounce@example.com'");
  assert.equal(after6.status, 'failed');
});

test('account settings and the dev mailbox', async () => {
  const { agent, email: address } = await newOrg();
  const acct = await agent.patch('/api/account').send({ name: 'Renamed', emailNotifications: false }).expect(200);
  assert.equal(acct.body.name, 'Renamed');
  assert.equal(acct.body.emailNotifications, false);
  assert.equal(acct.body.googleLinked, false);
  await runJobs();
  const box = await request(app).get(`/api/dev/mailbox?to=${encodeURIComponent(address)}`).expect(200);
  assert.ok(box.body.items.length >= 1);
});
