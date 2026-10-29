// Phase 11: the 14 critical workflows from the production spec, end to end over HTTP, in order.
// Each test is one numbered workflow; later ones build on the workspace the earlier ones set up.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { day, newOrg, newPersonal, pool, resetDatabase, runJobs, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const byName = (items, name) => items.find((i) => i.name === name);
const notes = async (who, kind) => (await who.agent.get('/api/notifications?limit=200')).body.items.filter((n) => n.kind === kind);
const s = {};

before(async () => {
  s.owner = await newOrg('Critical Co');
  s.manager = await userWithRole(s.owner.agent, 'manager');
  s.tech = await userWithRole(s.owner.agent, 'technician');
  s.auditor = await userWithRole(s.owner.agent, 'auditor');
  const invs = (await s.owner.agent.get('/api/inventories')).body.items;
  s.hub = byName(invs, 'Main IT Hub');
  s.available = byName(invs, 'Available Inventory');
  s.loaner = byName(invs, 'Loaner Inventory');
  s.borrower = (await s.owner.agent.post('/api/people').send({ name: 'Sam Lee', email: 'sam@example.com' }).expect(201)).body;
  s.other = (await s.owner.agent.post('/api/people').send({ name: 'Ana Ruiz', email: 'ana@example.com' }).expect(201)).body;
});

test('1. a personal user creates a device and schedules a warranty reminder', async () => {
  const p = await newPersonal('Pat');
  s.personal = p;
  const tv = (await p.agent.post('/api/assets')
    .send({ name: 'Living room TV', type: 'TVs & Monitors', serial: 'TV-123', warrantyExpires: day(1) }).expect(201)).body;
  const r = (await p.agent.post('/api/reminders')
    .send({ kind: 'warranty', assetId: tv.id, dateSource: 'warranty', offsets: [1] }).expect(201)).body;
  assert.equal(r.dueOn, day(1), 'the reminder follows the item’s warranty date');
  assert.equal(r.status, 'active');
  s.reminder = r;
  await p.agent.patch('/api/notification-settings').send({ sendHour: 0 }).expect(200);
  await runJobs();
  const sent = (await p.agent.get('/api/notifications')).body.items.filter((n) => n.reminderId === r.id);
  assert.equal(sent.length, 1, 'the reminder is delivered in the app');
});

test('2. the user edits, snoozes and disables a reminder', async () => {
  const { agent } = s.personal;
  const edited = (await agent.patch(`/api/reminders/${s.reminder.id}`).send({ title: 'TV warranty', offsets: [7, 1] }).expect(200)).body;
  assert.equal(edited.title, 'TV warranty');
  assert.deepEqual(edited.offsets, [7, 1]);
  const snoozed = (await agent.post(`/api/reminders/${s.reminder.id}/snooze`).send({ hours: 24 }).expect(200)).body;
  assert.ok(new Date(snoozed.snoozedUntil) > new Date());
  const paused = (await agent.patch(`/api/reminders/${s.reminder.id}`).send({ status: 'paused' }).expect(200)).body;
  assert.equal(paused.status, 'paused');
  const before = (await agent.get('/api/notifications')).body.items.length;
  await runJobs();
  assert.equal((await agent.get('/api/notifications')).body.items.length, before, 'a paused reminder sends nothing');
});

test('3. a manager creates a sub-inventory and delegates access to it', async () => {
  // The technician starts with access to the loaner inventory only.
  await s.owner.agent.patch(`/api/users/${s.tech.id}`).send({ accessScope: 'inventories', inventoryIds: [s.loaner.id] }).expect(200);
  s.kits = (await s.manager.agent.post('/api/inventories')
    .send({ name: 'Field Kits', kind: 'technician', parentId: s.hub.id, custodianId: s.tech.id }).expect(201)).body;
  assert.equal(s.kits.path, 'Main IT Hub / Field Kits');
  await s.tech.agent.get(`/api/inventories/${s.kits.id}`).expect(404);
  const asked = (await s.tech.agent.post('/api/access-requests').send({ inventoryId: s.kits.id, note: 'My kit' }).expect(201)).body;
  await s.manager.agent.post(`/api/access-requests/${asked.id}/approve`).send({}).expect(200);
  assert.equal((await s.tech.agent.get(`/api/inventories/${s.kits.id}`).expect(200)).body.name, 'Field Kits');
  assert.equal((await notes(s.tech, 'access_approved')).length, 1);
});

