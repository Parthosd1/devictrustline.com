// Phase 1: personal device records and the user-managed notification system.
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import http from 'node:http';
import { after, before, test } from 'node:test';
import { day, newOrg, newPersonal, pool, resetDatabase, runJobs } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const PNG = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(100, 7)]);
const PDF = Buffer.from('%PDF-1.7\n1 0 obj\n<<>>\nendobj\n');

// Reminders go out at the send hour; hour 0 means "already due today" whatever the time now.
const deliverNow = (agent) => agent.patch('/api/notification-settings').send({ sendHour: 0 }).expect(200);

const jobs = async (reminderId) => (await pool.query(
  'SELECT dedupe_key, days_before, status, fire_at FROM reminder_jobs WHERE reminder_id = $1 ORDER BY id', [reminderId])).rows;

test('workflow 1: a personal user registers a device and schedules a warranty reminder', async () => {
  const { agent, email } = await newPersonal();
  const cats = (await agent.get('/api/categories')).body.items.map((c) => c.name);
  for (const c of ['Phones & Tablets', 'TVs & Monitors', 'Gaming', 'Routers & Networking', 'Vehicles', 'Appliances']) {
    assert.ok(cats.includes(c), `personal workspaces offer ${c}`);
  }
  const phone = (await agent.post('/api/assets').send({
    name: 'Pixel 9', type: 'Phones & Tablets', manufacturer: 'Google', model: 'Pixel 9', serial: 'PX9-001',
    purchaseDate: day(-335), purchasePrice: '799.00', currency: 'USD', seller: 'Best Buy', condition: 'Good',
    returnDeadline: day(-320), warrantyExpires: day(30), notes: 'Blue case', tags: ['phone', 'daily'],
  }).expect(201)).body;
  await agent.post(`/api/assets/${phone.id}/attachments`).field('kind', 'receipt').attach('file', PDF, 'receipt.pdf').expect(201);
  await agent.post(`/api/assets/${phone.id}/attachments`).field('kind', 'photo').attach('file', PNG, 'phone.png').expect(201);
  await agent.post(`/api/assets/${phone.id}/warranties`).send({ kind: 'extended', provider: 'Asurion', startsOn: day(30), endsOn: day(365) }).expect(201);

  await deliverNow(agent);
  const reminder = (await agent.post('/api/reminders').send({
    kind: 'warranty', assetId: phone.id, dateSource: 'warranty', offsets: [365, 30, 7, 1],
  }).expect(201)).body;
  assert.equal(reminder.title, 'Pixel 9: warranty ends');
  assert.equal(reminder.dueOn, day(365), 'follows the latest warranty end date');
  assert.deepEqual(reminder.offsets, [365, 30, 7, 1]);

  const run = await runJobs();
  assert.ok(run.reminderJobsCreated >= 4);
  assert.equal(run.reminderJobsFired, 1, 'the 365-days-before reminder is due today');
  const again = await runJobs();
  assert.equal(again.reminderJobsCreated, 0);
  assert.equal(again.reminderJobsFired, 0, 'nothing is delivered twice');

  const notes = (await agent.get('/api/notifications')).body.items.filter((n) => n.assetId === phone.id);
  assert.equal(notes.length, 1, 'only the reminder the person set, not the workspace default as well');
  assert.equal(notes[0].reminderId, reminder.id);
  assert.match(notes[0].body, /due in 365 days/);
  assert.equal(notes[0].link, `/?asset=${phone.assetTag}`);
  assert.deepEqual(notes[0].deliveries.map((d) => [d.channel, d.status]), [['email', 'sent']]);
  const { rows: [mail] } = await pool.query('SELECT kind, subject FROM email_outbox WHERE to_address = $1 ORDER BY id DESC LIMIT 1', [email]);
  assert.deepEqual(mail, { kind: 'reminder', subject: 'Pixel 9: warranty ends' });

  const detail = (await agent.get(`/api/reminders/${reminder.id}`)).body;
  assert.equal(detail.history.filter((h) => h.status === 'sent').length, 1);
  assert.ok(detail.history.some((h) => h.status === 'pending' && h.daysBefore === 30));
  assert.ok(detail.nextNotificationAt);
  assert.ok(detail.lastNotifiedAt);

  // When the item's date changes, the reminder follows it and reschedules.
  await agent.patch(`/api/assets/${phone.id}`).send({ warrantyExpires: day(60) }).expect(200);
  const ext = (await agent.get(`/api/assets/${phone.id}/warranties`)).body.items.find((w) => w.kind === 'extended');
  await agent.delete(`/api/warranties/${ext.id}`).expect(204);
  await runJobs();
  assert.equal((await agent.get(`/api/reminders/${reminder.id}`)).body.dueOn, day(60));
  const rows = await jobs(reminder.id);
  assert.ok(rows.filter((j) => j.dedupe_key.includes(day(365))).every((j) => j.status !== 'pending'), 'old jobs cancelled');
  assert.ok(rows.some((j) => j.dedupe_key.includes(day(60)) && j.status === 'pending'));
});

