// Phase 5: audit types, frequencies, Draft → In Progress → Submitted → Approved / Rejected → Locked,
// no self-approval, damaged and duplicate-scan tracking, CSV report, scope.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase, runScheduledAudits, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const byName = (items, name) => items.find((i) => i.name === name);
let n = 0;

async function setup() {
  const org = await newOrg();
  const invs = (await org.agent.get('/api/inventories')).body.items;
  const hub = byName(invs, 'Main IT Hub');
  const available = byName(invs, 'Available Inventory');
  const returned = byName(invs, 'Returned Equipment');
  const auditor = await userWithRole(org.agent, 'auditor');
  const manager = await userWithRole(org.agent, 'manager');
  const item = async (inventoryId, name = 'Laptop') => (await org.agent.post('/api/assets')
    .send({ name, type: 'Laptop', serial: `AU-${Date.now()}-${++n}`, inventoryId }).expect(201)).body;
  return { ...org, hub, available, returned, auditor, manager, item };
}

test('workflow: a full hub audit goes draft → in progress → submitted → approved and locks', async () => {
  const s = await setup();
  const a = await s.item(s.available.id, 'A');
  const b = await s.item(s.available.id, 'B');
  const outside = await s.item(null, 'Unassigned');
  const draft = (await s.auditor.agent.post('/api/audits')
    .send({ auditType: 'full', period: 'quarterly', inventoryId: s.hub.id, draft: true }).expect(201)).body;
  assert.equal(draft.status, 'draft');
  assert.equal(draft.requiresReview, true);
  assert.equal(draft.counts.expected, 2, 'items in the hub and its sub-inventories, not elsewhere');
  await s.auditor.agent.post(`/api/audits/${draft.id}/scan`).send({ code: a.assetTag }).expect(409);
  await s.auditor.agent.post(`/api/audits/${draft.id}/begin`).expect(200);

  await s.auditor.agent.post(`/api/audits/${draft.id}/scan`).send({ code: a.assetTag }).expect(200);
  const again = (await s.auditor.agent.post(`/api/audits/${draft.id}/scan`).send({ code: a.assetTag }).expect(200)).body;
  assert.equal(again.duplicate, true);
  assert.equal(again.scanCount, 2);
  await s.auditor.agent.patch(`/api/audits/${draft.id}/items/${b.id}`).send({ result: 'verified', damaged: true }).expect(400);
  const dmg = (await s.auditor.agent.patch(`/api/audits/${draft.id}/items/${b.id}`)
    .send({ result: 'verified', damaged: true, note: 'Cracked screen' }).expect(200)).body;
  assert.equal(dmg.damaged, true);
  await s.auditor.agent.post(`/api/audits/${draft.id}/close`).expect(409);

  const submitted = (await s.auditor.agent.post(`/api/audits/${draft.id}/submit`).send({ note: 'Done' }).expect(200)).body;
  assert.equal(submitted.status, 'submitted');
  assert.equal(submitted.counts.damaged, 1);
  assert.equal(submitted.counts.duplicateScans, 1);
  await s.auditor.agent.post(`/api/audits/${draft.id}/scan`).send({ code: b.assetTag }).expect(409);
  // Auditors cannot approve, and nobody approves their own audit.
  await s.auditor.agent.post(`/api/audits/${draft.id}/approve`).send({}).expect(403);
  await new Promise((r) => setTimeout(r, 200));
  assert.ok((await s.manager.agent.get('/api/notifications')).body.items.some((x) => x.kind === 'audit_submitted'));

  const approved = (await s.manager.agent.post(`/api/audits/${draft.id}/approve`).send({ note: 'Looks right' }).expect(200)).body;
  assert.equal(approved.status, 'approved');
  assert.equal(approved.locked, true);
  assert.equal(approved.reviewedBy, 'manager');
  await s.manager.agent.post(`/api/audits/${draft.id}/reject`).send({ note: 'x' }).expect(409);
  await assert.rejects(pool.query("UPDATE audit_runs SET name = 'x' WHERE id = $1", [draft.id]));
  await assert.rejects(pool.query("UPDATE audit_items SET note = 'x' WHERE run_id = $1", [draft.id]));
  const full = (await s.agent.get(`/api/audits/${draft.id}`).expect(200)).body;
  assert.deepEqual(full.reviews.map((r) => r.action), ['started', 'submitted', 'approved']);
  const history = (await s.agent.get(`/api/assets/${a.id}/events`)).body.items;
  assert.ok(history.some((e) => e.type === 'audited'));
  assert.ok(!(await s.agent.get(`/api/assets/${outside.id}/events`)).body.items.some((e) => e.type === 'audited'));
  const csv = (await s.agent.get(`/api/audits/${draft.id}/export`).expect(200)).text;
  assert.match(csv.split('\r\n')[0], /"Asset tag","Item"/);
  assert.ok(csv.includes('Cracked screen'));
  assert.ok((await s.auditor.agent.get('/api/notifications')).body.items.some((x) => x.kind === 'audit_approved'));
});