test('4. a technician transfers a device into the sub-inventory', async () => {
  const laptop = (await s.owner.agent.post('/api/assets')
    .send({ name: 'Kit laptop', type: 'Laptop', serial: 'KIT-1', inventoryId: s.loaner.id }).expect(201)).body;
  const moved = (await s.tech.agent.post('/api/transfers').send({ codes: [laptop.assetTag], toInventoryId: s.kits.id, reason: 'Kit build' }).expect(201)).body;
  assert.equal(moved.completed.length, 1);
  assert.equal((await s.tech.agent.get(`/api/assets/${laptop.id}`)).body.inventoryId, s.kits.id);
  const history = (await s.owner.agent.get(`/api/transfers?assetId=${laptop.id}`)).body.items;
  assert.equal(history[0].requestedBy, 'technician');
});

test('5. a technician deploys a loaner', async () => {
  s.loanItem = (await s.owner.agent.post('/api/assets')
    .send({ name: 'Loaner laptop', type: 'Laptop', serial: 'LOAN-1', inventoryId: s.loaner.id }).expect(201)).body;
  const loan = (await s.tech.agent.post('/api/loans')
    .send({ assetId: s.loanItem.id, personId: s.borrower.id, dueOn: day(3), deployNow: true, acknowledgedName: 'Sam Lee' }).expect(201)).body;
  assert.equal(loan.status, 'deployed');
  assert.equal(loan.acknowledgedName, 'Sam Lee');
  s.loan = loan;
  assert.equal((await s.owner.agent.get(`/api/assets/${s.loanItem.id}`)).body.status, 'Deployed');
});

test('6. an overdue loaner triggers one notification', async () => {
  await pool.query('UPDATE loans SET due_on = $2, starts_on = $2 WHERE id = $1', [s.loan.id, day(-1)]);
  await runJobs();
  await runJobs();
  await runJobs();
  assert.equal((await notes(s.manager, 'loan_overdue')).length, 1, 'once, however often the job runs');
  assert.equal((await s.manager.agent.get('/api/loans?overdue=true')).body.items.length, 1);
});

test('7. the loaner is returned and inspected', async () => {
  const back = (await s.tech.agent.post(`/api/loans/${s.loan.id}/return`).send({ condition: 'Good' }).expect(200)).body;
  assert.equal(back.status, 'returned');
  const done = (await s.tech.agent.post(`/api/loans/${s.loan.id}/inspect`).send({ result: 'ready', note: 'Wiped and charged' }).expect(200)).body;
  assert.equal(done.status, 'inspected');
  assert.equal((await s.owner.agent.get(`/api/assets/${s.loanItem.id}`)).body.status, 'Available');
  assert.deepEqual((await s.owner.agent.get(`/api/loans/${s.loan.id}`)).body.events.map((e) => e.kind),
    ['reserved', 'deployed', 'overdue', 'returned', 'inspected']);
});

test('8. an auditor starts a full hub audit', async () => {
  s.audit = (await s.auditor.agent.post('/api/audits').send({ auditType: 'full', period: 'manual', inventoryId: s.hub.id }).expect(201)).body;
  assert.equal(s.audit.status, 'open');
  assert.equal(s.audit.requiresReview, true);
  assert.equal(s.audit.counts.expected, 2, 'every item in the hub and its sub-inventories');
});

test('9. the auditor scans assets and submits the audit', async () => {
  const run = (await s.auditor.agent.get(`/api/audits/${s.audit.id}`)).body;
  const [first] = run.items;
  await s.auditor.agent.post(`/api/audits/${s.audit.id}/scan`).send({ code: first.assetTag }).expect(200);
  const submitted = (await s.auditor.agent.post(`/api/audits/${s.audit.id}/submit`).send({ note: 'One not found' }).expect(200)).body;
  assert.equal(submitted.status, 'submitted');
  assert.equal(submitted.counts.verified, 1);
  assert.ok((await notes(s.manager, 'audit_submitted')).length >= 1);
});

test('10. the manager rejects the audit with a reason', async () => {
  await s.manager.agent.post(`/api/audits/${s.audit.id}/reject`).send({}).expect(400);
  const rejected = (await s.manager.agent.post(`/api/audits/${s.audit.id}/reject`).send({ note: 'Check the field kits shelf' }).expect(200)).body;
  assert.equal(rejected.status, 'rejected');
  assert.equal(rejected.reviewNote, 'Check the field kits shelf');
  assert.ok((await notes(s.auditor, 'audit_rejected')).length >= 1);
});

test('11. the auditor corrects and resubmits it', async () => {
  await s.auditor.agent.post(`/api/audits/${s.audit.id}/reopen`).send({ note: 'Recounting' }).expect(200);
  const run = (await s.auditor.agent.get(`/api/audits/${s.audit.id}`)).body;
  for (const i of run.items.filter((x) => x.result !== 'verified')) {
    await s.auditor.agent.post(`/api/audits/${s.audit.id}/scan`).send({ code: i.assetTag }).expect(200);
  }
  const again = (await s.auditor.agent.post(`/api/audits/${s.audit.id}/submit`).send({ note: 'All found' }).expect(200)).body;
  assert.equal(again.status, 'submitted');
  assert.equal(again.counts.verified, 2);
});

