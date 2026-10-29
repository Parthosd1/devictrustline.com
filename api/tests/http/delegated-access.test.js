// Phase 3: owners, access limited to selected inventories, invitations carrying that access,
// and access requests. Every check is made by the API; the client is never trusted.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import request from 'supertest';
import { app, lastEmail, newOrg, PASSWORD, pool, resetDatabase, tokenFrom, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const byName = (items, name) => items.find((i) => i.name === name);

// A workspace with a technician inventory, an item in it and an item elsewhere.
async function setup() {
  const org = await newOrg();
  const invs = (await org.agent.get('/api/inventories')).body.items;
  const hub = byName(invs, 'Main IT Hub');
  const available = byName(invs, 'Available Inventory');
  const techA = (await org.agent.post('/api/inventories')
    .send({ name: 'Technician A Inventory', kind: 'technician', parentId: hub.id }).expect(201)).body;
  const techBin = (await org.agent.post('/api/inventories').send({ name: 'Bin 1', parentId: techA.id }).expect(201)).body;
  const inside = (await org.agent.post('/api/assets')
    .send({ name: 'Tech laptop', type: 'Laptop', serial: `IN-${Date.now()}`, inventoryId: techBin.id }).expect(201)).body;
  const outside = (await org.agent.post('/api/assets')
    .send({ name: 'Spare laptop', type: 'Laptop', serial: `OUT-${Date.now()}`, inventoryId: available.id }).expect(201)).body;
  return { ...org, hub, available, techA, techBin, inside, outside };
}

async function scopedUser(owner, role, inventoryIds) {
  const u = await userWithRole(owner, role);
  const res = await owner.patch(`/api/users/${u.id}`).send({ accessScope: 'inventories', inventoryIds }).expect(200);
  assert.equal(res.body.accessScope, 'inventories');
  return u;
}

test('workflow 3: a technician limited to Technician A Inventory sees and changes only what is inside it', async () => {
  const s = await setup();
  const tech = await scopedUser(s.agent, 'technician', [s.techA.id]);
  const me = (await tech.agent.get('/api/auth/me').expect(200)).body.user;
  assert.equal(me.access.scope, 'inventories');
  assert.deepEqual(new Set(me.access.inventoryIds), new Set([s.techA.id, s.techBin.id]), 'sub-inventories are included');

  // Lists show only the granted inventories and their items.
  const invs = (await tech.agent.get('/api/inventories').expect(200)).body;
  assert.deepEqual(invs.items.map((i) => i.name).sort(), ['Bin 1', 'Technician A Inventory']);
  assert.equal(invs.unassigned, 0);
  const assets = (await tech.agent.get('/api/assets').expect(200)).body.items;
  assert.deepEqual(assets.map((a) => a.id), [s.inside.id]);
  const csv = (await tech.agent.get('/api/assets/export').expect(200)).text;
  assert.ok(csv.includes('Tech laptop') && !csv.includes('Spare laptop'));

  // Anything outside answers exactly like something that does not exist.
  await tech.agent.get(`/api/assets/${s.outside.id}`).expect(404);
  await tech.agent.patch(`/api/assets/${s.outside.id}`).send({ name: 'x' }).expect(404);
  await tech.agent.get(`/api/inventories/${s.available.id}`).expect(404);
  await tech.agent.get(`/api/assets/lookup?code=${encodeURIComponent(s.outside.serial)}`).expect(404);
  await tech.agent.post('/api/work-orders').send({ assetId: s.outside.id, title: 'Fix' }).expect(400);
  await tech.agent.post('/api/reminders')
    .send({ title: 'x', kind: 'custom', dueOn: '2030-01-01', offsets: [1], assetId: s.outside.id }).expect(400);

  // Inside, normal work goes on.
  await tech.agent.get(`/api/assets/${s.inside.id}`).expect(200);
  await tech.agent.patch(`/api/assets/${s.inside.id}`).send({ name: 'Tech laptop 2' }).expect(200);
  await tech.agent.post('/api/work-orders').send({ assetId: s.inside.id, title: 'Battery' }).expect(201);
  const created = await tech.agent.post('/api/assets')
    .send({ name: 'New mouse', type: 'Monitor', inventoryId: s.techBin.id }).expect(201);
  assert.equal(created.body.inventoryName, 'Bin 1');
  await tech.agent.post('/api/assets').send({ name: 'Nowhere', type: 'Monitor' }).expect(400);
  await tech.agent.post('/api/assets').send({ name: 'Elsewhere', type: 'Monitor', inventoryId: s.available.id }).expect(400);

  // Transfers stay inside the granted inventories.
  await tech.agent.post('/api/transfers').send({ assetIds: [s.inside.id], toInventoryId: s.techA.id }).expect(201);
  await tech.agent.post('/api/transfers').send({ assetIds: [s.inside.id], toInventoryId: s.available.id }).expect(400);
  await tech.agent.post('/api/transfers').send({ assetIds: [s.outside.id], toInventoryId: s.techA.id }).expect(404);
  // Moving an item out is done by someone with wider access; then it is gone from view.
  await s.agent.post('/api/transfers').send({ assetIds: [s.inside.id], toInventoryId: s.available.id }).expect(201);
  await tech.agent.get(`/api/assets/${s.inside.id}`).expect(404);
  const transfers = (await tech.agent.get('/api/transfers').expect(200)).body.items;
  assert.ok(transfers.length >= 1, 'transfers into or out of their inventories stay visible');

  // Workspace-wide areas are closed to limited access.
  await tech.agent.get('/api/reports/summary').expect(403);
  await tech.agent.get('/api/insights').expect(403);
  await tech.agent.post('/api/assets/import').send({}).expect(403);
  await tech.agent.post('/api/categories').send({ name: 'x' }).expect(403);
});

test('a manager limited to some inventories manages only inside them', async () => {
  const s = await setup();
  const mgr = await scopedUser(s.agent, 'manager', [s.techA.id]);
  await mgr.agent.post('/api/inventories').send({ name: 'Bin 2', parentId: s.techA.id }).expect(201);
  await mgr.agent.post('/api/inventories').send({ name: 'Top level' }).expect(400);
  await mgr.agent.post('/api/inventories').send({ name: 'Under hub', parentId: s.hub.id }).expect(400);
  await mgr.agent.patch(`/api/inventories/${s.techBin.id}`).send({ parentId: s.hub.id }).expect(400);
  await mgr.agent.patch(`/api/inventories/${s.available.id}`).send({ name: 'Mine now' }).expect(404);
});

test('owners and admins cannot be limited; role changes to admin widen access', async () => {
  const s = await setup();
  const admin = await userWithRole(s.agent, 'admin');
  await s.agent.patch(`/api/users/${admin.id}`).send({ accessScope: 'inventories', inventoryIds: [s.techA.id] }).expect(400);
  await s.agent.patch(`/api/users/${s.user.id}`).send({ accessScope: 'inventories', inventoryIds: [s.techA.id] }).expect(400);
  const tech = await scopedUser(s.agent, 'technician', [s.techA.id]);
  await s.agent.patch(`/api/users/${tech.id}`).send({ accessScope: 'inventories', inventoryIds: [] }).expect(400);
  await s.agent.patch(`/api/users/${tech.id}`)
    .send({ accessScope: 'inventories', inventoryIds: ['00000000-0000-4000-8000-000000000000'] }).expect(400);
  const promoted = (await s.agent.patch(`/api/users/${tech.id}`).send({ role: 'admin' }).expect(200)).body;
  assert.equal(promoted.accessScope, 'all');
  assert.deepEqual(promoted.inventoryIds, []);
  await tech.agent.get(`/api/assets/${s.outside.id}`).expect(200);
  // Access changes take effect on the next request, without signing in again.
  const back = (await s.agent.patch(`/api/users/${tech.id}`)
    .send({ role: 'technician', accessScope: 'inventories', inventoryIds: [s.techBin.id] }).expect(200)).body;
  assert.deepEqual(back.inventoryIds, [s.techBin.id]);
  await tech.agent.get(`/api/assets/${s.outside.id}`).expect(404);
  await tech.agent.get(`/api/inventories/${s.techA.id}`).expect(404);
  await tech.agent.get(`/api/inventories/${s.techBin.id}`).expect(200);
});

test('viewers and auditors stay read-only even inside their inventories', async () => {
  const s = await setup();
  const viewer = await scopedUser(s.agent, 'viewer', [s.techA.id]);
  await viewer.agent.get(`/api/assets/${s.inside.id}`).expect(200);
  await viewer.agent.patch(`/api/assets/${s.inside.id}`).send({ name: 'x' }).expect(403);
  await viewer.agent.post('/api/transfers').send({ assetIds: [s.inside.id], toInventoryId: s.techA.id }).expect(403);
  const auditor = await userWithRole(s.agent, 'auditor');
  await auditor.agent.post('/api/assets').send({ name: 'x', type: 'Monitor' }).expect(403);
  await auditor.agent.post('/api/inventories').send({ name: 'x' }).expect(403);
});

test('invitations carry limited access, expire, and can be revoked', async () => {
  const s = await setup();
  const email = `invited${Date.now()}@example.com`;
  const sent = await s.agent.post('/api/invitations')
    .send({ email, role: 'technician', accessScope: 'inventories', inventoryIds: [s.techA.id] }).expect(201);
  assert.equal(sent.body.accessScope, 'inventories');
  await s.agent.post('/api/invitations')
    .send({ email: `bad${email}`, role: 'admin', accessScope: 'inventories', inventoryIds: [s.techA.id] }).expect(400);
  const listed = byName((await s.agent.get('/api/invitations').expect(200)).body.items.map((i) => ({ ...i, name: i.email })), email);
  assert.deepEqual(listed.inventoryIds, [s.techA.id]);

  const token = tokenFrom((await lastEmail(email)).body, 'invite');
  const invited = request.agent(app);
  await invited.post('/api/invitations/accept').send({ token, name: 'Ina', password: PASSWORD }).expect(200);
  const me = (await invited.get('/api/auth/me').expect(200)).body.user;
  assert.equal(me.role, 'technician');
  assert.equal(me.access.scope, 'inventories');
  await invited.get(`/api/assets/${s.outside.id}`).expect(404);
  await invited.get(`/api/assets/${s.inside.id}`).expect(200);
  await request(app).post('/api/invitations/accept').send({ token, name: 'Again', password: PASSWORD }).expect(400);

  // Revoked and expired invitations no longer work.
  const other = `revoked${Date.now()}@example.com`;
  await s.agent.post('/api/invitations').send({ email: other, role: 'viewer' }).expect(201);
  const revokedToken = tokenFrom((await lastEmail(other)).body, 'invite');
  const inv = (await s.agent.get('/api/invitations')).body.items.find((i) => i.email === other);
  await s.agent.delete(`/api/invitations/${inv.id}`).expect(204);
  await request(app).post('/api/invitations/accept').send({ token: revokedToken, name: 'R', password: PASSWORD }).expect(400);

  const late = `late${Date.now()}@example.com`;
  await s.agent.post('/api/invitations').send({ email: late, role: 'viewer' }).expect(201);
  const lateToken = tokenFrom((await lastEmail(late)).body, 'invite');
  await pool.query("UPDATE auth_tokens SET expires_at = now() - interval '1 minute' WHERE email = $1", [late]);
  await request(app).post('/api/invitations/accept').send({ token: lateToken, name: 'L', password: PASSWORD }).expect(400);
});

test('access requests: asked by limited users, decided by someone else with full access, then granted', async () => {
  const s = await setup();
  const tech = await scopedUser(s.agent, 'technician', [s.techA.id]);
  const scopedMgr = await scopedUser(s.agent, 'manager', [s.techA.id]);
  const manager = await userWithRole(s.agent, 'manager');

  const options = (await tech.agent.get('/api/access-requests').expect(200)).body;
  assert.ok(byName(options.requestable, 'Available Inventory'));
  assert.ok(!byName(options.requestable, 'Bin 1'));
  assert.equal(options.canDecide, false);
  await manager.agent.post('/api/access-requests').send({ inventoryId: s.available.id }).expect(409);
  await tech.agent.post('/api/access-requests').send({ inventoryId: s.techBin.id }).expect(409);
  const req = (await tech.agent.post('/api/access-requests')
    .send({ inventoryId: s.available.id, note: 'Covering for Sam' }).expect(201)).body;
  assert.equal(req.status, 'pending');
  await tech.agent.post('/api/access-requests').send({ inventoryId: s.available.id }).expect(409);

  // The manager with full access is told; a limited manager cannot decide.
  await new Promise((r) => setTimeout(r, 200));
  const notes = (await manager.agent.get('/api/notifications').expect(200)).body.items;
  assert.ok(notes.some((n) => n.kind === 'access_requested'));
  await scopedMgr.agent.post(`/api/access-requests/${req.id}/approve`).send({}).expect(403);
  await tech.agent.post(`/api/access-requests/${req.id}/approve`).send({}).expect(403);
  assert.equal((await manager.agent.get('/api/access-requests?status=pending').expect(200)).body.items.length, 1);

  await tech.agent.get(`/api/assets/${s.outside.id}`).expect(404);
  const done = (await manager.agent.post(`/api/access-requests/${req.id}/approve`).send({ note: 'ok' }).expect(200)).body;
  assert.equal(done.status, 'approved');
  await manager.agent.post(`/api/access-requests/${req.id}/reject`).send({}).expect(409);
  await tech.agent.get(`/api/assets/${s.outside.id}`).expect(200);
  await new Promise((r) => setTimeout(r, 200));
  assert.ok((await tech.agent.get('/api/notifications')).body.items.some((n) => n.kind === 'access_approved'));

  // A request can be withdrawn by its author only.
  const second = (await tech.agent.post('/api/access-requests').send({ inventoryId: s.hub.id }).expect(201)).body;
  await scopedMgr.agent.post(`/api/access-requests/${second.id}/cancel`).expect(404);
  assert.equal((await tech.agent.post(`/api/access-requests/${second.id}/cancel`).expect(200)).body.status, 'cancelled');
});

test('workflow 13: nobody reaches another workspace, whatever ids they send', async () => {
  const a = await setup();
  const b = await setup();
  await a.agent.get(`/api/assets/${b.inside.id}`).expect(404);
  await a.agent.get(`/api/inventories/${b.techA.id}`).expect(404);
  await a.agent.post('/api/transfers').send({ assetIds: [a.inside.id], toInventoryId: b.techA.id }).expect(400);
  await a.agent.post('/api/transfers').send({ assetIds: [b.inside.id], toInventoryId: a.techA.id }).expect(404);
  const userB = await userWithRole(b.agent, 'technician');
  await a.agent.patch(`/api/users/${userB.id}`).send({ role: 'viewer' }).expect(404);
  const techA = await userWithRole(a.agent, 'technician');
  await a.agent.patch(`/api/users/${techA.id}`).send({ accessScope: 'inventories', inventoryIds: [b.techA.id] }).expect(400);
  await a.agent.post('/api/invitations')
    .send({ email: `x${Date.now()}@example.com`, role: 'viewer', accessScope: 'inventories', inventoryIds: [b.techA.id] }).expect(400);
  const scoped = await scopedUser(a.agent, 'technician', [a.techA.id]);
  await scoped.agent.post('/api/access-requests').send({ inventoryId: b.available.id }).expect(400);
  const reqB = (await (await scopedUser(b.agent, 'technician', [b.techA.id])).agent
    .post('/api/access-requests').send({ inventoryId: b.available.id }).expect(201)).body;
  await a.agent.post(`/api/access-requests/${reqB.id}/approve`).send({}).expect(404);
});
