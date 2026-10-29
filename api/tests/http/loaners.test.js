// Phase 4: loaners. Reserved → Deployed → Returned → Inspected → Available, with cancellation,
// extension, damaged returns, loss, overdue notices, history and reports.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { day, newOrg, pool, resetDatabase, runJobs, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const byName = (items, name) => items.find((i) => i.name === name);
let n = 0;

async function setup() {
  const org = await newOrg();
  const invs = (await org.agent.get('/api/inventories')).body.items;
  const loaner = byName(invs, 'Loaner Inventory');
  const available = byName(invs, 'Available Inventory');
  const tech = await userWithRole(org.agent, 'technician');
  const manager = await userWithRole(org.agent, 'manager');
  const person = (await org.agent.post('/api/people').send({ name: 'Priya Patel', email: `priya${++n}@example.com` }).expect(201)).body;
  const item = async (inventoryId, name = 'Loaner laptop') => (await org.agent.post('/api/assets')
    .send({ name, type: 'Laptop', serial: `LN-${Date.now()}-${++n}`, inventoryId }).expect(201)).body;
  return { ...org, loaner, available, tech, manager, person, item };
}

test('workflow: a loaner goes Reserved → Deployed → Returned → Inspected and is available again', async () => {
  const s = await setup();
  const laptop = await s.item(s.loaner.id);
  const loan = (await s.tech.agent.post('/api/loans')
    .send({ assetId: laptop.id, personId: s.person.id, dueOn: day(7), purpose: 'Field visit' }).expect(201)).body;
  assert.equal(loan.status, 'reserved');
  assert.match(loan.number, /^LN-\d+$/);
  await s.tech.agent.post('/api/loans').send({ assetId: laptop.id, personId: s.person.id, dueOn: day(7) }).expect(409);
  // A plain assignment cannot sidestep the loan once it is out.
  const out = (await s.tech.agent.post(`/api/loans/${loan.id}/deploy`).send({ acknowledgedName: 'Priya Patel' }).expect(200)).body;
  assert.equal(out.status, 'deployed');
  assert.ok(out.acknowledgedAt);
  let asset = (await s.agent.get(`/api/assets/${laptop.id}`)).body;
  assert.equal(asset.status, 'Deployed');
  assert.equal(asset.assignedPersonId, s.person.id);
  assert.equal(asset.loanDue, day(7));
  await s.agent.post(`/api/assets/${laptop.id}/return`).send({}).expect(409);
  await s.tech.agent.post(`/api/loans/${loan.id}/inspect`).send({ result: 'ready' }).expect(409);

  const back = (await s.tech.agent.post(`/api/loans/${loan.id}/return`).send({ condition: 'Good' }).expect(200)).body;
  assert.equal(back.status, 'returned');
  asset = (await s.agent.get(`/api/assets/${laptop.id}`)).body;
  assert.equal(asset.assignedPersonId, null);
  assert.equal(asset.loanDue, null);
  // Not lendable again until inspected.
  await s.tech.agent.post('/api/loans').send({ assetId: laptop.id, personId: s.person.id, dueOn: day(3) }).expect(409);
  const done = (await s.tech.agent.post(`/api/loans/${loan.id}/inspect`).send({ result: 'ready' }).expect(200)).body;
  assert.equal(done.status, 'inspected');
  assert.ok(done.closedAt);
  assert.equal((await s.agent.get(`/api/assets/${laptop.id}`)).body.status, 'Available');

  const full = (await s.agent.get(`/api/loans/${loan.id}`).expect(200)).body;
  assert.deepEqual(full.events.map((e) => e.kind), ['reserved', 'deployed', 'returned', 'inspected']);
  const history = (await s.agent.get(`/api/assets/${laptop.id}/events`)).body.items.map((e) => e.type);
  assert.ok(history.includes('loan_reserved') && history.includes('loan_inspected'));
  // History is permanent and closed loans never change.
  await assert.rejects(pool.query('DELETE FROM loan_events WHERE loan_id = $1', [loan.id]));
  await assert.rejects(pool.query("UPDATE loans SET due_on = '2099-01-01' WHERE id = $1", [loan.id]));
  // And the item can be lent again.
  await s.tech.agent.post('/api/loans').send({ code: laptop.assetTag, personId: s.person.id, dueOn: day(2), deployNow: true }).expect(201);
});

