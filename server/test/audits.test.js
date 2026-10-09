import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

after(() => pool.end());

let agent;
let hq;
let annex;
const assets = {};

before(async () => {
  await resetDatabase();
  ({ agent } = await newOrg());
  hq = (await agent.post('/api/buildings').send({ name: 'HQ' }).expect(201)).body;
  hq.a = (await agent.post(`/api/buildings/${hq.id}/locations`).send({ name: 'Desk A' }).expect(201)).body;
  hq.b = (await agent.post(`/api/buildings/${hq.id}/locations`).send({ name: 'Desk B' }).expect(201)).body;
  annex = (await agent.post('/api/buildings').send({ name: 'Annex' }).expect(201)).body;
  const add = async (key, body) => {
    assets[key] = (await agent.post('/api/assets').send({ type: 'Scanner', ...body }).expect(201)).body;
  };
  await add('s1', { name: 'Scanner 1', serial: 'S1', buildingId: hq.id, locationId: hq.a.id });
  await add('s2', { name: 'Scanner 2', serial: 'S2', buildingId: hq.id, locationId: hq.a.id });
  await add('s3', { name: 'Scanner 3', serial: 'S3', buildingId: hq.id, locationId: hq.b.id });
  await add('old', { name: 'Retired scanner', buildingId: hq.id, status: 'Retired' });
  await add('far', { name: 'Annex laptop', type: 'Laptop', buildingId: annex.id });
});

test('weekly and monthly audits run independently with their own snapshot', async () => {
  const weekly = (await agent.post('/api/audits').send({ period: 'weekly', buildingId: hq.id }).expect(201)).body;
  const monthly = (await agent.post('/api/audits').send({ period: 'monthly' }).expect(201)).body;
  assert.equal(weekly.counts.expected, 3, 'retired and out-of-scope assets are not expected');
  assert.equal(monthly.counts.expected, 4);
  assert.match(weekly.name, /^Weekly audit · .* · HQ$/);
  const days = (new Date(weekly.dueAt) - new Date(weekly.startedAt)) / 86400000;
  assert.ok(days > 6.9 && days < 7.1);
  await agent.post('/api/audits').send({ period: 'weekly', buildingId: hq.id }).expect(409);
  // A different scope may run alongside.
  await agent.post('/api/audits').send({ period: 'weekly', buildingId: annex.id }).expect(201);

  await agent.post(`/api/audits/${weekly.id}/scan`).send({ code: assets.s1.assetTag }).expect(200);
  const w = (await agent.get(`/api/audits/${weekly.id}`).expect(200)).body;
  const m = (await agent.get(`/api/audits/${monthly.id}`).expect(200)).body;
  assert.equal(w.counts.verified, 1);
  assert.equal(m.counts.verified, 0, 'a scan in one audit does not count in the other');
});

test('scans record verified, misplaced and unexpected assets; closing marks the rest missing', async () => {
  const run = (await agent.post('/api/audits').send({ period: 'weekly', buildingId: hq.id, locationId: hq.a.id }).expect(201)).body;
  assert.equal(run.counts.expected, 2);
  const ok = (await agent.post(`/api/audits/${run.id}/scan`).send({ code: 'https://devicetrustline.com/?asset=' + assets.s1.assetTag }).expect(200)).body;
  assert.equal(ok.result, 'verified');
  assert.equal(ok.method, 'scan');
  const extra = (await agent.post(`/api/audits/${run.id}/scan`).send({ code: 'S3' }).expect(200)).body;
  assert.equal(extra.expected, false, 'S3 belongs at Desk B');
  await agent.post(`/api/audits/${run.id}/scan`).send({ code: 'NOPE' }).expect(404);

  const closed = (await agent.post(`/api/audits/${run.id}/close`).expect(200)).body;
  assert.equal(closed.status, 'closed');
  assert.deepEqual(closed.summary, { expected: 2, verified: 1, wrongLocation: 0, missing: 1, pending: 0, unexpected: 1 });
  const detail = (await agent.get(`/api/audits/${run.id}`).expect(200)).body;
  assert.equal(detail.items.find((i) => i.assetId === assets.s2.id).result, 'missing');

  const history = (await agent.get(`/api/assets/${assets.s2.id}/events`).expect(200)).body.items;
  assert.equal(history[0].type, 'audited');
  assert.equal(history[0].changes.audit.to.result, 'missing');
});

test('a scan at the wrong location is flagged', async () => {
  const run = (await agent.post('/api/audits').send({ period: 'monthly', buildingId: hq.id }).expect(201)).body;
  const item = (await agent.post(`/api/audits/${run.id}/scan`).send({ code: 'S3', locationId: hq.a.id }).expect(200)).body;
  assert.equal(item.result, 'wrong_location');
  assert.equal(item.expectedLocationName, 'Desk B');
  assert.equal(item.foundLocationName, 'Desk A');
  const fixed = (await agent.patch(`/api/audits/${run.id}/items/${assets.s3.id}`).send({ result: 'verified', note: 'Moved back' }).expect(200)).body;
  assert.equal(fixed.result, 'verified');
  assert.equal(fixed.method, 'manual');
  assert.equal(fixed.note, 'Moved back');
});

test('closed audits are final, even in the database', async () => {
  const { items } = (await agent.get('/api/audits?status=closed').expect(200)).body;
  const run = items[0];
  await agent.post(`/api/audits/${run.id}/scan`).send({ code: 'S1' }).expect(409);
  await agent.post(`/api/audits/${run.id}/close`).expect(409);
  await agent.patch(`/api/audits/${run.id}/items/${assets.s2.id}`).send({ result: 'verified' }).expect(409);
  await assert.rejects(pool.query("UPDATE audit_items SET result = 'verified' WHERE run_id = $1", [run.id]), /closed audits/);
  await assert.rejects(pool.query("UPDATE audit_runs SET name = 'x' WHERE id = $1", [run.id]), /closed audits/);
  await assert.rejects(pool.query('DELETE FROM audit_runs WHERE id = $1', [run.id]), /closed audits/);
});

test('audit permissions and isolation', async () => {
  const auditor = await userWithRole(agent, 'auditor');
  const tech = await userWithRole(agent, 'technician');
  const run = (await auditor.agent.post('/api/audits').send({ period: 'weekly', buildingId: annex.id }).expect(409)).body;
  assert.ok(run.error);
  await tech.agent.post('/api/audits').send({ period: 'monthly', buildingId: annex.id }).expect(403);
  await tech.agent.get('/api/audits').expect(200);
  const open = (await auditor.agent.get('/api/audits?status=open&period=weekly').expect(200)).body.items[0];
  await auditor.agent.post(`/api/audits/${open.id}/scan`).send({ code: 'S1' }).expect(200);
  await tech.agent.post(`/api/audits/${open.id}/scan`).send({ code: 'S1' }).expect(403);

  const other = await newOrg();
  await other.agent.get(`/api/audits/${open.id}`).expect(404);
  await other.agent.post(`/api/audits/${open.id}/scan`).send({ code: 'S1' }).expect(404);
  await other.agent.post('/api/audits').send({ period: 'weekly', buildingId: hq.id }).expect(400);
  assert.equal((await other.agent.get('/api/audits').expect(200)).body.items.length, 0);
});