test('workflow 2: a person edits, snoozes, completes and disables reminders', async () => {
  const { agent } = await newPersonal();
  await deliverNow(agent);
  const r = (await agent.post('/api/reminders').send({ kind: 'custom', title: 'Renew car insurance', dueOn: day(20), offsets: [14, 3] }).expect(201)).body;
  await runJobs();
  assert.deepEqual((await jobs(r.id)).map((j) => [j.days_before, j.status]), [[14, 'pending'], [3, 'pending']]);

  // Edit: new date and timing. Old jobs are cancelled, new ones scheduled, including "same day".
  const edited = (await agent.patch(`/api/reminders/${r.id}`).send({ dueOn: day(10), offsets: [10, 0], notes: 'Policy 123' }).expect(200)).body;
  assert.equal(edited.dueOn, day(10));
  await runJobs();
  let rows = await jobs(r.id);
  assert.ok(rows.filter((j) => j.dedupe_key.includes(day(20))).every((j) => j.status === 'cancelled'));
  const fired = rows.find((j) => j.dedupe_key.endsWith(`${day(10)}:10`));
  assert.equal(fired.status, 'sent', 'the 10-days-before reminder was due today');
  const note = (await agent.get('/api/notifications')).body.items.find((n) => n.reminderId === r.id);
  assert.match(note.body, /Renew car insurance is due in 10 days .* Policy 123/);

  // Snooze for two hours: one notification then, nothing before.
  const snoozed = (await agent.post(`/api/reminders/${r.id}/snooze`).send({ hours: 2 }).expect(200)).body;
  assert.ok(new Date(snoozed.snoozedUntil) > new Date(Date.now() + 3600000));
  await runJobs();
  rows = await jobs(r.id);
  const snoozeJob = rows.find((j) => j.dedupe_key.includes(':snooze:'));
  assert.equal(snoozeJob.status, 'pending');
  assert.ok(Math.abs(new Date(snoozeJob.fire_at) - new Date(snoozed.snoozedUntil)) < 1000);
  await agent.post(`/api/reminders/${r.id}/snooze`).send({ until: '2000-01-01T00:00:00Z' }).expect(400);

  // Pause: everything pending is cancelled. Resume: scheduled again.
  const paused = (await agent.patch(`/api/reminders/${r.id}`).send({ status: 'paused' }).expect(200)).body;
  assert.equal(paused.status, 'paused');
  assert.equal(paused.snoozedUntil, null);
  await runJobs();
  assert.ok((await jobs(r.id)).every((j) => j.status !== 'pending'));
  await agent.post(`/api/reminders/${r.id}/snooze`).send({ hours: 1 }).expect(409);
  await agent.patch(`/api/reminders/${r.id}`).send({ status: 'active' }).expect(200);
  await runJobs();
  assert.ok((await jobs(r.id)).some((j) => j.status === 'pending' && j.days_before === 0));

  // Complete: done, no more notifications. It drops out of the default list.
  const done = (await agent.post(`/api/reminders/${r.id}/complete`).expect(200)).body;
  assert.equal(done.status, 'completed');
  await runJobs();
  assert.ok((await jobs(r.id)).every((j) => j.status !== 'pending'));
  assert.equal((await agent.get('/api/reminders')).body.items.length, 0);
  assert.equal((await agent.get('/api/reminders?status=completed')).body.items.length, 1);
  await agent.patch(`/api/reminders/${r.id}`).send({ dueOn: day(5) }).expect(409);

  // Turning notifications off entirely: due jobs are skipped, not delivered.
  const other = (await agent.post('/api/reminders').send({ kind: 'inspection', title: 'Smoke alarm test', dueOn: day(0), offsets: [0] }).expect(201)).body;
  const settings = (await agent.patch('/api/notification-settings').send({ enabled: false }).expect(200)).body;
  assert.equal(settings.enabled, false);
  await runJobs();
  assert.equal((await jobs(other.id))[0].status, 'skipped');
  assert.ok(!(await agent.get('/api/notifications')).body.items.some((n) => n.reminderId === other.id));

  await agent.delete(`/api/reminders/${other.id}`).expect(204);
  await agent.get(`/api/reminders/${other.id}`).expect(404);
  assert.equal((await jobs(other.id)).length, 0);
});

