import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import request from 'supertest';
import { app, newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const JPEG = Buffer.concat([Buffer.from([0xff, 0xd8, 0xff, 0xe0]), Buffer.alloc(200, 1)]);
const PDF = Buffer.from('%PDF-1.7\n1 0 obj\n<<>>\nendobj\n');

test('new workspaces get starting categories they can rename, add and remove', async () => {
  const personal = request.agent(app);
  await personal.post('/api/auth/signup').send({ accountType: 'personal', name: 'P', email: `p${Date.now()}@example.com`, password: 'correct horse battery staple' }).expect(201);
  const names = (await personal.get('/api/categories')).body.items.map((c) => c.name);
  assert.ok(names.includes('Appliances'));
  assert.ok(!names.includes('Scanner'));

  const { agent } = await newOrg();
  const cats = (await agent.get('/api/categories')).body.items;
  const laptop = cats.find((c) => c.name === 'Laptop');
  await agent.post('/api/assets').send({ name: 'ThinkPad', type: 'laptop' }).expect(201);
  const renamed = await agent.patch(`/api/categories/${laptop.id}`).send({ name: 'Notebook' }).expect(200);
  assert.equal(renamed.body.name, 'Notebook');
  const items = (await agent.get('/api/assets')).body.items;
  assert.equal(items[0].type, 'Notebook', 'items follow a renamed category');
  await agent.delete(`/api/categories/${laptop.id}`).expect(409);
  const drone = (await agent.post('/api/categories').send({ name: 'Drone' }).expect(201)).body;
  await agent.post('/api/categories').send({ name: 'drone' }).expect(409);
  await agent.post('/api/assets').send({ name: 'Mavic', type: 'Drone' }).expect(201);
  const bad = await agent.post('/api/assets').send({ name: 'X', type: 'Toaster' }).expect(400);
  assert.match(bad.body.details[0].message, /not a category/);
  const other = await newOrg();
  await other.agent.patch(`/api/categories/${drone.id}`).send({ name: 'Mine' }).expect(404);
});

test('items keep purchase, ownership, condition and date details', async () => {
  const { agent, user } = await newOrg();
  const res = await agent.post('/api/assets').send({
    name: 'Espresso machine', type: 'Other', assetTag: 'KITCHEN-1', manufacturer: 'Breville', model: 'BES870',
    condition: 'Good', ownerId: user.id, purchasePrice: '599.99', currency: 'usd', seller: 'Costco',
    purchaseDate: '2026-01-10', returnDeadline: '2026-02-09', maintenanceDue: '2026-12-01', warrantyExpires: '2027-01-10',
  }).expect(201);
  const item = res.body;
  assert.equal(item.assetTag, 'KITCHEN-1');
  assert.equal(item.ownerName, 'Admin');
  assert.equal(Number(item.purchasePrice), 599.99);
  assert.equal(item.currency, 'USD');
  assert.equal(item.condition, 'Good');
  assert.equal(item.warrantyExpires, '2027-01-10');
  assert.equal(item.warrantyStatus, 'active');
  await agent.post('/api/assets').send({ name: 'Dup', type: 'Other', assetTag: 'KITCHEN-1' }).expect(409);
  await agent.post('/api/assets').send({ name: 'X', type: 'Other', condition: 'Shiny' }).expect(400);
  await agent.post('/api/assets').send({ name: 'X', type: 'Other', currency: 'dollars' }).expect(400);
  const other = await newOrg();
  await agent.post('/api/assets').send({ name: 'X', type: 'Other', ownerId: other.user.id }).expect(400);

  const updated = await agent.patch(`/api/assets/${item.id}`).send({ condition: 'Fair', purchasePrice: null }).expect(200);
  assert.equal(updated.body.condition, 'Fair');
  assert.equal(updated.body.purchasePrice, null);
  const events = (await agent.get(`/api/assets/${item.id}/events`)).body.items;
  assert.deepEqual(events[0].changes.condition, { from: 'Good', to: 'Fair' });
});

test('tags are created as used, filter items, and can be renamed or removed', async () => {
  const { agent } = await newOrg();
  const a = (await agent.post('/api/assets').send({ name: 'Drill', type: 'Other', tags: ['garage', 'Power tools', 'garage'] }).expect(201)).body;
  assert.deepEqual(a.tags, ['garage', 'Power tools']);
  await agent.post('/api/assets').send({ name: 'Saw', type: 'Other', tags: ['Garage'] }).expect(201);
  const tagged = (await agent.get('/api/assets?tag=GARAGE')).body;
  assert.equal(tagged.total, 2);
  const changed = await agent.patch(`/api/assets/${a.id}`).send({ tags: ['shed'] }).expect(200);
  assert.deepEqual(changed.body.tags, ['shed']);
  const tags = (await agent.get('/api/tags')).body.items;
  const shed = tags.find((t) => t.name === 'shed');
  await agent.patch(`/api/tags/${shed.id}`).send({ name: 'Shed' }).expect(200);
  await agent.delete(`/api/tags/${shed.id}`).expect(204);
  assert.deepEqual((await agent.get(`/api/assets/${a.id}`)).body.tags, []);
  await agent.post('/api/assets').send({ name: 'X', type: 'Other', tags: ['x'.repeat(41)] }).expect(400);
});

test('custom fields are defined per workspace and checked by type', async () => {
  const { agent } = await newOrg();
  const color = (await agent.post('/api/custom-fields').send({ label: 'Color', type: 'select', options: ['Red', 'Blue'] }).expect(201)).body;
  const ram = (await agent.post('/api/custom-fields').send({ label: 'RAM (GB)', type: 'number' }).expect(201)).body;
  await agent.post('/api/custom-fields').send({ label: 'Bad', type: 'select' }).expect(400);
  const item = (await agent.post('/api/assets').send({ name: 'Bike', type: 'Other', custom: { [color.id]: 'Red', [ram.id]: 16 } }).expect(201)).body;
  assert.equal(item.custom[color.id], 'Red');
  await agent.patch(`/api/assets/${item.id}`).send({ custom: { [color.id]: 'Green' } }).expect(400);
  await agent.patch(`/api/assets/${item.id}`).send({ custom: { [ram.id]: 'lots' } }).expect(400);
  await agent.patch(`/api/assets/${item.id}`).send({ custom: { 'not-a-field': 1 } }).expect(400);
  const cleared = await agent.patch(`/api/assets/${item.id}`).send({ custom: { [ram.id]: null } }).expect(200);
  assert.deepEqual(cleared.body.custom, { [color.id]: 'Red' });
  await agent.delete(`/api/custom-fields/${color.id}`).expect(204);
  assert.deepEqual((await agent.get(`/api/assets/${item.id}`)).body.custom, {});

  const other = await newOrg();
  await other.agent.post('/api/assets').send({ name: 'X', type: 'Other', custom: { [ram.id]: 1 } }).expect(400);
  const viewer = await userWithRole(agent, 'viewer');
  await viewer.agent.post('/api/custom-fields').send({ label: 'Nope', type: 'text' }).expect(403);
});

test('attachments are private to the workspace and checked by content', async () => {
  const { agent } = await newOrg();
  const item = (await agent.post('/api/assets').send({ name: 'TV', type: 'Monitor' }).expect(201)).body;
  const photo = (await agent.post(`/api/assets/${item.id}/attachments`).field('kind', 'photo').attach('file', JPEG, 'tv.jpg').expect(201)).body;
  assert.equal(photo.contentType, 'image/jpeg');
  const receipt = (await agent.post(`/api/assets/${item.id}/attachments`).field('kind', 'receipt').attach('file', PDF, '../../receipt.pdf').expect(201)).body;
  assert.equal(receipt.filename, 'receipt.pdf');
  // An HTML file renamed to .jpg is refused.
  await agent.post(`/api/assets/${item.id}/attachments`).attach('file', Buffer.from('<script>alert(1)</script>'), 'x.jpg').expect(415);
  await agent.post(`/api/assets/${item.id}/attachments`).field('kind', 'weird').attach('file', JPEG, 'a.jpg').expect(400);

  const file = await agent.get(`/api/attachments/${photo.id}`).expect(200);
  assert.equal(file.headers['content-type'], 'image/jpeg');
  assert.equal(file.headers['x-content-type-options'], 'nosniff');
  assert.equal(Buffer.compare(file.body, JPEG), 0);
  const fresh = (await agent.get(`/api/assets/${item.id}`)).body;
  assert.equal(fresh.attachmentCount, 2);
  assert.equal(fresh.photoId, photo.id);

  const other = await newOrg();
  await other.agent.get(`/api/attachments/${photo.id}`).expect(404);
  await other.agent.delete(`/api/attachments/${photo.id}`).expect(404);
  await other.agent.post(`/api/assets/${item.id}/attachments`).attach('file', JPEG, 'a.jpg').expect(404);
  await request(app).get(`/api/attachments/${photo.id}`).expect(401);
  const viewer = await userWithRole(agent, 'viewer');
  await viewer.agent.get(`/api/attachments/${photo.id}`).expect(200);
  await viewer.agent.delete(`/api/attachments/${photo.id}`).expect(403);

  await agent.delete(`/api/attachments/${photo.id}`).expect(204);
  await agent.get(`/api/attachments/${photo.id}`).expect(404);
  const events = (await agent.get(`/api/assets/${item.id}/events`)).body.items.map((e) => e.type);
  assert.ok(events.includes('attachment_added') && events.includes('attachment_removed'));
});

test('uploads over the size limit are refused', async () => {
  const { agent } = await newOrg();
  const item = (await agent.post('/api/assets').send({ name: 'Big', type: 'Other' }).expect(201)).body;
  const big = Buffer.concat([JPEG, Buffer.alloc(11 * 1024 * 1024)]);
  const res = await agent.post(`/api/assets/${item.id}/attachments`).attach('file', big, 'big.jpg');
  assert.equal(res.status, 413);
});
