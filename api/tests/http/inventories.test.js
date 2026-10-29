// Phase 2: hierarchical inventories, transfers, approval and concurrency.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, newPersonal, pool, resetDatabase, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const byName = (items, name) => items.find((i) => i.name === name);

async function setup() {
  const org = await newOrg();
  const manager = await userWithRole(org.agent, 'manager');
  const tech = await userWithRole(org.agent, 'technician');
  const invs = (await org.agent.get('/api/inventories')).body.items;
  return { ...org, manager, tech, hub: byName(invs, 'Main IT Hub'), available: byName(invs, 'Available Inventory'), invs };
}

test('business workspaces start with a hub and its sub-inventories; personal ones do not', async () => {
  const { invs, hub } = await setup();
  assert.deepEqual(invs.map((i) => i.path), [
    'Main IT Hub', 'Main IT Hub / Available Inventory', 'Main IT Hub / Deployment Inventory',
    'Main IT Hub / Loaner Inventory', 'Main IT Hub / RMA / Repair Inventory', 'Main IT Hub / Returned Equipment',
  ]);
  assert.equal(hub.kind, 'hub');
  assert.ok(invs.filter((i) => i.parentId === hub.id).every((i) => i.depth === 1));
  const personal = await newPersonal();
  assert.equal((await personal.agent.get('/api/inventories')).body.items.length, 0);
});

test('workflow 3 (inventory part): a manager creates, renames, nests, archives and restores sub-inventories', async () => {
  const { manager, tech, hub, agent } = await setup();
  const viewer = await userWithRole(agent, 'viewer');
  await viewer.agent.post('/api/inventories').send({ name: 'X' }).expect(403);
  await tech.agent.post('/api/inventories').send({ name: 'X' }).expect(403);
  assert.ok((await viewer.agent.get('/api/inventories')).body.items.length >= 6, 'everyone can see inventories');

  const techInv = (await manager.agent.post('/api/inventories')
    .send({ name: 'Technician A Inventory', kind: 'technician', parentId: hub.id, custodianId: tech.id }).expect(201)).body;
  assert.equal(techInv.path, 'Main IT Hub / Technician A Inventory');
  assert.equal(techInv.custodianName, 'technician');
  await manager.agent.post('/api/inventories').send({ name: 'technician a inventory', parentId: hub.id }).expect(409);
  const site = (await manager.agent.post('/api/inventories').send({ name: 'Remote Site Inventory', kind: 'site' }).expect(201)).body;
  const renamed = (await manager.agent.patch(`/api/inventories/${site.id}`).send({ name: 'Dallas Site', parentId: hub.id }).expect(200)).body;
  assert.equal(renamed.path, 'Main IT Hub / Dallas Site');
  await manager.agent.patch(`/api/inventories/${hub.id}`).send({ parentId: site.id }).expect(400);
  await manager.agent.patch(`/api/inventories/${site.id}`).send({ custodianId: '00000000-0000-4000-8000-000000000000' }).expect(400);

  const item = (await agent.post('/api/assets').send({ name: 'Spare dock', type: 'Docking Station', inventoryId: site.id }).expect(201)).body;
  assert.equal(item.inventoryName, 'Dallas Site');
  await manager.agent.post(`/api/inventories/${hub.id}/archive`).expect(409);
  const full = await manager.agent.post(`/api/inventories/${site.id}/archive`).expect(409);
  assert.match(full.body.error, /Transfer the 1 item/);
  await tech.agent.post('/api/transfers').send({ assetIds: [item.id], toInventoryId: techInv.id }).expect(201);
  const archived = (await manager.agent.post(`/api/inventories/${site.id}/archive`).expect(200)).body;
  assert.ok(archived.archivedAt);
  assert.ok(!(await agent.get('/api/inventories')).body.items.some((i) => i.id === site.id));
  assert.equal((await agent.get('/api/inventories?archived=true')).body.items.length, 1);
  await manager.agent.patch(`/api/inventories/${site.id}`).send({ name: 'Y' }).expect(409);
  await tech.agent.post('/api/transfers').send({ assetIds: [item.id], toInventoryId: site.id }).expect(409);
  await manager.agent.post(`/api/inventories/${site.id}/restore`).expect(200);
});