test('recurring reminders roll forward, and completing one moves to the next date', async () => {
  const { agent } = await newPersonal();
  const battery = (await agent.post('/api/reminders').send({
    kind: 'maintenance', title: 'Replace smoke alarm battery', dueOn: day(-3), offsets: [7, 0], repeat: 'daily', repeatEvery: 10,
  }).expect(201)).body;
  await runJobs();
  assert.equal((await agent.get(`/api/reminders/${battery.id}`)).body.dueOn, day(7), 'moved to the next repetition');
  const next = (await agent.post(`/api/reminders/${battery.id}/complete`).expect(200)).body;
  assert.equal(next.status, 'active');
  assert.equal(next.dueOn, day(17));
  const ending = (await agent.post('/api/reminders').send({
    kind: 'renewal', title: 'Phone plan', dueOn: day(-1), offsets: [1], repeat: 'monthly', repeatUntil: day(5),
  }).expect(201)).body;
  await runJobs();
  assert.equal((await agent.get(`/api/reminders/${ending.id}`)).body.status, 'completed', 'past its last repetition');
  await agent.post('/api/reminders').send({ kind: 'warranty', assetId: battery.id, dateSource: 'warranty', offsets: [1] }).expect(400);
});

test('reminder input is validated and each person sees only their own', async () => {
  const { agent } = await newPersonal();
  const item = (await agent.post('/api/assets').send({ name: 'Router', type: 'Routers & Networking' }).expect(201)).body;
  await agent.post('/api/reminders').send({ kind: 'custom', dueOn: day(3) }).expect(400);
  await agent.post('/api/reminders').send({ kind: 'nope', dueOn: day(3) }).expect(400);
  await agent.post('/api/reminders').send({ kind: 'renewal', dueOn: day(3), offsets: [] }).expect(400);
  await agent.post('/api/reminders').send({ kind: 'renewal', dueOn: day(3), offsets: [400] }).expect(400);
  await agent.post('/api/reminders').send({ kind: 'renewal', dueOn: '03/04/2026' }).expect(400);
  const noDate = await agent.post('/api/reminders').send({ kind: 'warranty', assetId: item.id, dateSource: 'warranty' }).expect(400);
  assert.match(noDate.body.error, /no date/);
  // Default timing comes from the person's settings.
  await agent.patch('/api/notification-settings').send({ defaultOffsets: [60, 14, 0] }).expect(200);
  const r = (await agent.post('/api/reminders').send({ kind: 'renewal', assetId: item.id, dueOn: day(90) }).expect(201)).body;
  assert.deepEqual(r.offsets, [60, 14, 0]);
  assert.equal(r.title, 'Router: subscription or protection plan renews');

  const stranger = await newPersonal('Sam');
  await stranger.agent.get(`/api/reminders/${r.id}`).expect(404);
  await stranger.agent.patch(`/api/reminders/${r.id}`).send({ title: 'Mine' }).expect(404);
  await stranger.agent.post(`/api/reminders/${r.id}/complete`).expect(404);
  await stranger.agent.delete(`/api/reminders/${r.id}`).expect(404);
  await stranger.agent.post('/api/reminders').send({ kind: 'renewal', assetId: item.id, dueOn: day(4) }).expect(400);
  assert.equal((await stranger.agent.get('/api/reminders')).body.items.length, 0);
});