test('rejection sends an audit back for rework, with history; owners cannot approve their own', async () => {
  const s = await setup();
  await s.item(s.available.id);
  const run = (await s.agent.post('/api/audits').send({ auditType: 'transfer', inventoryId: s.available.id }).expect(201)).body;
  assert.equal(run.period, 'manual');
  await s.agent.post(`/api/audits/${run.id}/submit`).send({}).expect(200);
  await s.agent.post(`/api/audits/${run.id}/approve`).send({}).expect(403);
  await s.manager.agent.post(`/api/audits/${run.id}/reject`).send({}).expect(400);
  const rejected = (await s.manager.agent.post(`/api/audits/${run.id}/reject`).send({ note: 'Recount shelf 2' }).expect(200)).body;
  assert.equal(rejected.status, 'rejected');
  await s.agent.post(`/api/audits/${run.id}/approve`).send({}).expect(409);
  const reopened = (await s.agent.post(`/api/audits/${run.id}/reopen`).send({ note: 'Recounting' }).expect(200)).body;
  assert.equal(reopened.status, 'open');
  // The second submission is by the manager, so now only someone else may approve.
  await s.manager.agent.post(`/api/audits/${run.id}/submit`).send({}).expect(200);
  await s.manager.agent.post(`/api/audits/${run.id}/approve`).send({}).expect(403);
  const admin = await userWithRole(s.agent, 'admin');
  await admin.agent.post(`/api/audits/${run.id}/approve`).send({}).expect(200);
  const reviews = (await s.agent.get(`/api/audits/${run.id}`)).body.reviews.map((r) => r.action);
  assert.deepEqual(reviews, ['started', 'submitted', 'rejected', 'reopened', 'submitted', 'approved']);
});

test('types: soft closes without review, spot takes chosen items, return defaults to returned equipment', async () => {
  const s = await setup();
  const a = await s.item(s.available.id, 'Spot A');
  await s.item(s.available.id, 'Spot B');
  const r = await s.item(s.returned.id, 'Came back');
  const soft = (await s.auditor.agent.post('/api/audits').send({ auditType: 'soft', inventoryId: s.available.id }).expect(201)).body;
  assert.equal(soft.requiresReview, false);
  await s.auditor.agent.post(`/api/audits/${soft.id}/submit`).send({}).expect(409);
  assert.equal((await s.auditor.agent.post(`/api/audits/${soft.id}/close`).expect(200)).body.status, 'closed');

  await s.auditor.agent.post('/api/audits').send({ auditType: 'spot' }).expect(400);
  const spot = (await s.auditor.agent.post('/api/audits').send({ auditType: 'spot', assetIds: [a.id] }).expect(201)).body;
  assert.equal(spot.counts.expected, 1);
  await s.auditor.agent.post('/api/audits').send({ auditType: 'spot', assetIds: ['00000000-0000-4000-8000-000000000000'] }).expect(400);
  await s.auditor.agent.post('/api/audits').send({ auditType: 'full', assetIds: [a.id] }).expect(400);
  await s.auditor.agent.post('/api/audits').send({ auditType: 'transfer' }).expect(400);

  const ret = (await s.auditor.agent.post('/api/audits').send({ auditType: 'return' }).expect(201)).body;
  assert.equal(ret.inventoryName, 'Returned Equipment');
  assert.equal(ret.counts.expected, 1);
  const items = (await s.auditor.agent.get(`/api/audits/${ret.id}`)).body.items;
  assert.equal(items[0].assetId, r.id);
  await s.auditor.agent.post('/api/audits').send({ auditType: 'return' }).expect(409);
  assert.equal((await s.agent.get('/api/audits?auditType=return')).body.items.length, 1);
});

test('frequencies: quarterly and annual automatic audits start once per period', async () => {
  const s = await setup();
  await s.agent.patch('/api/settings').send({ autoAudits: { quarterly: true, annual: true } }).expect(200);
  const started = await runScheduledAudits();
  const mine = (await s.agent.get('/api/audits').expect(200)).body.items;
  assert.deepEqual(mine.map((a) => a.period).sort(), ['annual', 'quarterly']);
  assert.ok(started.length >= 2);
  await runScheduledAudits();
  assert.equal((await s.agent.get('/api/audits')).body.items.length, 2, 'not started twice');
});

test('limited access: audits inside one\'s inventories only', async () => {
  const s = await setup();
  const inside = await s.item(s.available.id, 'Inside');
  const out = await s.item(s.returned.id, 'Out');
  const scoped = await userWithRole(s.agent, 'auditor');
  await s.agent.patch(`/api/users/${scoped.id}`).send({ accessScope: 'inventories', inventoryIds: [s.available.id] }).expect(200);
  await scoped.agent.post('/api/audits').send({ period: 'weekly' }).expect(400);
  await scoped.agent.post('/api/audits').send({ auditType: 'full', inventoryId: s.returned.id }).expect(400);
  const mine = (await scoped.agent.post('/api/audits').send({ auditType: 'full', inventoryId: s.available.id }).expect(201)).body;
  await scoped.agent.post(`/api/audits/${mine.id}/scan`).send({ code: inside.assetTag }).expect(200);
  await scoped.agent.post(`/api/audits/${mine.id}/scan`).send({ code: out.assetTag }).expect(404);
  const other = (await s.agent.post('/api/audits').send({ auditType: 'full', inventoryId: s.returned.id }).expect(201)).body;
  await scoped.agent.get(`/api/audits/${other.id}`).expect(404);
  const listed = (await scoped.agent.get('/api/audits')).body.items.map((a) => a.id);
  assert.deepEqual(listed, [mine.id]);
});