test('12. the manager approves and the audit is locked', async () => {
  const approved = (await s.manager.agent.post(`/api/audits/${s.audit.id}/approve`).send({ note: 'Good' }).expect(200)).body;
  assert.equal(approved.status, 'approved');
  assert.equal(approved.locked, true);
  await s.auditor.agent.post(`/api/audits/${s.audit.id}/reopen`).send({}).expect(409);
  await assert.rejects(pool.query("UPDATE audit_items SET note = 'x' WHERE run_id = $1", [s.audit.id]), 'the database refuses changes');
  const full = (await s.owner.agent.get(`/api/audits/${s.audit.id}`)).body;
  assert.deepEqual(full.reviews.map((r) => r.action), ['started', 'submitted', 'rejected', 'reopened', 'submitted', 'approved']);
});

test('13. an unauthorized user cannot access another workspace', async () => {
  const outsider = await newOrg('Elsewhere Inc');
  const id = s.loanItem.id;
  await outsider.agent.get(`/api/assets/${id}`).expect(404);
  await outsider.agent.patch(`/api/assets/${id}`).send({ name: 'Mine now' }).expect(404);
  await outsider.agent.get(`/api/loans/${s.loan.id}`).expect(404);
  await outsider.agent.get(`/api/audits/${s.audit.id}`).expect(404);
  await outsider.agent.get(`/api/inventories/${s.kits.id}`).expect(404);
  await outsider.agent.post('/api/transfers').send({ assetIds: [id], toInventoryId: s.kits.id }).expect(400);
  const theirs = (await outsider.agent.get('/api/assets?limit=500')).body.items;
  assert.ok(!theirs.some((a) => a.id === id));
  // Not signed in at all.
  const anon = (await import('supertest')).default(process.env.TEST_API_URL);
  await anon.get(`/api/assets/${id}`).expect(401);
  // The personal user from workflow 1 cannot see the business either.
  await s.personal.agent.get(`/api/assets/${id}`).expect(404);
});

test('14. concurrent actions cannot double-loan or double-transfer an asset', async () => {
  const item = (await s.owner.agent.post('/api/assets')
    .send({ name: 'Contested laptop', type: 'Laptop', serial: 'RACE-1', inventoryId: s.loaner.id }).expect(201)).body;
  const loans = await Promise.all([s.borrower, s.other, s.borrower, s.other, s.borrower, s.other].map((p, i) =>
    (i % 2 ? s.tech : s.manager).agent.post('/api/loans').send({ assetId: item.id, personId: p.id, dueOn: day(5), deployNow: true })));
  assert.deepEqual(loans.map((r) => r.status).sort(), [201, 409, 409, 409, 409, 409]);
  const { rows: [{ open }] } = await pool.query(
    "SELECT count(*)::int AS open FROM loans WHERE asset_id = $1 AND status IN ('reserved', 'deployed', 'returned')", [item.id]);
  assert.equal(open, 1);

  const box = (await s.owner.agent.post('/api/assets')
    .send({ name: 'Contested monitor', type: 'Monitor', serial: 'RACE-2', inventoryId: s.available.id }).expect(201)).body;
  const targets = [s.kits.id, s.loaner.id, s.kits.id, s.loaner.id, s.kits.id, s.loaner.id];
  const moves = await Promise.all(targets.map((to, i) => (i % 2 ? s.manager : s.owner).agent.post('/api/transfers')
    .send({ assetIds: [box.id], toInventoryId: to, fromInventoryId: s.available.id })));
  const moved = moves.filter((r) => r.status === 201 && r.body.completed.length === 1);
  assert.equal(moved.length, 1, 'only one transfer moves it out of Available Inventory');
  const { rows } = await pool.query("SELECT count(*)::int AS n FROM inventory_transfers WHERE asset_id = $1 AND status = 'completed'", [box.id]);
  assert.equal(rows[0].n, 1);

  // With approval required, only one transfer can be waiting for an item at a time.
  await s.owner.agent.patch(`/api/inventories/${s.kits.id}`).send({ requiresApproval: true }).expect(200);
  const spare = (await s.owner.agent.post('/api/assets')
    .send({ name: 'Contested dock', type: 'Other', serial: 'RACE-3', inventoryId: s.loaner.id }).expect(201)).body;
  await Promise.all(Array.from({ length: 6 }, () => s.tech.agent.post('/api/transfers').send({ assetIds: [spare.id], toInventoryId: s.kits.id })));
  const pending = await pool.query("SELECT count(*)::int AS n FROM inventory_transfers WHERE asset_id = $1 AND status = 'pending'", [spare.id]);
  assert.equal(pending.rows[0].n, 1);
});
