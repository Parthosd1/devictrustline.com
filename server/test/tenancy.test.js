import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

test('organizations never see or touch each other\'s data', async () => {
  const a = await newOrg('Org A');
  const b = await newOrg('Org B');
  const { body: building } = await a.agent.post('/api/buildings').send({ name: 'HQ' }).expect(201);
  const { body: location } = await a.agent.post(`/api/buildings/${building.id}/locations`).send({ name: 'IT Room' }).expect(201);
  const { body: asset } = await a.agent.post('/api/assets')
    .send({ name: 'Secret Laptop', type: 'Laptop', serial: 'S-1', buildingId: building.id, locationId: location.id })
    .expect(201);

  const list = await b.agent.get('/api/assets').expect(200);
  assert.equal(list.body.total, 0);
  assert.equal((await b.agent.get('/api/buildings').expect(200)).body.items.length, 0);
  await b.agent.get(`/api/assets/${asset.id}`).expect(404);
  await b.agent.get(`/api/assets/${asset.id}/events`).expect(404);
  await b.agent.patch(`/api/assets/${asset.id}`).send({ status: 'Retired' }).expect(404);
  await b.agent.patch(`/api/buildings/${building.id}`).send({ name: 'Mine' }).expect(404);
  await b.agent.post(`/api/buildings/${building.id}/locations`).send({ name: 'X' }).expect(404);
  await b.agent.delete(`/api/locations/${location.id}`).expect(404);
  await b.agent.patch(`/api/users/${a.user.id}`).send({ role: 'viewer' }).expect(404);
  assert.ok(!(await b.agent.get('/api/users').expect(200)).body.items.some((u) => u.id === a.user.id));

  // Org B cannot place its own asset in Org A's building either.
  await b.agent.post('/api/assets').send({ name: 'Sneaky', type: 'Laptop', buildingId: building.id }).expect(400);

  // Serial numbers and asset tags are unique per organization, not globally.
  const own = await b.agent.post('/api/assets').send({ name: 'Laptop', type: 'Laptop', serial: 'S-1' }).expect(201);
  assert.equal(own.body.assetTag, 'DT-1001');
});