test('notification settings: channels, time zone and quiet hours', async () => {
  const { agent } = await newPersonal();
  const s = (await agent.get('/api/notification-settings')).body;
  assert.deepEqual([s.enabled, s.inApp, s.email, s.push, s.sms], [true, true, true, false, false]);
  assert.deepEqual(s.available, { email: true, push: true, sms: true });
  assert.ok(s.pushPublicKey);
  await agent.patch('/api/notification-settings').send({ timezone: 'Mars/Olympus' }).expect(400);
  await agent.patch('/api/notification-settings').send({ quietStart: '22:00' }).expect(400);
  await agent.patch('/api/notification-settings').send({ quietStart: '25:00', quietEnd: '07:00' }).expect(400);
  await agent.patch('/api/notification-settings').send({ sms: true }).expect(400);
  await agent.patch('/api/notification-settings').send({ phone: '555-1234' }).expect(400);

  // Quiet hours around the current moment hold email back until they end; in-app is immediate.
  const { rows: [{ now }] } = await pool.query("SELECT to_char(now() AT TIME ZONE 'UTC', 'HH24:MI') AS now");
  const [h] = now.split(':').map(Number);
  const pad = (n) => String((n + 24) % 24).padStart(2, '0');
  const set = (await agent.patch('/api/notification-settings')
    .send({ timezone: 'UTC', quietStart: `${pad(h - 1)}:00`, quietEnd: `${pad(h + 2)}:00`, sendHour: 0 }).expect(200)).body;
  assert.equal(set.timezone, 'UTC');
  assert.equal(set.quietEnd, `${pad(h + 2)}:00`);
  await agent.post('/api/reminders').send({ kind: 'custom', title: 'Quiet test', dueOn: day(0), offsets: [0] }).expect(201);
  await runJobs();
  const note = (await agent.get('/api/notifications')).body.items.find((n) => n.title === 'Quiet test');
  assert.ok(note, 'in-app notification is there at once');
  const email = note.deliveries.find((d) => d.channel === 'email');
  assert.equal(email.status, 'queued');
  assert.ok(new Date(email.nextAttemptAt) > new Date(), 'email waits for quiet hours to end');

  const cleared = (await agent.patch('/api/notification-settings').send({ quietStart: null, quietEnd: null, email: false }).expect(200)).body;
  assert.equal(cleared.quietStart, null);
  assert.equal(cleared.email, false);
  assert.equal((await agent.get('/api/account')).body.emailNotifications, false, 'the same switch as account settings');
});

// A stand-in push service that decrypts what it receives, as the browser would.
function pushService(browser, auth) {
  const received = [];
  const server = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const body = Buffer.concat(chunks);
      if (req.url.includes('gone')) { res.writeHead(410).end(); return; }
      const salt = body.subarray(0, 16);
      const idlen = body[20];
      const keyid = body.subarray(21, 21 + idlen);
      const data = body.subarray(21 + idlen);
      const shared = browser.computeSecret(keyid);
      const info = Buffer.concat([Buffer.from('WebPush: info\0'), browser.getPublicKey(), keyid]);
      const ikm = Buffer.from(crypto.hkdfSync('sha256', shared, auth, info, 32));
      const cek = Buffer.from(crypto.hkdfSync('sha256', ikm, salt, Buffer.from('Content-Encoding: aes128gcm\0'), 16));
      const nonce = Buffer.from(crypto.hkdfSync('sha256', ikm, salt, Buffer.from('Content-Encoding: nonce\0'), 12));
      const decipher = crypto.createDecipheriv('aes-128-gcm', cek, nonce);
      decipher.setAuthTag(data.subarray(data.length - 16));
      const plain = Buffer.concat([decipher.update(data.subarray(0, data.length - 16)), decipher.final()]);
      received.push({ headers: req.headers, payload: JSON.parse(plain.subarray(0, plain.length - 1).toString()) });
      res.writeHead(201, { location: 'https://push.test/msg/1' }).end();
    });
  });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve({ server, received, port: server.address().port })));
}

