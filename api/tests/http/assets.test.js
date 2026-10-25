import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase } from './helpers.js';

after(() => pool.end());

let agent;
let hq;
let annex;
before(async () => {
  await resetDatabase();
  ({ agent } = await newOrg());
  hq = (await agent.post('/api/buildings').send({ name: 'Building A' }).expect(201)).body;
  annex = (await agent.post('/api/buildings').send({ name: 'Building B' }).expect(201)).body;
  hq.station = (await agent.post(`/api/buildings/${hq.id}/locations`).send({ name: 'Station 214-02' }).expect(201)).body;
  annex.storage = (await agent.post(`/api/buildings/${annex.id}/locations`).send({ name: 'Loaner Storage' }).expect(201)).body;
});

test('creating assets assigns sequential tags and records history', async () => {
  const first = (await agent.post('/api/assets').send({
    name: 'Zebra DS3678', type: 'Scanner', serial: 'ZB-1', status: 'Deployed',
    buildingId: hq.id, locationId: hq.station.id, purchaseDate: '2025-02-01',
  }).expect(201)).body;
  const second = (await agent.post('/api/assets').send({ name: 'Dell P2422H', type: 'Monitor', serial: 'DP-1' }).expect(201)).body;
  assert.equal(first.assetTag, 'DT-1001');
  assert.equal(second.assetTag, 'DT-1002');
  assert.equal(first.buildingName, 'Building A');
  assert.equal(first.locationName, 'Station 214-02');
  assert.equal(first.purchaseDate, '2025-02-01');
  assert.equal(second.status, 'Available');
  const events = (await agent.get(`/api/assets/${first.id}/events`).expect(200)).body.items;
  assert.equal(events.length, 1);
  assert.equal(events[0].type, 'created');
  assert.equal(events[0].actor.name, 'Admin');
});

test('duplicate serials are rejected regardless of case', async () => {
  await agent.post('/api/assets').send({ name: 'Copy', type: 'Scanner', serial: 'zb-1' }).expect(409);
});

test('invalid input is rejected with details', async () => {
  const res = await agent.post('/api/assets').send({ name: '', type: 'Toaster' }).expect(400);
  assert.ok(res.body.details.length >= 2);
  await agent.post('/api/assets').send({ name: 'X', type: 'Laptop', unknownField: 1 }).expect(400);
  await agent.post('/api/assets').send({ name: 'X', type: 'Laptop', locationId: hq.station.id }).expect(400);
  await agent.post('/api/assets')
    .send({ name: 'X', type: 'Laptop', buildingId: annex.id, locationId: hq.station.id }).expect(400);
  await agent.get('/api/assets/not-a-uuid').expect(400);
});

test('updates record exactly what changed', async () => {
  const a = (await agent.post('/api/assets').send({
    name: 'HP EliteDesk 800', type: 'Desktop', serial: 'HP-1', buildingId: hq.id, locationId: hq.station.id,
  }).expect(201)).body;
  const moved = (await agent.patch(`/api/assets/${a.id}`).send({ buildingId: annex.id, status: 'Deployed' }).expect(200)).body;
  assert.equal(moved.buildingName, 'Building B');
  assert.equal(moved.locationId, null, 'moving buildings clears the old location');
  await agent.patch(`/api/assets/${a.id}`).send({ name: 'HP EliteDesk 800' }).expect(200);
  const events = (await agent.get(`/api/assets/${a.id}/events`).expect(200)).body.items;
  assert.equal(events.length, 2, 'a no-op update writes no event');
  assert.equal(events[0].type, 'status_changed');
  assert.deepEqual(events[0].changes.status, { from: 'Available', to: 'Deployed' });
  assert.deepEqual(events[0].changes.locationId, { from: hq.station.id, to: null });
});

test('search and filters', async () => {
  const all = (await agent.get('/api/assets').expect(200)).body;
  assert.equal(all.total, 3);
  assert.equal((await agent.get('/api/assets?q=zebra').expect(200)).body.total, 1);
  assert.equal((await agent.get('/api/assets?q=station%20214').expect(200)).body.total, 1);
  assert.equal((await agent.get('/api/assets?q=100_').expect(200)).body.total, 0, 'LIKE wildcards are escaped');
  assert.equal((await agent.get('/api/assets?status=Deployed').expect(200)).body.total, 2);
  assert.equal((await agent.get(`/api/assets?buildingId=${annex.id}`).expect(200)).body.total, 1);
  const page = (await agent.get('/api/assets?limit=2&offset=2').expect(200)).body;
  assert.equal(page.items.length, 1);
  await agent.get('/api/assets?status=Lost').expect(400);
});

test('asset history cannot be edited or deleted, even directly in the database', async () => {
  await assert.rejects(pool.query("UPDATE asset_events SET event_type = 'tampered'"), /append-only/);
  await assert.rejects(pool.query('DELETE FROM asset_events'), /append-only/);
});

test('buildings list counts and refuses to delete occupied sites', async () => {
  const items = (await agent.get('/api/buildings').expect(200)).body.items;
  const a = items.find((b) => b.id === hq.id);
  assert.equal(a.assetCount, 1);
  assert.equal(a.locations[0].name, 'Station 214-02');
  await agent.delete(`/api/buildings/${hq.id}`).expect(409);
  await agent.delete(`/api/locations/${hq.station.id}`).expect(409);
  await agent.delete(`/api/locations/${annex.storage.id}`).expect(204);
  const empty = (await agent.post('/api/buildings').send({ name: 'Temp' }).expect(201)).body;
  await agent.delete(`/api/buildings/${empty.id}`).expect(204);
  await agent.post('/api/buildings').send({ name: 'Building A' }).expect(409);
});
