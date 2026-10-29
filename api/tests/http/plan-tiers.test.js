import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { day, newOrg, newPersonal, pool, resetDatabase } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const setPlan = (orgId, code) => pool.query('UPDATE organizations SET plan = $2 WHERE id = $1', [orgId, code]);

test('plans: four tiers, each workspace sees the plans of its kind', async () => {
  const p = await newPersonal();
  const mine = (await p.agent.get('/api/plan').expect(200)).body;
  assert.equal(mine.plan.code, 'free_personal');
  assert.deepEqual(mine.plans.map((x) => x.name), ['Free Personal', 'Personal Pro']);
  assert.equal(mine.plan.features.sms, false);
  assert.equal(mine.billing.configured, false);
  assert.ok('reminders' in mine.usage && 'inventories' in mine.usage);

  const b = await newOrg();
  const biz = (await b.agent.get('/api/plan').expect(200)).body;
  assert.equal(biz.plan.code, 'business');
  assert.deepEqual(biz.plans.map((x) => x.name), ['Business', 'Enterprise']);
  assert.equal(biz.plan.limits.inventories, 100);
  assert.equal(biz.usage.inventories, 6); // Main IT Hub and its five stock rooms
  const enterprise = biz.plans.find((x) => x.code === 'enterprise');
  assert.ok(Object.values(enterprise.limits).every((v) => v === null));

  // Checkout still says payment is not set up rather than pretending.
  const res = await b.agent.post('/api/billing/checkout').send({ plan: 'enterprise' }).expect(503);
  assert.equal(res.body.code, 'billing_unconfigured');
});

test('plans: reminder and inventory limits are enforced on the server', async () => {
  const p = await newPersonal();
  await pool.query(`UPDATE plan_definitions SET limits = limits || '{"reminders": 1}' WHERE code = 'free_personal'`);
  try {
    await p.agent.post('/api/reminders').send({ kind: 'custom', title: 'One', dueOn: day(10) }).expect(201);
    const over = await p.agent.post('/api/reminders').send({ kind: 'custom', title: 'Two', dueOn: day(10) }).expect(402);
    assert.equal(over.body.code, 'plan_limit');
    assert.match(over.body.error, /Free Personal plan includes up to 1 active reminders/);
    // Completed reminders do not count.
    const [r] = (await p.agent.get('/api/reminders')).body.items;
    await p.agent.post(`/api/reminders/${r.id}/complete`).expect(200);
    await p.agent.post('/api/reminders').send({ kind: 'custom', title: 'Two', dueOn: day(10) }).expect(201);
  } finally {
    await pool.query(`UPDATE plan_definitions SET limits = limits || '{"reminders": 50}' WHERE code = 'free_personal'`);
  }

  const b = await newOrg();
  await pool.query(`UPDATE plan_definitions SET limits = limits || '{"inventories": 7}' WHERE code = 'business'`);
  try {
    await b.agent.post('/api/inventories').send({ name: 'Seventh' }).expect(201);
    const over = await b.agent.post('/api/inventories').send({ name: 'Eighth' }).expect(402);
    assert.match(over.body.error, /Business plan includes up to 7 inventories/);
    // Enterprise has no limit.
    await setPlan(b.user.organization.id, 'enterprise');
    await b.agent.post('/api/inventories').send({ name: 'Eighth' }).expect(201);
  } finally {
    await pool.query(`UPDATE plan_definitions SET limits = limits || '{"inventories": 100}' WHERE code = 'business'`);
  }
});

test('plans: text messages and reports are features of the plan', async () => {
  const p = await newPersonal();
  const refused = await p.agent.patch('/api/notification-settings').send({ phone: '+15555550123', sms: true }).expect(402);
  assert.equal(refused.body.code, 'plan_feature');
  assert.match(refused.body.error, /Text message notifications are not part of the Free Personal plan/);
  assert.equal((await p.agent.get('/api/notification-settings')).body.planIncludesSms, false);
  await setPlan(p.user.organization.id, 'personal_pro');
  const s = (await p.agent.patch('/api/notification-settings').send({ phone: '+15555550123', sms: true }).expect(200)).body;
  assert.equal(s.sms, true);
  assert.equal(s.planIncludesSms, true);

  // Moving back to a plan without text messages stops them, and the delivery log says why.
  await setPlan(p.user.organization.id, 'free_personal');
  await p.agent.post('/api/notification-settings/test').expect(200);
  const [n] = (await p.agent.get('/api/notifications')).body.items;
  const sms = n.deliveries.find((d) => d.channel === 'sms');
  assert.equal(sms.status, 'unavailable');
  assert.match(sms.error, /plan does not include text messages/);

  const b = await newOrg();
  await b.agent.get('/api/reports/summary').expect(200);
  await pool.query(`UPDATE plan_definitions SET features = features || '{"reports": false}' WHERE code = 'business'`);
  try {
    const r = await b.agent.get('/api/reports/summary').expect(402);
    assert.equal(r.body.code, 'plan_feature');
  } finally {
    await pool.query(`UPDATE plan_definitions SET features = features || '{"reports": true}' WHERE code = 'business'`);
  }
});
