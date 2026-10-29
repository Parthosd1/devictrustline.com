// Phase 6: one queue for everything waiting on a manager, shown only to those who may act on it,
// and follow-up tasks raised from it.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { day, newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const byName = (items, name) => items.find((i) => i.name === name);
let n = 0;

async function busyWorkspace() {
  const org = await newOrg();
  const invs = (await org.agent.get('/api/inventories')).body.items;
  const available = byName(invs, 'Available Inventory');
  const loaner = byName(invs, 'Loaner Inventory');
  const repair = byName(invs, 'RMA / Repair Inventory');
  await org.agent.patch(`/api/inventories/${repair.id}`).send({ requiresApproval: true }).expect(200);
  const manager = await userWithRole(org.agent, 'manager');
  const tech = await userWithRole(org.agent, 'technician');
  const auditor = await userWithRole(org.agent, 'auditor');
  const item = async (inventoryId, name, extra = {}) => (await org.agent.post('/api/assets')
    .send({ name, type: 'Laptop', serial: `AC-${Date.now()}-${++n}`, inventoryId, ...extra }).expect(201)).body;
  const person = (await org.agent.post('/api/people').send({ name: 'Borrower' }).expect(201)).body;

  // A transfer waiting for approval.
  const broken = await item(available.id, 'Broken laptop');
  await tech.agent.post('/api/transfers').send({ assetIds: [broken.id], toInventoryId: repair.id }).expect(201);
  // An audit waiting for review, with an item missing and one damaged.
  const lost = await item(available.id, 'Never found');
  const scratched = await item(available.id, 'Scratched');
  const audit = (await auditor.agent.post('/api/audits').send({ auditType: 'full', inventoryId: available.id }).expect(201)).body;
  await auditor.agent.patch(`/api/audits/${audit.id}/items/${scratched.id}`)
    .send({ result: 'verified', damaged: true, note: 'Scratched lid' }).expect(200);
  await auditor.agent.patch(`/api/audits/${audit.id}/items/${broken.id}`).send({ result: 'verified' }).expect(200);
  await auditor.agent.post(`/api/audits/${audit.id}/submit`).send({}).expect(200);
  // A soft audit already closed, with a missing item.
  const soft = (await auditor.agent.post('/api/audits').send({ auditType: 'soft', assetIds: [lost.id], requiresReview: false }).expect(400));
  void soft;
  const spot = (await auditor.agent.post('/api/audits').send({ auditType: 'spot', assetIds: [lost.id], requiresReview: false }).expect(201)).body;
  await auditor.agent.post(`/api/audits/${spot.id}/close`).expect(200);
  // An overdue loan and a returned one awaiting inspection.
  const late = await item(loaner.id, 'Late loaner');
  const l = (await tech.agent.post('/api/loans').send({ assetId: late.id, personId: person.id, dueOn: day(1), deployNow: true }).expect(201)).body;
  await pool.query('ALTER TABLE loans DISABLE TRIGGER loans_closed_final');
  await pool.query('UPDATE loans SET due_on = $2, starts_on = $2 WHERE id = $1', [l.id, day(-3)]);
  await pool.query('ALTER TABLE loans ENABLE TRIGGER loans_closed_final');
  const back = await item(loaner.id, 'Back loaner');
  const l2 = (await tech.agent.post('/api/loans').send({ assetId: back.id, personId: person.id, dueOn: day(2), deployNow: true }).expect(201)).body;
  await tech.agent.post(`/api/loans/${l2.id}/return`).send({ damaged: true, note: 'Dented' }).expect(200);
  // A warranty ending soon and maintenance due.
  const w = await item(available.id, 'Warranty soon', { maintenanceDue: day(2) });
  await org.agent.post(`/api/assets/${w.id}/warranties`).send({ kind: 'manufacturer', endsOn: day(10) }).expect(201);
  return { ...org, available, manager, tech, auditor, audit, lost, scratched, l, l2, w };
}

