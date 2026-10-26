import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

after(() => pool.end());

let admin;
let tech;
let viewer;
let scanner;
let laptop;
let person;
before(async () => {
  await resetDatabase();
  admin = await newOrg();
  tech = await userWithRole(admin.agent, 'technician');
  viewer = await userWithRole(admin.agent, 'viewer');
  scanner = (await admin.agent.post('/api/assets').send({ name: 'Zebra DS3678', type: 'Scanner' }).expect(201)).body;
  laptop = (await admin.agent.post('/api/assets').send({ name: 'ThinkPad', type: 'Laptop' }).expect(201)).body;
  person = (await admin.agent.post('/api/people').send({ name: 'Priya' }).expect(201)).body;
  await admin.agent.post(`/api/assets/${laptop.id}/assign`).send({ personId: person.id }).expect(200);
});

const get = (agent, path) => agent.get(path).expect(200).then((r) => r.body);

test('opening a work order numbers it, puts the asset in Maintenance and logs history', async () => {
  const wo = (await tech.agent.post('/api/work-orders').send({
    assetId: scanner.id, title: 'Trigger sticks', priority: 'high', assigneeId: tech.id, dueDate: '2020-01-01',
  }).expect(201)).body;
  assert.equal(wo.number, 'WO-1001');
  assert.equal(wo.assigneeName, 'technician');
  assert.equal(wo.overdue, true);
  assert.equal((await get(admin.agent, `/api/assets/${scanner.id}`)).status, 'Maintenance');
  const types = (await get(admin.agent, `/api/assets/${scanner.id}/events`)).items.map((e) => e.type);
  assert.deepEqual(types.slice(0, 2), ['status_changed', 'maintenance_opened']);
});

test('updates, notes and resolution', async () => {
  const [wo] = (await get(admin.agent, '/api/work-orders?status=active')).items;
  await tech.agent.patch(`/api/work-orders/${wo.id}`).send({ status: 'waiting_parts', vendor: 'Zebra', cost: '42.50' }).expect(200);
  await tech.agent.post(`/api/work-orders/${wo.id}/notes`).send({ body: 'Ordered a new trigger' }).expect(201);
  const resolved = (await tech.agent.post(`/api/work-orders/${wo.id}/resolve`).send({ resolution: 'Replaced trigger' }).expect(200)).body;
  assert.equal(resolved.status, 'resolved');
  assert.equal(resolved.cost, 42.5);
  assert.ok(resolved.closedAt);
  assert.equal((await get(admin.agent, `/api/assets/${scanner.id}`)).status, 'Available', 'unassigned assets return to Available');
  const detail = await get(admin.agent, `/api/work-orders/${wo.id}`);
  assert.deepEqual(detail.notes.map((n) => n.body), [
    'Opened', 'Status: Waiting on parts', 'Ordered a new trigger', 'Resolved: Replaced trigger',
  ]);
  await tech.agent.patch(`/api/work-orders/${wo.id}`).send({ title: 'x' }).expect(409);
  await tech.agent.post(`/api/work-orders/${wo.id}/resolve`).send({ resolution: 'again' }).expect(409);
  await assert.rejects(pool.query('DELETE FROM work_order_notes'), /append-only/);
});

test('an assigned asset goes back to Deployed, and only when its last work order closes', async () => {
  const a = (await admin.agent.post('/api/work-orders').send({ assetId: laptop.id, title: 'Battery' }).expect(201)).body;
  const b = (await admin.agent.post('/api/work-orders').send({ assetId: laptop.id, title: 'Keyboard' }).expect(201)).body;
  assert.equal(b.number, 'WO-1003');
  await admin.agent.post(`/api/work-orders/${a.id}/cancel`).send({ reason: 'Duplicate' }).expect(200);
  assert.equal((await get(admin.agent, `/api/assets/${laptop.id}`)).status, 'Maintenance');
  await admin.agent.post(`/api/work-orders/${b.id}/resolve`).send({ resolution: 'Swapped keyboard' }).expect(200);
  assert.equal((await get(admin.agent, `/api/assets/${laptop.id}`)).status, 'Deployed');
  assert.equal((await get(admin.agent, `/api/work-orders?assetId=${laptop.id}`)).items.length, 2);
  assert.equal((await get(admin.agent, '/api/work-orders?status=closed')).items.length, 3);
});

test('rules: permissions, assignees, retired assets, isolation', async () => {
  await viewer.agent.get('/api/work-orders').expect(200);
  await viewer.agent.post('/api/work-orders').send({ assetId: scanner.id, title: 'x' }).expect(403);
  await admin.agent.post('/api/work-orders').send({ assetId: scanner.id, title: 'x', assigneeId: viewer.id }).expect(400);
  const names = (await get(tech.agent, '/api/work-orders/assignees')).items.map((u) => u.role);
  assert.ok(!names.includes('viewer'));
  const retired = (await admin.agent.post('/api/assets').send({ name: 'Old', type: 'Other', status: 'Retired' }).expect(201)).body;
  await admin.agent.post('/api/work-orders').send({ assetId: retired.id, title: 'x' }).expect(409);

  const other = await newOrg();
  await other.agent.post('/api/work-orders').send({ assetId: scanner.id, title: 'x' }).expect(400);
  const [mine] = (await get(admin.agent, '/api/work-orders')).items;
  await other.agent.get(`/api/work-orders/${mine.id}`).expect(404);
  await other.agent.post(`/api/work-orders/${mine.id}/notes`).send({ body: 'x' }).expect(404);
  assert.equal((await get(other.agent, '/api/work-orders')).items.length, 0);
});