test('workflow 4: a technician transfers a device (and its components) into a sub-inventory', async () => {
  const { agent, tech, manager, hub, available } = await setup();
  const techInv = (await manager.agent.post('/api/inventories').send({ name: 'Tech A', kind: 'technician', parentId: hub.id, custodianId: tech.id }).expect(201)).body;
  const laptop = (await agent.post('/api/assets').send({ name: 'Latitude', type: 'Laptop', inventoryId: available.id }).expect(201)).body;
  const charger = (await agent.post('/api/assets').send({ name: 'Charger', type: 'Accessory', parentId: laptop.id, inventoryId: available.id }).expect(201)).body;

  const res = (await tech.agent.post('/api/transfers')
    .send({ codes: [laptop.assetTag], toInventoryId: techInv.id, fromInventoryId: available.id, reason: 'Field visit' }).expect(201)).body;
  assert.equal(res.completed.length, 2, 'the charger moves with the laptop');
  assert.equal(res.pending.length, 0);
  const main = res.completed.find((t) => t.assetId === laptop.id);
  assert.deepEqual([main.fromInventoryName, main.toInventoryName, main.method, main.reason, main.requestedBy],
    ['Available Inventory', 'Tech A', 'scan', 'Field visit', 'technician']);
  assert.equal(res.completed.find((t) => t.assetId === charger.id).viaAssetTag, laptop.assetTag);

  const after = (await agent.get(`/api/assets/${laptop.id}`)).body;
  assert.equal(after.inventoryName, 'Tech A');
  assert.equal(after.custodianName, 'technician');
  const events = (await agent.get(`/api/assets/${laptop.id}/events`)).body.items;
  assert.equal(events[0].type, 'transferred');
  assert.equal(events[0].changes.reason.to, 'Field visit');
  assert.equal(events[0].actor.name, 'technician');

  const history = (await agent.get(`/api/transfers?assetId=${laptop.id}`)).body.items;
  assert.equal(history.length, 1);
  const inHub = (await agent.get(`/api/assets?inventoryId=${hub.id}&includeSub=true`)).body;
  assert.equal(inHub.total, 2);
  assert.equal((await agent.get(`/api/assets?inventoryId=${hub.id}`)).body.total, 0);
  assert.equal((await agent.get('/api/assets?inventoryId=none')).body.total, 0);

  // The same move again is refused, and so is a move from where it no longer is.
  await tech.agent.post('/api/transfers').send({ assetIds: [laptop.id], toInventoryId: techInv.id }).expect(409);
  await tech.agent.post('/api/transfers').send({ assetIds: [laptop.id], toInventoryId: hub.id, fromInventoryId: available.id }).expect(409);
  await agent.patch(`/api/assets/${laptop.id}`).send({ inventoryId: hub.id }).expect(400);
  const viewer = await userWithRole(agent, 'viewer');
  await viewer.agent.post('/api/transfers').send({ assetIds: [laptop.id], toInventoryId: hub.id }).expect(403);

  // History is permanent.
  await assert.rejects(pool.query("UPDATE inventory_transfers SET reason = 'x' WHERE id = $1", [main.id]), /cannot change/);
  await assert.rejects(pool.query('DELETE FROM inventory_transfers WHERE id = $1', [main.id]), /append-only/);
});

