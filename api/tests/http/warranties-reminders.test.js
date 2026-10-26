import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { lastEmail, newOrg, pool, resetDatabase, runJobs, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const day = (n) => new Date(Date.now() + n * 86400000).toISOString().slice(0, 10);

test('warranties: several per item, states, and the item shows the latest end date', async () => {
  const { agent } = await newOrg();
  const item = (await agent.post('/api/assets').send({ name: 'Fridge', type: 'Other', warrantyExpires: day(400) }).expect(201)).body;
  let list = (await agent.get(`/api/assets/${item.id}/warranties`)).body.items;
  assert.equal(list.length, 1);
  assert.equal(list[0].kind, 'manufacturer');
  assert.equal(list[0].status, 'active');

  const ext = (await agent.post(`/api/assets/${item.id}/warranties`).send({
    kind: 'extended', provider: 'SquareTrade', startsOn: day(400), endsOn: day(1100), coverage: 'Parts and labor', claimReference: 'ST-1',
  }).expect(201)).body;
  assert.equal((await agent.get(`/api/assets/${item.id}`)).body.warrantyExpires, day(1100));
  await agent.post(`/api/assets/${item.id}/warranties`).send({ startsOn: day(10), endsOn: day(5) }).expect(400);

  const soon = (await agent.patch(`/api/warranties/${ext.id}`).send({ startsOn: day(-355), endsOn: day(10) }).expect(200)).body;
  assert.equal(soon.status, 'expiring_soon');
  assert.equal(soon.daysLeft, 10);
  await agent.patch(`/api/warranties/${ext.id}`).send({ endsOn: day(-400) }).expect(400);
  const unknown = (await agent.post(`/api/assets/${item.id}/warranties`).send({ kind: 'other' }).expect(201)).body;
  assert.equal(unknown.status, 'unknown');

  // Changing the single date on the item edits the manufacturer warranty.
  await agent.patch(`/api/assets/${item.id}`).send({ warrantyExpires: day(-3) }).expect(200);
  list = (await agent.get(`/api/assets/${item.id}/warranties`)).body.items;
  assert.equal(list.find((w) => w.kind === 'manufacturer').status, 'expired');
  await agent.delete(`/api/warranties/${ext.id}`).expect(204);
  assert.equal((await agent.get(`/api/assets/${item.id}`)).body.warrantyStatus, 'expired');

  const other = await newOrg();
  await other.agent.patch(`/api/warranties/${unknown.id}`).send({ provider: 'x' }).expect(404);
  await other.agent.get(`/api/assets/${item.id}/warranties`).expect(404);
});

test('proof of purchase must be a file on the same item', async () => {
  const { agent } = await newOrg();
  const a = (await agent.post('/api/assets').send({ name: 'A', type: 'Other' })).body;
  const b = (await agent.post('/api/assets').send({ name: 'B', type: 'Other' })).body;
  const pdf = Buffer.from('%PDF-1.4 receipt');
  const file = (await agent.post(`/api/assets/${b.id}/attachments`).field('kind', 'receipt').attach('file', pdf, 'r.pdf').expect(201)).body;
  await agent.post(`/api/assets/${a.id}/warranties`).send({ endsOn: day(30), proofAttachmentId: file.id }).expect(400);
  const w = (await agent.post(`/api/assets/${b.id}/warranties`).send({ endsOn: day(30), proofAttachmentId: file.id }).expect(201)).body;
  assert.equal(w.proofFilename, 'r.pdf');
  await agent.delete(`/api/attachments/${file.id}`).expect(204);
  const after = (await agent.get(`/api/assets/${b.id}/warranties`)).body.items[0];
  assert.equal(after.proofAttachmentId, null);
});

test('reminders fire once, on the right day, to the right people, with honest email status', async () => {
  const { agent, email } = await newOrg();
  const viewer = await userWithRole(agent, 'viewer');
  const item = (await agent.post('/api/assets').send({ name: 'Laptop A', type: 'Laptop', warrantyExpires: day(30) }).expect(201)).body;
  const first = await runJobs();
  assert.ok(first.remindersCreated >= 2, 'the 30, 7 and 1 day reminders are scheduled');
  assert.ok(first.remindersFired >= 1, 'the 30-day reminder is due today');
  const again = await runJobs();
  assert.equal(again.remindersCreated, 0);
  assert.equal(again.remindersFired, 0);

  const notes = (await agent.get('/api/notifications')).body;
  const mine = notes.items.filter((n) => n.assetId === item.id);
  assert.equal(mine.length, 1);
  assert.match(mine[0].title, /Laptop A: warranty ends/);
  assert.match(mine[0].body, /in 30 days/);
  assert.equal(mine[0].emailStatus, 'sent');
  assert.equal((await lastEmail(email)).kind, 'reminder');
  assert.equal((await viewer.agent.get('/api/notifications')).body.items.length, 0, 'viewers are not notified');

  await agent.post(`/api/notifications/${mine[0].id}/read`).expect(204);
  assert.equal((await agent.get('/api/notifications?unread=true')).body.unread, notes.unread - 1);
  await agent.post('/api/notifications/read-all').expect(204);
  assert.equal((await agent.get('/api/notifications')).body.unread, 0);
  await viewer.agent.post(`/api/notifications/${mine[0].id}/read`).expect(404);
});

test('changing a date cancels the old reminders and schedules new ones', async () => {
  const { agent } = await newOrg();
  const item = (await agent.post('/api/assets').send({ name: 'Mixer', type: 'Other', returnDeadline: day(20) }).expect(201)).body;
  await runJobs();
  const before = await pool.query("SELECT status, due_on::text FROM reminders WHERE asset_id = $1 AND kind = 'return_window'", [item.id]);
  assert.ok(before.rows.length >= 2 && before.rows.every((r) => r.status === 'pending'));
  await agent.patch(`/api/assets/${item.id}`).send({ returnDeadline: day(40) }).expect(200);
  const result = await runJobs();
  assert.ok(result.remindersCancelled >= 2);
  const rows = (await pool.query("SELECT status, due_on::text AS due FROM reminders WHERE asset_id = $1 AND kind = 'return_window'", [item.id])).rows;
  assert.ok(rows.filter((r) => r.due === day(20)).every((r) => r.status === 'cancelled'));
  assert.ok(rows.some((r) => r.due === day(40) && r.status === 'pending'));
  // Clearing the date cancels everything for it.
  await agent.patch(`/api/assets/${item.id}`).send({ returnDeadline: null }).expect(200);
  await runJobs();
  const left = await pool.query("SELECT count(*)::int AS n FROM reminders WHERE asset_id = $1 AND status = 'pending'", [item.id]);
  assert.equal(left.rows[0].n, 0);
});

test('reminders use the workspace time zone, its own day lists, and owners when set', async () => {
  const { agent } = await newOrg();
  const tech = await userWithRole(agent, 'technician');
  await agent.patch('/api/settings').send({ timezone: 'Not/AZone' }).expect(400);
  const settings = await agent.patch('/api/settings').send({ timezone: 'Pacific/Kiritimati', reminders: { maintenanceDays: [2] } }).expect(200);
  assert.equal(settings.body.timezone, 'Pacific/Kiritimati');
  assert.deepEqual(settings.body.reminders.maintenanceDays, [2]);
  assert.deepEqual(settings.body.reminders.warrantyDays, [90, 30, 7, 1]);
  const { rows: [{ today }] } = await pool.query("SELECT (now() AT TIME ZONE 'Pacific/Kiritimati')::date::text AS today");
  const due = new Date(new Date(`${today}T00:00:00Z`).getTime() + 2 * 86400000).toISOString().slice(0, 10);
  const item = (await agent.post('/api/assets').send({ name: 'Boiler', type: 'Other', maintenanceDue: due, ownerId: tech.id }).expect(201)).body;
  await runJobs();
  const rows = (await pool.query('SELECT days_before, status FROM reminders WHERE asset_id = $1', [item.id])).rows;
  assert.deepEqual(rows, [{ days_before: 2, status: 'sent' }]);
  assert.equal((await tech.agent.get('/api/notifications')).body.items.length, 1, 'the owner is told');
  assert.equal((await agent.get('/api/notifications')).body.items.filter((n) => n.assetId === item.id).length, 0);
});

test('a deadline missed while the server was down is recorded, not announced late', async () => {
  const { agent } = await newOrg();
  const item = (await agent.post('/api/assets').send({ name: 'Old', type: 'Other', maintenanceDue: day(1) }).expect(201)).body;
  await pool.query(
    `INSERT INTO reminders (org_id, asset_id, kind, source_id, due_on, remind_on, days_before, title, dedupe_key)
     SELECT org_id, id, 'maintenance', 'gone', current_date - 2, current_date - 3, 1, 'x', 'test-missed' FROM assets WHERE id = $1`, [item.id]);
  await runJobs();
  const { rows } = await pool.query("SELECT status FROM reminders WHERE dedupe_key = 'test-missed'");
  assert.equal(rows[0].status, 'cancelled', 'a reminder whose deadline no longer exists is cancelled');
});

test('loans: due dates, deadlines list and overdue insight', async () => {
  const { agent } = await newOrg();
  const person = (await agent.post('/api/people').send({ name: 'Jo' }).expect(201)).body;
  const item = (await agent.post('/api/assets').send({ name: 'Camera', type: 'Other' }).expect(201)).body;
  const lent = (await agent.post(`/api/assets/${item.id}/assign`).send({ personId: person.id, dueDate: day(-2) }).expect(200)).body;
  assert.equal(lent.loanDue, day(-2));
  const deadlines = (await agent.get('/api/deadlines')).body.items;
  assert.ok(deadlines.some((d) => d.kind === 'loan' && d.daysLeft === -2));
  const insights = (await agent.get('/api/insights')).body.items;
  const overdue = insights.find((i) => i.kind === 'overdue_returns');
  assert.equal(overdue.items[0].name, 'Camera');
  assert.ok(overdue.explanation);
  const back = (await agent.post(`/api/assets/${item.id}/return`).send({}).expect(200)).body;
  assert.equal(back.loanDue, null);
});
