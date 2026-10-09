import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase, userWithRole } from './helpers.js';
// After helpers, which points the database pool at the test database.
import { runScheduledAudits } from '../src/scheduler.js';

after(() => pool.end());

let admin;
before(async () => {
  await resetDatabase();
  admin = await newOrg();
  await admin.agent.post('/api/buildings').send({ name: 'Building A' }).expect(201);
});

const rows = [
  { name: 'Dell Latitude 5440', type: 'laptop', serial: 'IMP-1', building: 'building a', status: 'deployed', warrantyExpires: '2020-01-01' },
  { name: 'Zebra DS3678', type: 'Scanner', serial: 'IMP-2', building: 'Building B', location: 'Station 4151' },
];

test('CSV import validates everything before writing anything', async () => {
  const dry = (await admin.agent.post('/api/assets/import').send({ rows }).expect(200)).body;
  assert.equal(dry.valid, 1);
  assert.match(dry.errors[0].messages.join(), /building: "Building B" does not exist/);
  assert.equal(dry.errors[0].row, 2);
  await admin.agent.post('/api/assets/import').send({ rows, dryRun: false }).expect(400);
  assert.equal((await admin.agent.get('/api/assets').expect(200)).body.total, 0, 'nothing written');

  const bad = (await admin.agent.post('/api/assets/import').send({
    rows: [{ name: '', type: 'Toaster' }, { name: 'X', type: 'Laptop', serial: 'D' }, { name: 'Y', type: 'Laptop', serial: 'd' }],
  }).expect(200)).body;
  assert.deepEqual(bad.errors.map((e) => e.row), [1, 3]);
  assert.match(bad.errors[0].messages.join(), /"Toaster" is not one of/);
  assert.match(bad.errors[1].messages.join(), /serial: d already exists/);
});

test('CSV import can create buildings and locations and records history', async () => {
  const res = (await admin.agent.post('/api/assets/import').send({ rows, createSites: true, dryRun: false }).expect(201)).body;
  assert.equal(res.created, 2);
  assert.deepEqual(res.newBuildings, ['Building B']);
  assert.deepEqual(res.newLocations, ['Building B / Station 4151']);
  const list = (await admin.agent.get('/api/assets').expect(200)).body.items;
  const zebra = list.find((a) => a.serial === 'IMP-2');
  assert.equal(zebra.locationName, 'Station 4151');
  assert.equal(list.find((a) => a.serial === 'IMP-1').status, 'Deployed');
  assert.deepEqual(list.map((a) => a.assetTag).sort(), ['DT-1001', 'DT-1002']);
  const [ev] = (await admin.agent.get(`/api/assets/${zebra.id}/events`).expect(200)).body.items;
  assert.equal(ev.changes.source.to, 'csv import');
  const viewer = await userWithRole(admin.agent, 'viewer');
  await viewer.agent.post('/api/assets/import').send({ rows }).expect(403);
});

test('reports summarize inventory, warranties, maintenance and audits', async () => {
  const [laptop] = (await admin.agent.get('/api/assets?q=IMP-1').expect(200)).body.items;
  const wo = (await admin.agent.post('/api/work-orders').send({ assetId: laptop.id, title: 'Battery' }).expect(201)).body;
  await admin.agent.post(`/api/work-orders/${wo.id}/resolve`).send({ resolution: 'Replaced', cost: 80 }).expect(200);
  const run = (await admin.agent.post('/api/audits').send({ period: 'monthly' }).expect(201)).body;
  await admin.agent.post(`/api/audits/${run.id}/close`).expect(200);

  const r = (await admin.agent.get('/api/reports/summary').expect(200)).body;
  assert.equal(r.byType.find((t) => t.key === 'Laptop').count, 1);
  assert.equal(r.byBuilding.length, 2);
  assert.equal(r.warranty[0].assetTag, laptop.assetTag);
  assert.equal(r.warranty[0].expired, true);
  assert.equal(r.maintenance.resolved30d, 1);
  assert.equal(r.maintenanceCostByMonth.length, 12);
  assert.equal(r.maintenanceCostByMonth.at(-1).cost, 80);
  assert.equal(r.recentAudits[0].summary.missing, 2);
  assert.equal(r.stillMissing.length, 2);
});

test('automatic audits open once per week and month for organizations that turn them on', async () => {
  await admin.agent.patch('/api/settings').send({ autoAudits: { weekly: true } }).expect(200);
  assert.deepEqual((await admin.agent.get('/api/settings').expect(200)).body.autoAudits, { weekly: true, monthly: false });
  const tech = await userWithRole(admin.agent, 'technician');
  await tech.agent.patch('/api/settings').send({ autoAudits: { monthly: true } }).expect(403);

  const other = await newOrg();
  const first = await runScheduledAudits({ log: () => {} });
  assert.equal(first.length, 1);
  assert.equal(first[0].period, 'weekly');
  assert.equal((await runScheduledAudits({ log: () => {} })).length, 0, 'not twice');

  const open = (await admin.agent.get('/api/audits?period=weekly&status=open').expect(200)).body.items;
  assert.equal(open.length, 1);
  assert.equal(open[0].scheduled, true);
  assert.equal(open[0].counts.expected, 2);
  // Closing it does not cause another this week.
  await admin.agent.post(`/api/audits/${open[0].id}/close`).expect(200);
  assert.equal((await runScheduledAudits({ log: () => {} })).length, 0);
  assert.equal((await other.agent.get('/api/audits').expect(200)).body.items.length, 0);
});