test('transfers into an approval-required inventory wait for a manager', async () => {
  const { agent, tech, manager, hub, available } = await setup();
  const secure = (await manager.agent.post('/api/inventories').send({ name: 'Secure Cage', parentId: hub.id, requiresApproval: true }).expect(201)).body;
  const a = (await agent.post('/api/assets').send({ name: 'Server A', type: 'Desktop', inventoryId: available.id }).expect(201)).body;
  const b = (await agent.post('/api/assets').send({ name: 'Server B', type: 'Desktop', inventoryId: available.id }).expect(201)).body;

  const req = (await tech.agent.post('/api/transfers').send({ assetIds: [a.id], toInventoryId: secure.id, reason: 'Decommission' }).expect(201)).body;
  assert.equal(req.pending.length, 1);
  const pending = req.pending[0];
  assert.equal((await agent.get(`/api/assets/${a.id}`)).body.inventoryName, 'Available Inventory', 'nothing moves yet');
  const dup = await tech.agent.post('/api/transfers').send({ assetIds: [a.id], toInventoryId: hub.id }).expect(409);
  assert.match(dup.body.error, /waiting for approval/);

  const managerNote = (await manager.agent.get('/api/notifications')).body.items.find((n) => n.kind === 'transfer_requested');
  assert.ok(managerNote, 'approvers are told');
  assert.match(managerNote.body, /technician asked to move Server A/);
  assert.equal((await tech.agent.get('/api/notifications')).body.items.length, 0);

  await tech.agent.post(`/api/transfers/${pending.id}/approve`).expect(403);
  const approved = (await manager.agent.post(`/api/transfers/${pending.id}/approve`).send({ note: 'ok' }).expect(200)).body;
  assert.deepEqual([approved.status, approved.decidedBy, approved.decisionNote], ['completed', 'manager', 'ok']);
  assert.equal((await agent.get(`/api/assets/${a.id}`)).body.inventoryName, 'Secure Cage');
  await manager.agent.post(`/api/transfers/${pending.id}/approve`).expect(409);
  const told = (await tech.agent.get('/api/notifications')).body.items[0];
  assert.equal(told.kind, 'transfer_completed');

  // Reject needs a reason; cancel is for the person who asked or a manager.
  const r2 = (await tech.agent.post('/api/transfers').send({ assetIds: [b.id], toInventoryId: secure.id }).expect(201)).body.pending[0];
  await manager.agent.post(`/api/transfers/${r2.id}/reject`).send({}).expect(400);
  const rejected = (await manager.agent.post(`/api/transfers/${r2.id}/reject`).send({ reason: 'Not this one' }).expect(200)).body;
  assert.equal(rejected.status, 'rejected');
  assert.equal((await tech.agent.get('/api/notifications')).body.items[0].kind, 'transfer_rejected');
  const r3 = (await tech.agent.post('/api/transfers').send({ assetIds: [b.id], toInventoryId: secure.id }).expect(201)).body.pending[0];
  const other = await userWithRole(agent, 'technician');
  await other.agent.post(`/api/transfers/${r3.id}/cancel`).expect(403);
  assert.equal((await tech.agent.post(`/api/transfers/${r3.id}/cancel`).expect(200)).body.status, 'cancelled');

  // Moving out of it needs approval too, unless a manager does it.
  const out = (await tech.agent.post('/api/transfers').send({ assetIds: [a.id], toInventoryId: available.id }).expect(201)).body;
  assert.equal(out.pending.length, 1);
  await manager.agent.post(`/api/transfers/${out.pending[0].id}/cancel`).expect(200);
  const direct = (await manager.agent.post('/api/transfers').send({ assetIds: [a.id], toInventoryId: available.id }).expect(201)).body;
  assert.equal(direct.completed.length, 1);

  // A request approved after the item moved some other way is refused.
  const r4 = (await tech.agent.post('/api/transfers').send({ assetIds: [b.id], toInventoryId: secure.id }).expect(201)).body.pending[0];
  await pool.query('UPDATE assets SET inventory_id = $1 WHERE id = $2', [hub.id, b.id]);
  await manager.agent.post(`/api/transfers/${r4.id}/approve`).expect(409);
  assert.equal((await agent.get('/api/transfers?status=pending')).body.items.length, 1);
});

