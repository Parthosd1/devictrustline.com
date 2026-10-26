import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

after(() => pool.end());

let agent;
let hq;
let annex;
let person;
const create = async (body) => (await agent.post('/api/assets').send({ type: 'Accessory', ...body }).expect(201)).body;
const events = async (id) => (await agent.get(`/api/assets/${id}/events`).expect(200)).body.items;

before(async () => {
  await resetDatabase();
  ({ agent } = await newOrg());
  hq = (await agent.post('/api/buildings').send({ name: 'HQ' }).expect(201)).body;
  hq.desk = (await agent.post(`/api/buildings/${hq.id}/locations`).send({ name: 'Desk 214-02' }).expect(201)).body;
  annex = (await agent.post('/api/buildings').send({ name: 'Annex' }).expect(201)).body;
  annex.desk = (await agent.post(`/api/buildings/${annex.id}/locations`).send({ name: 'Desk 4151' }).expect(201)).body;
  person = (await agent.post('/api/people').send({ name: 'Priya Shah', email: 'Priya@Example.com', department: 'Receiving' }).expect(201)).body;
});

test('people can be created, listed and validated', async () => {
  assert.equal(person.email, 'priya@example.com');
  assert.equal(person.assetCount, 0);
  await agent.post('/api/people').send({ name: 'Dup', email: 'priya@example.com' }).expect(409);
  await agent.post('/api/people').send({ name: '' }).expect(400);
  const list = (await agent.get('/api/people').expect(200)).body.items;
  assert.equal(list.length, 1);
});

test('components inherit their parent\'s placement and follow it when it moves', async () => {
  const station = await create({ name: 'Workstation 214-02', type: 'Workstation', buildingId: hq.id, locationId: hq.desk.id });
  const monitor = await create({ name: 'Dell P2422H', type: 'Monitor', parentId: station.id });
  assert.equal(monitor.locationId, hq.desk.id, 'inherits placement');
  assert.equal(monitor.parentTag, station.assetTag);
  const scanner = await create({ name: 'Zebra DS3678', type: 'Scanner' });
  await agent.patch(`/api/assets/${scanner.id}`).send({ parentId: station.id }).expect(200);

  const detail = (await agent.get(`/api/assets/${station.id}`).expect(200)).body;
  assert.equal(detail.childCount, 2);
  assert.deepEqual(detail.children.map((c) => c.name).sort(), ['Dell P2422H', 'Zebra DS3678']);

  await agent.patch(`/api/assets/${station.id}`).send({ buildingId: annex.id, locationId: annex.desk.id }).expect(200);
  const moved = (await agent.get(`/api/assets?parentId=${station.id}`).expect(200)).body.items;
  assert.ok(moved.every((c) => c.locationId === annex.desk.id && c.buildingId === annex.id));
  const [ev] = await events(monitor.id);
  assert.equal(ev.type, 'moved_with_parent');
  assert.equal(ev.changes.via.to, station.id);
});

test('cycles are refused', async () => {
  const a = await create({ name: 'A' });
  const b = await create({ name: 'B', parentId: a.id });
  const c = await create({ name: 'C', parentId: b.id });
  await agent.patch(`/api/assets/${a.id}`).send({ parentId: c.id }).expect(400);
  await agent.patch(`/api/assets/${a.id}`).send({ parentId: a.id }).expect(400);
  await agent.patch(`/api/assets/${c.id}`).send({ parentId: null }).expect(200);
});

test('assigning a parent assigns its components; returning clears them', async () => {
  const laptop = await create({ name: 'ThinkPad T14', type: 'Laptop' });
  const dock = await create({ name: 'USB-C Dock', type: 'Docking Station', parentId: laptop.id });
  const assigned = (await agent.post(`/api/assets/${laptop.id}/assign`).send({ personId: person.id, note: 'New hire' }).expect(200)).body;
  assert.equal(assigned.assignedPersonName, 'Priya Shah');
  assert.equal(assigned.status, 'Deployed');
  const dockNow = (await agent.get(`/api/assets/${dock.id}`).expect(200)).body;
  assert.equal(dockNow.assignedPersonId, person.id);

  const [ev] = await events(laptop.id);
  assert.equal(ev.type, 'assigned');
  assert.deepEqual(ev.changes.assignedTo.to, { id: person.id, name: 'Priya Shah' });
  assert.equal(ev.changes.note.to, 'New hire');

  assert.equal((await agent.get(`/api/assets?personId=${person.id}`).expect(200)).body.total, 2);
  assert.equal((await agent.get('/api/assets?q=priya').expect(200)).body.total, 2);
  assert.equal((await agent.get('/api/people').expect(200)).body.items[0].assetCount, 2);

  const returned = (await agent.post(`/api/assets/${laptop.id}/return`).send({ status: 'Maintenance' }).expect(200)).body;
  assert.equal(returned.assignedPersonId, null);
  assert.equal(returned.status, 'Maintenance');
  assert.equal((await agent.get(`/api/assets/${dock.id}`).expect(200)).body.assignedPersonId, null);
  assert.equal((await events(dock.id))[0].type, 'returned');
});

test('assignment rules', async () => {
  const retired = await create({ name: 'Old', status: 'Retired' });
  await agent.post(`/api/assets/${retired.id}/assign`).send({ personId: person.id }).expect(409);
  const leaver = (await agent.post('/api/people').send({ name: 'Leaver' }).expect(201)).body;
  await agent.patch(`/api/people/${leaver.id}`).send({ isActive: false }).expect(200);
  const a = await create({ name: 'Phone', type: 'Phone' });
  await agent.post(`/api/assets/${a.id}/assign`).send({ personId: leaver.id }).expect(409);

  const viewer = await userWithRole(agent, 'viewer');
  await viewer.agent.post(`/api/assets/${a.id}/assign`).send({ personId: person.id }).expect(403);
  await viewer.agent.post('/api/people').send({ name: 'X' }).expect(403);

  const other = await newOrg();
  const theirs = (await other.agent.post('/api/assets').send({ name: 'Theirs', type: 'Laptop' }).expect(201)).body;
  await other.agent.post(`/api/assets/${theirs.id}/assign`).send({ personId: person.id }).expect(400);
  await other.agent.patch(`/api/assets/${theirs.id}`).send({ parentId: a.id }).expect(400);
  await other.agent.patch(`/api/people/${person.id}`).send({ name: 'Hijack' }).expect(404);
});
