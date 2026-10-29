// Phase 7: workspace event notifications with workspace defaults, per-person overrides, and
// delivery limited to people who may see the item.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import request from 'supertest';
import { app, day, lastEmail, newOrg, PASSWORD, pool, resetDatabase, runJobs, tokenFrom, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const byName = (items, name) => items.find((i) => i.name === name);
const settle = () => new Promise((r) => setTimeout(r, 250));
const kinds = async (who) => (await who.agent.get('/api/notifications')).body.items.map((n) => n.kind);
let n = 0;

async function setup() {
  const org = await newOrg();
  const invs = (await org.agent.get('/api/inventories')).body.items;
  const available = byName(invs, 'Available Inventory');
  const loaner = byName(invs, 'Loaner Inventory');
  const repair = byName(invs, 'RMA / Repair Inventory');
  const manager = await userWithRole(org.agent, 'manager');
  const tech = await userWithRole(org.agent, 'technician');
  const item = async (inventoryId, extra = {}) => (await org.agent.post('/api/assets')
    .send({ name: `Item ${++n}`, type: 'Laptop', serial: `BN-${Date.now()}-${n}`, inventoryId, ...extra }).expect(201)).body;
  const person = (await org.agent.post('/api/people').send({ name: 'Sam' }).expect(201)).body;
  return { ...org, available, loaner, repair, manager, tech, item, person };
}

test('preferences: workspace defaults, personal overrides and validation', async () => {
  const s = await setup();
  const prefs = (await s.manager.agent.get('/api/notification-settings/events').expect(200)).body.groups;
  assert.deepEqual(prefs.map((g) => g.group),
    ['transfers', 'access', 'audits', 'missing', 'loans', 'assignments', 'tasks', 'invitations']);
  assert.deepEqual(prefs[0].effective, { inApp: true, email: true, push: true, sms: true });
  assert.equal(prefs[0].own, null);

  await s.manager.agent.patch('/api/settings').send({ notifications: { transfers: { inApp: true, email: false, push: true, sms: false } } }).expect(403);
  await s.agent.patch('/api/settings').send({ notifications: { nope: { inApp: true, email: true, push: true, sms: true } } }).expect(400);
  await s.agent.patch('/api/settings').send({ notifications: { transfers: { inApp: true } } }).expect(400);
  await s.agent.patch('/api/settings').send({ notifications: { transfers: { inApp: true, email: false, push: true, sms: false } } }).expect(200);
  let t = (await s.manager.agent.get('/api/notification-settings/events')).body.groups[0];
  assert.deepEqual(t.workspace, { inApp: true, email: false, push: true, sms: false });
  assert.deepEqual(t.effective, t.workspace);

  const own = { inApp: false, email: false, push: false, sms: false };
  t = (await s.manager.agent.put('/api/notification-settings/events').send({ transfers: own }).expect(200)).body.groups[0];
  assert.deepEqual(t.effective, own);
  await s.manager.agent.put('/api/notification-settings/events').send({ bogus: own }).expect(400);
  await s.manager.agent.put('/api/notification-settings/events').send({ transfers: { inApp: 'yes' } }).expect(400);

  // With in-app off for transfers, the manager hears nothing about a transfer; the owner does.
  await s.agent.patch(`/api/inventories/${s.repair.id}`).send({ requiresApproval: true }).expect(200);
  const a = await s.item(s.available.id);
  await s.tech.agent.post('/api/transfers').send({ assetIds: [a.id], toInventoryId: s.repair.id }).expect(201);
  await settle();
  assert.ok(!(await kinds(s.manager)).includes('transfer_requested'));
  assert.ok((await kinds(s)).includes('transfer_requested'));
  // Back to the workspace default.
  t = (await s.manager.agent.put('/api/notification-settings/events').send({ transfers: null }).expect(200)).body.groups[0];
  assert.equal(t.own, null);
});

test('emails follow the workspace default for a group', async () => {
  const s = await setup();
  await s.agent.patch(`/api/inventories/${s.repair.id}`).send({ requiresApproval: true }).expect(200);
  await s.agent.patch('/api/settings').send({ notifications: { transfers: { inApp: true, email: false, push: false, sms: false } } }).expect(200);
  const a = await s.item(s.available.id);
  await s.tech.agent.post('/api/transfers').send({ assetIds: [a.id], toInventoryId: s.repair.id }).expect(201);
  await settle();
  await runJobs();
  const { rows } = await pool.query("SELECT count(*)::int AS c FROM email_outbox WHERE to_address = $1 AND subject LIKE 'Transfer%'", [s.manager.email]);
  assert.equal(rows[0].c, 0);
  assert.ok((await kinds(s.manager)).includes('transfer_requested'));
});

test('assignments, returns, loan returns, missing items and accepted invitations are announced', async () => {
  const s = await setup();
  await s.agent.patch(`/api/inventories/${s.available.id}`).send({ custodianId: s.manager.id }).expect(200);
  const owned = await s.item(s.available.id, { ownerId: s.tech.id });
  await s.agent.post(`/api/assets/${owned.id}/assign`).send({ personId: s.person.id }).expect(200);
  await s.agent.post(`/api/assets/${owned.id}/return`).send({}).expect(200);
  await settle();
  assert.ok((await kinds(s.tech)).includes('item_assigned'), 'the item owner hears');
  assert.ok((await kinds(s.manager)).includes('item_returned'), 'the custodian hears');
  assert.ok(!(await kinds(s)).includes('item_assigned'), 'not the person who did it');

  const lend = await s.item(s.loaner.id);
  const loan = (await s.tech.agent.post('/api/loans').send({ assetId: lend.id, personId: s.person.id, dueOn: day(3), deployNow: true }).expect(201)).body;
  await s.agent.post(`/api/loans/${loan.id}/return`).send({ damaged: true, note: 'Bent' }).expect(200);
  await settle();
  const notes = (await s.tech.agent.get('/api/notifications')).body.items.filter((x) => x.kind === 'loan_returned');
  assert.equal(notes.length, 1);
  assert.match(notes[0].body, /reported damaged/);

  await s.item(s.available.id);
  const soft = (await s.agent.post('/api/audits').send({ auditType: 'soft', inventoryId: s.available.id }).expect(201)).body;
  await s.agent.post(`/api/audits/${soft.id}/close`).expect(200);
  await settle();
  assert.ok((await kinds(s.manager)).includes('audit_missing'));

  const email = `joiner${Date.now()}@example.com`;
  await s.manager.agent.post('/api/invitations').send({ email, role: 'viewer' }).expect(403);
  await s.agent.post('/api/invitations').send({ email, role: 'viewer' }).expect(201);
  const token = tokenFrom((await lastEmail(email)).body, 'invite');
  await request(app).post('/api/invitations/accept').send({ token, name: 'Jo', password: PASSWORD }).expect(200);
  await settle();
  assert.ok((await kinds(s)).includes('invitation_accepted'));
});

test('people never hear about items outside their access', async () => {
  const s = await setup();
  await s.agent.patch(`/api/inventories/${s.repair.id}`).send({ requiresApproval: true }).expect(200);
  const scopedMgr = await userWithRole(s.agent, 'manager');
  await s.agent.patch(`/api/users/${scopedMgr.id}`).send({ accessScope: 'inventories', inventoryIds: [s.loaner.id] }).expect(200);
  const a = await s.item(s.available.id);
  await s.tech.agent.post('/api/transfers').send({ assetIds: [a.id], toInventoryId: s.repair.id }).expect(201);
  await settle();
  assert.ok(!(await kinds(scopedMgr)).includes('transfer_requested'));
  assert.ok((await kinds(s.manager)).includes('transfer_requested'));
});