test('only items in a loaner inventory, available and unassigned, can be lent', async () => {
  const s = await setup();
  const spare = await s.item(s.available.id, 'Spare');
  await s.tech.agent.post('/api/loans').send({ assetId: spare.id, personId: s.person.id, dueOn: day(3) }).expect(409);
  const sub = (await s.agent.post('/api/inventories').send({ name: 'Loaner shelf B', parentId: s.loaner.id }).expect(201)).body;
  const nested = await s.item(sub.id, 'Nested loaner');
  await s.tech.agent.post('/api/loans').send({ assetId: nested.id, personId: s.person.id, dueOn: day(3) }).expect(201);
  const past = await s.item(s.loaner.id, 'Past');
  await s.tech.agent.post('/api/loans').send({ assetId: past.id, personId: s.person.id, dueOn: day(-1) }).expect(400);
  await s.tech.agent.post('/api/loans')
    .send({ assetId: past.id, personId: s.person.id, startsOn: day(5), dueOn: day(4) }).expect(400);
  const viewer = await userWithRole(s.agent, 'viewer');
  await viewer.agent.post('/api/loans').send({ assetId: past.id, personId: s.person.id, dueOn: day(3) }).expect(403);
  assert.ok((await viewer.agent.get('/api/loans').expect(200)).body.items.length >= 1);
  const inactive = (await s.agent.post('/api/people').send({ name: 'Gone' }).expect(201)).body;
  await s.agent.patch(`/api/people/${inactive.id}`).send({ isActive: false }).expect(200);
  await s.tech.agent.post('/api/loans').send({ assetId: past.id, personId: inactive.id, dueOn: day(3) }).expect(409);
});

test('exceptions: cancel a reservation, extend (once for technicians), damaged return, repair, lost', async () => {
  const s = await setup();
  const a = await s.item(s.loaner.id, 'A');
  const r = (await s.tech.agent.post('/api/loans').send({ assetId: a.id, personId: s.person.id, dueOn: day(3) }).expect(201)).body;
  assert.equal((await s.tech.agent.post(`/api/loans/${r.id}/cancel`).send({ reason: 'Not needed' }).expect(200)).body.status, 'cancelled');
  await s.tech.agent.post(`/api/loans/${r.id}/deploy`).send({}).expect(409);

  const b = await s.item(s.loaner.id, 'B');
  const l = (await s.tech.agent.post('/api/loans').send({ assetId: b.id, personId: s.person.id, dueOn: day(3), deployNow: true }).expect(201)).body;
  assert.equal(l.status, 'deployed');
  await s.tech.agent.post(`/api/loans/${l.id}/extend`).send({ dueOn: day(2), reason: 'x' }).expect(400);
  await s.tech.agent.post(`/api/loans/${l.id}/extend`).send({ dueOn: day(10) }).expect(400);
  const ext = (await s.tech.agent.post(`/api/loans/${l.id}/extend`).send({ dueOn: day(10), reason: 'Trip extended' }).expect(200)).body;
  assert.equal(ext.dueOn, day(10));
  assert.equal((await s.agent.get(`/api/assets/${b.id}`)).body.loanDue, day(10));
  await s.tech.agent.post(`/api/loans/${l.id}/extend`).send({ dueOn: day(12), reason: 'Again' }).expect(403);
  await s.manager.agent.post(`/api/loans/${l.id}/extend`).send({ dueOn: day(12), reason: 'Approved' }).expect(200);

  await s.tech.agent.post(`/api/loans/${l.id}/return`).send({ damaged: true }).expect(400);
  await s.tech.agent.post(`/api/loans/${l.id}/return`).send({ damaged: true, condition: 'Poor', note: 'Cracked hinge' }).expect(200);
  assert.equal((await s.agent.get(`/api/assets/${b.id}`)).body.status, 'Maintenance');
  await s.tech.agent.post(`/api/loans/${l.id}/inspect`).send({ result: 'repair' }).expect(400);
  const insp = (await s.tech.agent.post(`/api/loans/${l.id}/inspect`).send({ result: 'repair', note: 'Send to vendor' }).expect(200)).body;
  assert.equal(insp.inspectionResult, 'repair');
  assert.equal((await s.agent.get(`/api/assets/${b.id}`)).body.status, 'Maintenance');

  const c = await s.item(s.loaner.id, 'C');
  const gone = (await s.tech.agent.post('/api/loans').send({ assetId: c.id, personId: s.person.id, dueOn: day(3), deployNow: true }).expect(201)).body;
  await s.tech.agent.post(`/api/loans/${gone.id}/lost`).send({ note: 'Left in taxi' }).expect(403);
  await s.manager.agent.post(`/api/loans/${gone.id}/lost`).send({}).expect(400);
  assert.equal((await s.manager.agent.post(`/api/loans/${gone.id}/lost`).send({ note: 'Left in taxi' }).expect(200)).body.status, 'lost');
  await s.tech.agent.post('/api/loans').send({ assetId: c.id, personId: s.person.id, dueOn: day(3) }).expect(409);
});

