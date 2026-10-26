import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import request from 'supertest';
import { pool, resetDatabase } from './helpers.js';

const { createApp } = await import('../src/app.js');
const dir = mkdtempSync(path.join(tmpdir(), 'dt-web-'));
let app;

before(async () => {
  await resetDatabase();
  mkdirSync(path.join(dir, 'assets'));
  writeFileSync(path.join(dir, 'index.html'), '<!doctype html><title>DeviceTrustline</title>');
  writeFileSync(path.join(dir, 'assets', 'index-abc123.js'), 'console.log(1)');
  app = createApp({ staticDir: dir });
});
after(async () => { rmSync(dir, { recursive: true, force: true }); await pool.end(); });

test('serves the web app with an index.html fallback for app routes', async () => {
  const home = await request(app).get('/').expect(200);
  assert.match(home.text, /DeviceTrustline/);
  assert.equal(home.headers['cache-control'], 'no-cache');
  const deep = await request(app).get('/?asset=DT-1001').expect(200);
  assert.match(deep.text, /DeviceTrustline/);
  await request(app).get('/inventory/anything').expect(200);
});

test('hashed assets are cached long-term and missing ones are a 404, not the app', async () => {
  const js = await request(app).get('/assets/index-abc123.js').expect(200);
  assert.match(js.headers['cache-control'], /max-age=31536000.*immutable|immutable.*max-age=31536000/);
  await request(app).get('/assets/missing.js').expect(404);
});

test('API routes are never answered with the web app', async () => {
  const res = await request(app).get('/api/nope').expect(401);
  assert.equal(res.body.error, 'Sign in required');
  await request(app).get('/api/health').expect(200);
});

test('security headers restrict scripts, framing and device access', async () => {
  const res = await request(app).get('/');
  assert.match(res.headers['content-security-policy'], /script-src 'self'/);
  assert.match(res.headers['content-security-policy'], /frame-ancestors 'none'/);
  assert.equal(res.headers['permissions-policy'], 'camera=(self), microphone=(), geolocation=()');
});