test('browser push and text messages are delivered, logged and retried', async () => {
  const { agent, user } = await newPersonal();
  // Text messages are part of Personal Pro, not Free Personal.
  await pool.query("UPDATE organizations SET plan = 'personal_pro' WHERE id = $1", [user.organization.id]);
  const browser = crypto.createECDH('prime256v1');
  browser.generateKeys();
  const auth = crypto.randomBytes(16);
  const svc = await pushService(browser, auth);
  try {
    const keys = { p256dh: browser.getPublicKey().toString('base64url'), auth: auth.toString('base64url') };
    await agent.post('/api/push/subscriptions').send({ endpoint: 'ftp://x', keys }).expect(400);
    await agent.post('/api/push/subscriptions').send({ endpoint: `http://127.0.0.1:${svc.port}/send/abc`, keys: { ...keys, p256dh: 'bad' } }).expect(400);
    await agent.post('/api/push/subscriptions').send({ endpoint: `http://127.0.0.1:${svc.port}/send/abc`, keys, expirationTime: null }).expect(201);
    await agent.post('/api/push/subscriptions').send({ endpoint: `http://127.0.0.1:${svc.port}/gone/def`, keys }).expect(201);
    const s = (await agent.patch('/api/notification-settings').send({ phone: '+15555550123', sms: true, sendHour: 0 }).expect(200)).body;
    assert.deepEqual([s.push, s.sms, s.pushSubscriptions], [true, true, 2]);

    await agent.post('/api/reminders').send({ kind: 'loan_return', title: 'Get the drill back from Sam', dueOn: day(1), offsets: [1] }).expect(201);
    const run = await runJobs();
    assert.equal(run.reminderJobsFired, 1);
    assert.equal(svc.received.length, 1);
    const [msg] = svc.received;
    assert.match(msg.headers.authorization, /^vapid t=[\w-]+\.[\w-]+\.[\w-]+, k=[\w-]+$/);
    assert.equal(msg.headers['content-encoding'], 'aes128gcm');
    assert.equal(msg.payload.title, 'Get the drill back from Sam');
    assert.match(msg.payload.body, /due tomorrow/);

    const note = (await agent.get('/api/notifications')).body.items[0];
    const by = Object.fromEntries(note.deliveries.map((d) => [`${d.channel}${d.error ? '-err' : ''}`, d.status]));
    assert.deepEqual(by, { email: 'sent', push: 'sent', 'push-err': 'failed', sms: 'sent' });
    const { rows } = await pool.query('SELECT endpoint FROM push_subscriptions WHERE endpoint LIKE $1', ['%gone%']);
    assert.equal(rows.length, 0, 'an expired browser subscription is removed');

    // A rejected text message is retried with backoff, not lost and not duplicated.
    await agent.patch('/api/notification-settings').send({ phone: '+15555550000' }).expect(200);
    await agent.post('/api/notification-settings/test').expect(200);
    await runJobs();
    const { rows: [sms] } = await pool.query(
      "SELECT status, attempts, last_error, next_attempt_at > now() AS later FROM notification_deliveries WHERE channel = 'sms' ORDER BY id DESC LIMIT 1");
    assert.deepEqual([sms.status, sms.attempts, sms.later], ['queued', 1, true]);
    assert.match(sms.last_error, /simulated rejection/);

    const del = await agent.delete('/api/push/subscriptions').send({});
    assert.equal(del.status, 204, JSON.stringify(del.body));
    assert.equal((await agent.get('/api/notification-settings')).body.pushSubscriptions, 0);
  } finally {
    svc.server.close();
  }
});

test('items can be archived, restored, searched and exported', async () => {
  const { agent } = await newPersonal();
  const tv = (await agent.post('/api/assets').send({ name: 'Living room TV', type: 'TVs & Monitors', manufacturer: 'LG', serial: 'LG-55' }).expect(201)).body;
  const ps = (await agent.post('/api/assets').send({ name: '=HYPERLINK("x")', type: 'Gaming', notes: 'PS5' }).expect(201)).body;
  const archived = (await agent.post(`/api/assets/${tv.id}/archive`).expect(200)).body;
  assert.ok(archived.archivedAt);
  assert.deepEqual((await agent.get('/api/assets')).body.items.map((a) => a.id), [ps.id]);
  assert.deepEqual((await agent.get('/api/assets?archived=true')).body.items.map((a) => a.id), [tv.id]);
  assert.equal((await agent.get('/api/assets?archived=all&q=LG')).body.total, 1);
  const events = (await agent.get(`/api/assets/${tv.id}/events`)).body.items;
  assert.equal(events[0].type, 'archived');
  const restored = (await agent.post(`/api/assets/${tv.id}/restore`).expect(200)).body;
  assert.equal(restored.archivedAt, null);

  const csv = await agent.get('/api/assets/export?type=Gaming').expect(200);
  assert.match(csv.headers['content-type'], /text\/csv/);
  const lines = csv.text.trim().split('\r\n');
  assert.equal(lines.length, 2);
  assert.match(lines[0], /^"Asset tag","Name","Category","Brand"/);
  assert.match(lines[1], /"'=HYPERLINK\(""x""\)"/, 'formulas are neutralised');
  const other = await newOrg();
  await other.agent.post(`/api/assets/${tv.id}/archive`).expect(404);
  assert.equal((await other.agent.get('/api/assets/export')).text.trim().split('\r\n').length, 1);
});
