import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import request from 'supertest';
import { PASSWORD, app, newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

let n = 0;
async function personal() {
  const agent = request.agent(app);
  const email = `personal${++n}-${Date.now()}@example.com`;
  const res = await agent.post('/api/auth/signup').send({ accountType: 'personal', name: 'Pat', email, password: PASSWORD }).expect(201);
  return { agent, user: res.body.user, email };
}

test('plans: limits are enforced on the server and the operator can change them', async () => {
  const { agent, user } = await personal();
  const plan = (await agent.get('/api/plan')).body;
  assert.equal(plan.plan.code, 'personal_basic');
  assert.equal(plan.billing.configured, false);
  assert.ok(plan.plans.every((p) => p.code.startsWith('personal')));

  await pool.query(`UPDATE plan_definitions SET limits = limits || '{"items": 2, "members": 1}' WHERE code = 'personal_basic'`);
  try {
    await agent.post('/api/assets').send({ name: 'One', type: 'Other' }).expect(201);
    await agent.post('/api/assets').send({ name: 'Two', type: 'Other' }).expect(201);
    const over = await agent.post('/api/assets').send({ name: 'Three', type: 'Other' }).expect(402);
    assert.equal(over.body.code, 'plan_limit');
    assert.match(over.body.error, /Personal Basic plan includes up to 2 items/);
    const csv = await agent.post('/api/assets/import').send({ rows: [{ name: 'X', type: 'Other' }], dryRun: false });
    assert.equal(csv.status, 402);
    await agent.post('/api/users').send({ name: 'B', email: `b${Date.now()}@example.com`, role: 'viewer', password: PASSWORD }).expect(402);
    // Retired items do not count.
    const items = (await agent.get('/api/assets')).body.items;
    await agent.patch(`/api/assets/${items[0].id}`).send({ status: 'Retired' }).expect(200);
    await agent.post('/api/assets').send({ name: 'Three', type: 'Other' }).expect(201);
    assert.equal((await agent.get('/api/plan')).body.usage.items, 2);
  } finally {
    await pool.query(`UPDATE plan_definitions SET limits = limits || '{"items": 100, "members": 2}' WHERE code = 'personal_basic'`);
  }
  assert.equal(user.organization.kind, 'personal');
});

test('checkout explains that payment is not set up instead of pretending', async () => {
  const { agent } = await personal();
  const res = await agent.post('/api/billing/checkout').send({ plan: 'personal_plus' }).expect(503);
  assert.equal(res.body.code, 'billing_unconfigured');
});

test('insights explain each suggestion', async () => {
  const { agent } = await newOrg();
  for (const name of ['Spare 1', 'Spare 2', 'Spare 3']) await agent.post('/api/assets').send({ name, type: 'Monitor' }).expect(201);
  const surplus = (await agent.get('/api/insights')).body.items.find((i) => i.kind === 'surplus');
  assert.equal(surplus.items[0].type, 'Monitor');
  assert.match(surplus.explanation, /nothing is listed for you/);
});

async function pairedAgent(admin, name = 'FRONTDESK-PC') {
  const { code } = (await admin.post('/api/agents/pairing-codes').expect(201)).body;
  const res = await request(app).post('/api/agent/pair').send({ code: code.toLowerCase(), name, platform: 'windows', version: '1.0.0' }).expect(201);
  return { ...res.body, code, auth: { Authorization: `Bearer ${res.body.token}` } };
}

const scan = (devices, complete = true) => ({ version: '1.0.0', complete, devices });
const dellMonitor = { localId: 'DISPLAY\\DEL4321\\1', class: 'monitor', name: 'Dell P2422H', manufacturer: 'Dell', model: 'P2422H', serial: 'CN0ABC123' };
const usbMouse = { localId: 'HID\\VID_046D&PID_C077\\7&1', class: 'peripheral', name: 'USB Optical Mouse', manufacturer: 'Logitech', model: 'M100' };

test('pairing codes work once and pair a computer to one workspace', async () => {
  const { agent: admin } = await newOrg('Pairing Co');
  const paired = await pairedAgent(admin);
  assert.equal(paired.workspace, 'Pairing Co');
  assert.ok(paired.token.startsWith('dta_'));
  await request(app).post('/api/agent/pair').send({ code: paired.code, name: 'X', platform: 'windows' }).expect(400);
  await request(app).post('/api/agent/pair').send({ code: 'WRONG-CODE1', name: 'X', platform: 'windows' }).expect(400);
  const status = await request(app).get('/api/agent/status').set(paired.auth).expect(200);
  assert.equal(status.body.paused, false);
  // An agent credential is not a user session, and a session is not an agent credential.
  await request(app).get('/api/assets').set(paired.auth).expect(401);
  await admin.get('/api/agent/status').expect(401);
  const tech = await userWithRole(admin, 'technician');
  await tech.agent.post('/api/agents/pairing-codes').expect(403);
});

test('discovered devices wait for review; nothing is invented', async () => {
  const { agent: admin } = await newOrg();
  const pc = await pairedAgent(admin);
  const res = await request(app).post('/api/agent/report').set(pc.auth)
    .send(scan([dellMonitor, usbMouse, usbMouse, { ...usbMouse, localId: '' }])).expect(200);
  assert.equal(res.body.accepted, 2, 'duplicates and devices without an id are dropped');
  assert.equal((await admin.get('/api/assets')).body.total, 0, 'no item is created automatically');

  const queue = (await admin.get('/api/discovered-devices')).body.items;
  assert.equal(queue.length, 2);
  const monitor = queue.find((d) => d.class === 'monitor');
  const mouse = queue.find((d) => d.class === 'peripheral');
  assert.equal(monitor.identifiedBySerial, true);
  assert.equal(mouse.serial, null);

  const item = (await admin.post(`/api/discovered-devices/${monitor.id}/add`).send({ type: 'Monitor' }).expect(200)).body;
  assert.equal(item.serial, 'CN0ABC123');
  assert.equal(item.purchaseDate, null);
  assert.equal(item.warrantyExpires, null);
  const fromMouse = (await admin.post(`/api/discovered-devices/${mouse.id}/add`).send({ type: 'Accessory' }).expect(200)).body;
  assert.equal(fromMouse.serial, null, 'no serial is made up when none was reported');
  await admin.post(`/api/discovered-devices/${monitor.id}/add`).send({ type: 'Monitor' }).expect(409);
  assert.equal((await admin.get('/api/discovered-devices?state=linked')).body.items.length, 2);
});

test('same model is not the same device; serials follow a device between computers', async () => {
  const { agent: admin } = await newOrg();
  const a = await pairedAgent(admin, 'PC-A');
  const b = await pairedAgent(admin, 'PC-B');
  await request(app).post('/api/agent/report').set(a.auth).send(scan([dellMonitor, usbMouse])).expect(200);
  // PC-B has the same mouse model (no serial) and the same monitor (same serial).
  await request(app).post('/api/agent/report').set(b.auth).send(scan([usbMouse])).expect(200);
  const all = (await admin.get('/api/discovered-devices?state=all')).body.items;
  assert.equal(all.filter((d) => d.class === 'peripheral').length, 2, 'two mice, not one');

  const monitor = all.find((d) => d.class === 'monitor');
  const item = (await admin.post(`/api/discovered-devices/${monitor.id}/add`).send({ type: 'Monitor' })).body;
  const moved = await request(app).post('/api/agent/report').set(b.auth).send(scan([usbMouse, dellMonitor])).expect(200);
  assert.equal(moved.body.moved, 1);
  const events = (await admin.get(`/api/assets/${item.id}/events`)).body.items;
  assert.deepEqual(events[0].changes.computer, { from: 'PC-A', to: 'PC-B' });
  // A full scan from PC-A without the monitor marks nothing missing for it: it moved.
  const fromA = await request(app).post('/api/agent/report').set(a.auth).send(scan([usbMouse])).expect(200);
  assert.equal(fromA.body.missing, 0);
  // A full scan from PC-B without the mouse marks its mouse missing.
  const missing = await request(app).post('/api/agent/report').set(b.auth).send(scan([dellMonitor])).expect(200);
  assert.equal(missing.body.missing, 1);
});

test('linking, ignoring, pausing and revoking', async () => {
  const { agent: admin } = await newOrg();
  const pc = await pairedAgent(admin);
  await request(app).post('/api/agent/report').set(pc.auth).send(scan([dellMonitor, usbMouse])).expect(200);
  const [d1, d2] = (await admin.get('/api/discovered-devices')).body.items;
  const existing = (await admin.post('/api/assets').send({ name: 'Front desk monitor', type: 'Monitor' })).body;
  const linked = (await admin.post(`/api/discovered-devices/${d1.id}/link`).send({ assetId: existing.id }).expect(200)).body;
  assert.equal(linked.assetId, existing.id);
  await admin.post(`/api/discovered-devices/${d2.id}/ignore`).expect(200);
  assert.equal((await admin.get('/api/discovered-devices')).body.items.length, 0);
  await admin.post(`/api/discovered-devices/${d2.id}/restore`).expect(200);

  const other = await newOrg();
  await other.agent.post(`/api/discovered-devices/${d2.id}/ignore`).expect(404);
  await other.agent.post(`/api/discovered-devices/${d1.id}/link`).send({ assetId: existing.id }).expect(404);
  await admin.post(`/api/discovered-devices/${d2.id}/link`).send({ assetId: (await other.agent.post('/api/assets').send({ name: 'x', type: 'Other' })).body.id }).expect(404);

  await admin.patch(`/api/agents/${pc.agentId}`).send({ paused: true }).expect(200);
  const paused = await request(app).post('/api/agent/report').set(pc.auth).send(scan([{ ...usbMouse, localId: 'new' }])).expect(200);
  assert.equal(paused.body.paused, true);
  assert.equal(paused.body.accepted, 0);
  await other.agent.delete(`/api/agents/${pc.agentId}`).expect(404);
  await admin.delete(`/api/agents/${pc.agentId}`).expect(204);
  const revoked = await request(app).post('/api/agent/report').set(pc.auth).send(scan([])).expect(401);
  assert.equal(revoked.body.code, 'agent_revoked');
  const agents = (await admin.get('/api/agents')).body.items;
  assert.ok(agents[0].revokedAt);
});

test('paired computers count toward the plan', async () => {
  const { agent } = await personal();
  await pool.query(`UPDATE plan_definitions SET limits = limits || '{"devices": 1}' WHERE code = 'personal_basic'`);
  try {
    await pairedAgent(agent);
    await agent.post('/api/agents/pairing-codes').expect(402);
  } finally {
    await pool.query(`UPDATE plan_definitions SET limits = limits || '{"devices": 2}' WHERE code = 'personal_basic'`);
  }
});

test('the real device companion pairs, reports this computer, and disconnects', async () => {
  const { execFile } = await import('node:child_process');
  const { mkdtempSync } = await import('node:fs');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const run = (args, env) => new Promise((resolve) => {
    execFile(process.env.TEST_AGENT_BIN, args, { env: { ...process.env, ...env } }, (err, stdout, stderr) => resolve({ code: err ? err.code : 0, stdout, stderr }));
  });
  const env = { DT_AGENT_CONFIG: join(mkdtempSync(join(tmpdir(), 'dt-agent-')), 'agent.json') };
  const { agent: admin } = await newOrg('Agent Co');
  const { code } = (await admin.post('/api/agents/pairing-codes').expect(201)).body;

  assert.notEqual((await run(['pair', 'http://example.com', code], env)).code, 0, 'plain http is refused');
  const paired = await run(['pair', app, code, '--name', 'CI-RUNNER'], env);
  assert.equal(paired.code, 0, paired.stderr);
  assert.match(paired.stdout, /Paired CI-RUNNER with Agent Co/);
  const scan = await run(['scan'], env);
  assert.equal(scan.code, 0, scan.stderr);
  const report = await run(['report'], env);
  assert.equal(report.code, 0, report.stderr);
  assert.match(report.stdout, /Sent \d+ devices to Agent Co/);
  const agents = (await admin.get('/api/agents')).body.items;
  assert.equal(agents[0].name, 'CI-RUNNER');
  assert.equal(agents[0].platform, 'linux');

  assert.equal((await run(['pause'], env)).code, 0);
  assert.notEqual((await run(['report'], env)).code, 0, 'paused locally');
  await run(['resume'], env);
  const off = await run(['disconnect'], env);
  assert.match(off.stdout, /Disconnected from Agent Co/);
  assert.ok((await admin.get('/api/agents')).body.items[0].revokedAt);
  assert.notEqual((await run(['report'], env)).code, 0, 'forgotten after disconnecting');
});