test('the approval center gathers every pending decision and follow-up for a manager', async () => {
  const s = await busyWorkspace();
  const q = (await s.manager.agent.get('/api/approvals').expect(200)).body;
  assert.equal(q.counts.transfers, 1);
  assert.equal(q.counts.audits, 1);
  assert.equal(q.sections.audits[0].canReview, true);
  assert.equal(q.sections.audits[0].damaged, 1);
  assert.equal(q.counts.overdueLoans, 1);
  assert.equal(q.sections.overdueLoans[0].daysOverdue, 3);
  assert.equal(q.counts.loansToInspect, 1);
  assert.deepEqual(q.sections.missing.map((m) => m.assetId), [s.lost.id]);
  assert.ok(q.sections.damaged.some((d) => d.source === 'loan'));
  assert.equal(q.sections.warranties.length, 1);
  assert.ok(q.sections.maintenance.some((m) => m.assetId === s.w.id));
  assert.ok('accessRequests' in q.sections);

  // Acting through each area's own route empties the queue.
  await s.manager.agent.post(`/api/transfers/${q.sections.transfers[0].id}/approve`).send({}).expect(200);
  await s.manager.agent.post(`/api/audits/${s.audit.id}/approve`).send({}).expect(200);
  await s.manager.agent.post(`/api/loans/${s.l2.id}/inspect`).send({ result: 'repair', note: 'Dent' }).expect(200);
  const after = (await s.manager.agent.get('/api/approvals')).body;
  assert.equal(after.counts.transfers, 0);
  assert.equal(after.counts.audits, 0);
  assert.equal(after.counts.loansToInspect, 0);
  assert.ok(after.sections.damaged.some((d) => d.source === 'audit'), 'approved audit damage now listed');
});

test('sections follow permissions: technicians and viewers see no decisions', async () => {
  const s = await busyWorkspace();
  const tech = (await s.tech.agent.get('/api/approvals').expect(200)).body;
  assert.ok(!('transfers' in tech.sections) && !('audits' in tech.sections) && !('accessRequests' in tech.sections));
  assert.equal(tech.counts.overdueLoans, 1);
  const viewer = await userWithRole(s.agent, 'viewer');
  const v = (await viewer.agent.get('/api/approvals').expect(200)).body;
  assert.ok(!('transfers' in v.sections));
  const auditorView = (await s.auditor.agent.get('/api/approvals')).body;
  assert.ok(!('audits' in auditorView.sections), 'auditors cannot approve audits');
});

test('follow-up tasks: raised once, assigned, closed by the assignee with a resolution', async () => {
  const s = await busyWorkspace();
  const body = { title: 'Find the missing laptop', kind: 'missing', assetId: s.lost.id, sourceType: 'audit', sourceId: s.audit.id, assigneeId: s.tech.id, dueOn: day(3) };
  await s.tech.agent.post('/api/tasks').send(body).expect(403);
  const task = (await s.manager.agent.post('/api/tasks').send(body).expect(201)).body;
  assert.equal(task.assigneeName, 'technician');
  await s.manager.agent.post('/api/tasks').send(body).expect(409);
  await s.manager.agent.post('/api/tasks').send({ ...body, sourceId: undefined }).expect(400);
  const q = (await s.manager.agent.get('/api/approvals')).body;
  assert.equal(q.sections.missing.find((m) => m.assetId === s.lost.id).hasTask, true);
  assert.equal(q.openTasks, 1);
  await new Promise((r) => setTimeout(r, 200));
  assert.ok((await s.tech.agent.get('/api/notifications')).body.items.some((x) => x.kind === 'task_assigned'));

  assert.equal((await s.tech.agent.get('/api/tasks?mine=true')).body.items.length, 1);
  await s.tech.agent.patch(`/api/tasks/${task.id}`).send({ title: 'Renamed' }).expect(403);
  await s.tech.agent.patch(`/api/tasks/${task.id}`).send({ status: 'done' }).expect(400);
  const done = (await s.tech.agent.patch(`/api/tasks/${task.id}`).send({ status: 'done', resolution: 'Found in locker 4' }).expect(200)).body;
  assert.equal(done.status, 'done');
  assert.ok(done.closedAt);
  const auditor = s.auditor;
  await auditor.agent.patch(`/api/tasks/${task.id}`).send({ status: 'open' }).expect(403);
  assert.equal((await s.manager.agent.get('/api/tasks')).body.items.length, 0);
  assert.equal((await s.manager.agent.get('/api/tasks?status=done')).body.items.length, 1);
  const other = await newOrg();
  await other.agent.patch(`/api/tasks/${task.id}`).send({ status: 'open' }).expect(404);
  await other.agent.post('/api/tasks').send({ title: 'x', assetId: s.lost.id }).expect(400);
});
