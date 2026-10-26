import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase } from './helpers.js';
// After helpers, which points the database pool at the test database.
import { parseScannedCode } from '../src/routes/assets.js';

after(() => pool.end());

let agent;
let laptop;
before(async () => {
  await resetDatabase();
  ({ agent } = await newOrg());
  laptop = (await agent.post('/api/assets').send({ name: 'ThinkPad', type: 'Laptop', serial: 'PF-3XK9' }).expect(201)).body;
});

test('scanned codes resolve by asset tag, label URL or serial', async () => {
  const byTag = (await agent.get('/api/assets/lookup?code=dt-1001').expect(200)).body;
  assert.equal(byTag.id, laptop.id);
  assert.equal(byTag.matchedBy, 'assetTag');
  const url = encodeURIComponent('https://devicetrustline.com/?asset=DT-1001');
  assert.equal((await agent.get(`/api/assets/lookup?code=${url}`).expect(200)).body.id, laptop.id);
  const bySerial = (await agent.get('/api/assets/lookup?code=%20pf-3xk9%20').expect(200)).body;
  assert.equal(bySerial.matchedBy, 'serial');
  await agent.get('/api/assets/lookup?code=NOPE-1').expect(404);
  await agent.get('/api/assets/lookup').expect(400);
});

test('lookups never cross organizations', async () => {
  const other = await newOrg();
  await other.agent.get('/api/assets/lookup?code=PF-3XK9').expect(404);
});

test('parseScannedCode', () => {
  assert.equal(parseScannedCode(' DT-7 '), 'DT-7');
  assert.equal(parseScannedCode('https://x.test/path?asset=DT-9&z=1'), 'DT-9');
  assert.equal(parseScannedCode('https://example.com/no-asset'), 'https://example.com/no-asset');
});