test('overdue loans are flagged, reported once per due date, and summarised in reports and CSV', async () => {
  const s = await setup();
  const d = await s.item(s.loaner.id, 'Overdue one');
  await s.agent.patch(`/api/inventories/${s.loaner.id}`).send({ custodianId: s.tech.id }).expect(200);
  const l = (await s.tech.agent.post('/api/loans').send({ assetId: d.id, personId: s.person.id, dueOn: day(1), deployNow: true }).expect(201)).body;
  await pool.query("ALTER TABLE loans DISABLE TRIGGER loans_closed_final");
  await pool.query('UPDATE loans SET due_on = $2, starts_on = $2 WHERE id = $1', [l.id, day(-2)]);
  await pool.query("ALTER TABLE loans ENABLE TRIGGER loans_closed_final");
  await runJobs();
  await runJobs();
  const listed = (await s.agent.get('/api/loans?overdue=true').expect(200)).body;
  assert.deepEqual(listed.items.map((i) => i.id), [l.id]);
  assert.equal(listed.items[0].daysOverdue, 2);
  assert.equal(listed.summary.overdue, 1);
  assert.equal(listed.summary.deployed, 1);
  const events = (await s.agent.get(`/api/loans/${l.id}`)).body.events.filter((e) => e.kind === 'overdue');
  assert.equal(events.length, 1, 'one overdue record per due date');
  for (const who of [s, s.manager, s.tech]) {
    const notes = (await who.agent.get('/api/notifications')).body.items.filter((x) => x.kind === 'loan_overdue');
    assert.equal(notes.length, 1, 'owner, manager and the inventory custodian are told once');
  }
  const csv = (await s.agent.get('/api/loans/export').expect(200)).text;
  assert.match(csv.split('\r\n')[0], /"Loan","Status","Asset tag"/);
  assert.ok(csv.includes(l.number));
});

test('loans respect limited access and other workspaces', async () => {
  const s = await setup();
  const mine = await s.item(s.loaner.id, 'Mine');
  const l = (await s.agent.post('/api/loans').send({ assetId: mine.id, personId: s.person.id, dueOn: day(3) }).expect(201)).body;
  const scoped = await userWithRole(s.agent, 'technician');
  await s.agent.patch(`/api/users/${scoped.id}`).send({ accessScope: 'inventories', inventoryIds: [s.available.id] }).expect(200);
  await scoped.agent.get(`/api/loans/${l.id}`).expect(404);
  await scoped.agent.post(`/api/loans/${l.id}/deploy`).send({}).expect(404);
  assert.equal((await scoped.agent.get('/api/loans')).body.items.length, 0);
  const other = await setup();
  await other.agent.get(`/api/loans/${l.id}`).expect(404);
  await other.agent.post('/api/loans').send({ assetId: mine.id, personId: other.person.id, dueOn: day(3) }).expect(400);
  await s.agent.post('/api/loans').send({ assetId: (await s.item(s.loaner.id)).id, personId: other.person.id, dueOn: day(3) }).expect(400);
});
