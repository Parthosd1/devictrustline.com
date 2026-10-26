import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import request from 'supertest';
import { PASSWORD, app, newOrg, pool, resetDatabase } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

let n = 0;

// Walks a browser (agent) through Google: start -> stand-in Google -> callback.
// `who` picks the Google account; `tamper` alters the ID token or callback.
async function google(agent, who, { mode = 'login', tamper = {} } = {}) {
  const start = await agent.get(`/api/auth/google/start?mode=${mode}`).expect(302);
  const authorize = new URL(start.headers.location);
  assert.equal(authorize.searchParams.get('code_challenge_method'), 'S256');
  assert.ok(authorize.searchParams.get('nonce'));
  for (const [k, v] of Object.entries({ ...who, ...tamper })) authorize.searchParams.set(k, v);
  const idp = await fetch(authorize, { redirect: 'manual' });
  const back = new URL(idp.headers.get('location'));
  const res = await agent.get(`${back.pathname}${back.search}`).expect(302);
  return new URL(res.headers.location).searchParams;
}

const person = () => ({ sub: `google-${++n}-${Date.now()}`, email: `g${n}-${Date.now()}@gmail.com` });

test('first Google sign-in creates a verified personal account; later ones sign in by subject', async () => {
  const who = person();
  const agent = request.agent(app);
  assert.equal((await google(agent, who)).get('google'), 'signed_in');
  const me = await agent.get('/api/auth/me').expect(200);
  assert.equal(me.body.user.email, who.email);
  assert.equal(me.body.user.emailVerified, true);
  assert.equal(me.body.user.hasPassword, false);
  assert.equal(me.body.user.organization.kind, 'personal');

  // Same subject, different email at Google: still the same account.
  const again = request.agent(app);
  assert.equal((await google(again, { ...who, email: `changed-${who.email}` })).get('google'), 'signed_in');
  assert.equal((await again.get('/api/auth/me')).body.user.id, me.body.user.id);
});

test('a Google email matching an existing account does not sign in to it', async () => {
  const { email } = await newOrg();
  const agent = request.agent(app);
  const result = await google(agent, { sub: `attacker-${Date.now()}`, email });
  assert.equal(result.get('google_error'), 'link_required');
  await agent.get('/api/auth/me').expect(401);
});

test('unverified Google emails cannot create accounts', async () => {
  const agent = request.agent(app);
  const result = await google(agent, { ...person(), email_verified: 'false' });
  assert.equal(result.get('google_error'), 'email_unverified');
});

test('linking needs a signed-in account and then signs in by Google', async () => {
  await request(app).get('/api/auth/google/start?mode=link').expect(401);
  const { agent, user } = await newOrg();
  const who = { ...person(), email: 'different-address@gmail.com' };
  assert.equal((await google(agent, who, { mode: 'link' })).get('google'), 'linked');
  assert.equal((await agent.get('/api/account')).body.googleLinked, true);

  const fresh = request.agent(app);
  assert.equal((await google(fresh, who)).get('google'), 'signed_in');
  assert.equal((await fresh.get('/api/auth/me')).body.user.id, user.id);

  // The same Google account cannot be linked to a second account.
  const other = await newOrg();
  assert.equal((await google(other.agent, who, { mode: 'link' })).get('google_error'), 'already_linked');

  // After unlinking, that Google account no longer reaches this account.
  await agent.delete('/api/auth/google').expect(200);
  const after = request.agent(app);
  await google(after, who);
  assert.notEqual((await after.get('/api/auth/me')).body.user.id, user.id);
  await request(app).post('/api/auth/login').send({ email: user.email, password: PASSWORD }).expect(200);
});

test('Google-only accounts cannot unlink their only way in', async () => {
  const agent = request.agent(app);
  await google(agent, person());
  await agent.delete('/api/auth/google').expect(400);
});

test('forged, replayed and tampered callbacks are refused', async () => {
  // Wrong nonce in the ID token.
  assert.equal((await google(request.agent(app), person(), { tamper: { bad_nonce: '1' } })).get('google_error'), 'invalid');
  // ID token for another app.
  assert.equal((await google(request.agent(app), person(), { tamper: { aud: 'someone-else' } })).get('google_error'), 'invalid');
  // ID token from another issuer.
  assert.equal((await google(request.agent(app), person(), { tamper: { iss: 'https://evil.example' } })).get('google_error'), 'invalid');

  // A callback in a browser that did not start the sign-in (login CSRF), and a replayed one.
  const agent = request.agent(app);
  const start = await agent.get('/api/auth/google/start').expect(302);
  const authorize = new URL(start.headers.location);
  for (const [k, v] of Object.entries(person())) authorize.searchParams.set(k, v);
  const back = new URL((await fetch(authorize, { redirect: 'manual' })).headers.get('location'));
  const path = `${back.pathname}${back.search}`;
  const forged = await request(app).get(path).expect(302);
  assert.equal(new URL(forged.headers.location).searchParams.get('google_error'), 'invalid');
  const ok = await agent.get(path).expect(302);
  assert.equal(new URL(ok.headers.location).searchParams.get('google'), 'signed_in');
  const replay = await agent.get(path).expect(302);
  assert.equal(new URL(replay.headers.location).searchParams.get('google_error'), 'invalid');
});
