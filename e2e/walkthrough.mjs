// Browser walk-through of the main workflows, driven like a person would, against the real API.
// An owner on a laptop-sized screen sets up an inventory, transfers, lends and inspects a loaner,
// adds a reminder, runs a full audit and submits it; a manager on a phone-sized screen approves it
// and opens every page. Fails on any page error, console error, failed API call or sideways scroll.
// Expects a freshly seeded workspace (see run.sh). Screenshots go to SHOTS_DIR when set.
import { chromium } from 'playwright';

const BASE = process.env.E2E_URL || 'http://127.0.0.1:4300';
const OWNER = { email: 'demo@example.com', password: process.env.SEED_ADMIN_PASSWORD || 'demo password 123' };
const MANAGER = { email: 'morgan@example.com', password: 'manager password 1' };
const PAGES = ['Approvals', 'Inventories', 'Loaners', 'Reminders', 'Notifications', 'Audits', 'Users', 'Settings', 'Devices', 'Plan'];

const errors = [];
const step = (name) => console.log(`- ${name}`);
const browser = await chromium.launch();

async function signIn({ email, password }, viewport) {
  const ctx = await browser.newContext({ viewport });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${email}: page error: ${e.message}`));
  page.on('console', (m) => {
    // The sign-in page asks who is signed in before anyone is; that 401 is expected.
    if (m.type() === 'error' && !/status of 401/.test(m.text())) errors.push(`${email}: console: ${m.text()}`);
  });
  page.on('response', async (r) => {
    if (r.url().includes('/api/') && r.status() >= 400 && !r.url().endsWith('/api/auth/me')) {
      errors.push(`${email}: ${r.status()} ${r.request().method()} ${r.url()} ${await r.text().catch(() => '')}`);
    }
  });
  page.on('dialog', (d) => d.accept(d.type() === 'prompt' ? 'Checked by the walk-through' : undefined));
  await page.goto(BASE);
  await page.fill('input[type=email]', email);
  await page.fill('input[type=password]', password);
  await page.click('button.primary');
  await page.waitForSelector('.sidebar');
  return page;
}

async function go(page, label, phone) {
  if (phone) await page.click('.mobile-menu');
  await page.click(`.nav-item:has-text("${label}")`);
  await page.waitForLoadState('networkidle');
}

async function shot(page, name) {
  if (process.env.SHOTS_DIR) await page.screenshot({ path: `${process.env.SHOTS_DIR}/${name}.png`, fullPage: true });
}

// What a manager and a borrower need, made through the API as the owner.
async function setUp(page) {
  const api = (method, path, data) => page.request.fetch(`${BASE}/api${path}`, { method, data, headers: { origin: BASE } });
  const ok = async (res) => { if (!res.ok()) throw new Error(`${res.status()} ${await res.text()}`); return res.json(); };
  await ok(await api('POST', '/users', { name: 'Morgan Manager', email: MANAGER.email, role: 'manager', password: MANAGER.password }));
  await ok(await api('POST', '/people', { name: 'Jamie Rivera', email: 'jamie@example.com' }));
}

try {
  const page = await signIn(OWNER, { width: 1366, height: 900 });
  await setUp(page);

  step('owner opens every page');
  for (const p of PAGES) { await go(page, p); await shot(page, `owner-${p}`); }

  step('owner creates an inventory and transfers an item into the loaner inventory');
  await go(page, 'Inventories');
  await page.click('button:has-text("New inventory")');
  await page.fill('.modal input >> nth=0', 'Field Kits');
  await page.selectOption('.modal select >> nth=0', 'technician');
  await page.click('.modal button.primary');
  await page.waitForSelector('td:has-text("Field Kits")');
  await page.click('.panel-head button:has-text("Transfer")');
  await page.fill('.modal textarea', 'DT-1001');
  await page.selectOption('.modal select', { label: '  Loaner Inventory' });
  await page.click('.modal button.primary');
  await page.waitForSelector('.modal :text("Moved: 1 item")');
  await page.click('.modal button:has-text("Done")');

  step('owner lends it, checks it in damaged and sends it to repair');
  await go(page, 'Loaners');
  await page.click('button:has-text("Lend")');
  await page.waitForSelector('.modal select >> nth=0 >> option:has-text("DT-1001")', { state: 'attached' });
  await page.selectOption('.modal select >> nth=0', { index: 1 });
  await page.selectOption('.modal select >> nth=1', { index: 1 });
  await page.fill('.modal input[placeholder^="Their full name"]', 'Jamie Rivera');
  await page.click('.modal button.primary');
  await page.click('.modal button:has-text("Check in")');
  await page.check('.modal form input[type=checkbox]');
  await page.fill('.modal textarea', 'Cracked hinge');
  await page.click('.modal form button.primary');
  await page.click('.modal button:has-text("Inspect")');
  await page.selectOption('.modal form select', 'repair');
  await page.fill('.modal textarea', 'Hinge replacement');
  await page.click('.modal form button.primary');
  await page.waitForSelector('.modal :text("sent for repair")');
  await shot(page, 'owner-loan');
  await page.click('.modal button[aria-label=Close]');

  step('owner adds a reminder');
  await go(page, 'Reminders');
  await page.click('button:has-text("New reminder")');
  await page.selectOption('.modal select >> nth=0', 'custom');
  await page.fill('.modal input[placeholder^="Leave empty"]', 'Renew support contract');
  await page.fill('.modal input[type=date]', '2030-12-01');
  await page.click('.modal button.primary');
  await page.waitForSelector('td:has-text("Renew support contract")');

  step('owner runs a full audit, scans an item and submits it');
  await go(page, 'Audits');
  // The tab reloads its audits and then re-mounts the start form, so choose the type only after that.
  await Promise.all([
    page.waitForResponse((r) => r.url().includes('/api/audits?') && r.url().includes('period=manual')),
    page.click('.tabs button:has-text("One-off")'),
  ]);
  await page.waitForSelector('.start-audit h2:has-text("one-off")');
  await page.selectOption('.start-audit select >> nth=0', 'full');
  await page.waitForFunction(() => document.querySelector('.start-audit select').value === 'full');
  await page.click('.start-audit button.primary');
  const scan = page.locator('input[aria-label="Audit scan"]');
  await scan.fill('DT-1001');
  await scan.press('Enter');
  await page.waitForSelector('.scan-flash:has-text("verified")');
  await page.click('button:has-text("Submit for review")');
  await page.waitForSelector('.status:has-text("Waiting for review")');

  step('owner changes an event preference and sends a test notification');
  await go(page, 'Settings');
  const loanEmail = page.locator('input[aria-label="Loaners: Email"]').first();
  await Promise.all([page.waitForResponse((r) => r.url().endsWith('/api/notification-settings/events') && r.request().method() === 'PUT'), loanEmail.click()]);
  await page.waitForFunction(() => !document.querySelector('input[aria-label="Loaners: Email"]').checked);
  await page.click('button:has-text("Send a test")');
  await page.waitForSelector(':text("Test sent")');
  await go(page, 'Notifications');
  await page.waitForSelector('.note-list :text("Test notification")');

  step('manager approves the audit from the Approval Center on a phone');
  const phone = await signIn(MANAGER, { width: 390, height: 844 });
  await go(phone, 'Approvals', true);
  await phone.click('.queue-list button:has-text("Approve")');
  await phone.waitForSelector(':text("All clear")');
  await shot(phone, 'manager-approvals');
  for (const p of PAGES.filter((x) => x !== 'Users' && x !== 'Devices')) {
    await go(phone, p, true);
    const wide = await phone.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
    if (wide) errors.push(`${p} scrolls sideways at 390px`);
    await shot(phone, `manager-${p}`);
  }
} catch (err) {
  errors.push(`step failed: ${err.message.split('\n')[0]}`);
  for (const ctx of browser.contexts()) for (const p of ctx.pages()) await shot(p, `failure-${ctx.pages().indexOf(p)}`).catch(() => {});
} finally {
  await browser.close();
}

if (errors.length) {
  console.error(`FAILED\n${errors.join('\n')}`);
  process.exit(1);
}
console.log('Walk-through passed: no page errors, console errors, failed API calls or sideways scrolling.');