test('bulk transfers report what was skipped and why', async () => {
  const { agent, tech, hub, available } = await setup();
  const ids = [];
  for (const n of [1, 2, 3]) ids.push((await agent.post('/api/assets').send({ name: `Monitor ${n}`, type: 'Monitor', inventoryId: available.id }).expect(201)).body.id);
  const retired = (await agent.post('/api/assets').send({ name: 'Old', type: 'Monitor', status: 'Retired', inventoryId: available.id }).expect(201)).body;
  await tech.agent.post('/api/transfers').send({ assetIds: [ids[0]], toInventoryId: hub.id }).expect(201);
  const res = (await tech.agent.post('/api/transfers').send({ assetIds: [...ids, retired.id], toInventoryId: hub.id }).expect(201)).body;
  assert.equal(res.completed.length, 2);
  assert.deepEqual(res.skipped.map((s) => s.reason).sort(), ['Already in that inventory', 'Archived and retired items are not moved']);
  assert.ok(res.completed.every((t) => t.method === 'bulk' && t.batchId === res.batchId));
  await tech.agent.post('/api/transfers').send({ assetIds: [], toInventoryId: hub.id }).expect(400);
});

test('workflow 14 (transfers): concurrent moves cannot double-transfer an item', async () => {
  const { agent, tech, manager, hub, available, invs } = await setup();
  const item = (await agent.post('/api/assets').send({ name: 'Contested', type: 'Laptop', inventoryId: available.id }).expect(201)).body;
  const targets = [hub, ...invs.filter((i) => i.id !== available.id && i.id !== hub.id)];
  const results = await Promise.all(targets.map((t) => tech.agent.post('/api/transfers')
    .send({ assetIds: [item.id], toInventoryId: t.id, fromInventoryId: available.id })));
  assert.equal(results.filter((r) => r.status === 201).length, 1, 'exactly one move wins');
  assert.ok(results.filter((r) => r.status !== 201).every((r) => r.status === 409));
  const { rows } = await pool.query("SELECT count(*)::int AS n FROM inventory_transfers WHERE asset_id = $1 AND status = 'completed'", [item.id]);
  assert.equal(rows[0].n, 1);

  // Concurrent approval requests: one waits, the rest are refused.
  const cage = (await manager.agent.post('/api/inventories').send({ name: 'Cage', requiresApproval: true }).expect(201)).body;
  const cage2 = (await manager.agent.post('/api/inventories').send({ name: 'Cage 2', requiresApproval: true }).expect(201)).body;
  const reqs = await Promise.all([cage, cage2, cage, cage2].map((c) => tech.agent.post('/api/transfers').send({ assetIds: [item.id], toInventoryId: c.id })));
  assert.equal(reqs.filter((r) => r.status === 201).length, 1);
  const pending = await pool.query("SELECT count(*)::int AS n FROM inventory_transfers WHERE asset_id = $1 AND status = 'pending'", [item.id]);
  assert.equal(pending.rows[0].n, 1);
});

test('inventories and transfers stay inside their workspace', async () => {
  const a = await setup();
  const b = await setup();
  const item = (await a.agent.post('/api/assets').send({ name: 'Mine', type: 'Laptop', inventoryId: a.available.id }).expect(201)).body;
  await b.agent.get(`/api/inventories/${a.hub.id}`).expect(404);
  await b.agent.patch(`/api/inventories/${a.hub.id}`).send({ name: 'Taken' }).expect(404);
  await b.agent.post('/api/transfers').send({ assetIds: [item.id], toInventoryId: b.hub.id }).expect(404);
  await a.agent.post('/api/transfers').send({ assetIds: [item.id], toInventoryId: b.hub.id }).expect(400);
  await b.agent.post('/api/assets').send({ name: 'X', type: 'Laptop', inventoryId: a.hub.id }).expect(400);
  const t = (await a.agent.post('/api/transfers').send({ assetIds: [item.id], toInventoryId: a.hub.id }).expect(201)).body.completed[0];
  await b.agent.get(`/api/transfers/${t.id}`).expect(404);
  assert.equal((await b.agent.get('/api/transfers')).body.items.length, 0);
});
