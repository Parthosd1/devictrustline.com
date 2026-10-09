import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { newOrg, pool, resetDatabase, userWithRole } from './helpers.js';

before(resetDatabase);
after(() => pool.end());

const asset = { name: 'Dell Latitude 5440', type: 'Laptop', serial: 'RBAC-1' };

test('each role gets exactly its permissions', async () => {
  const { agent: admin } = await newOrg();
  const expectations = {
    manager: { writeAsset: 201, writeSite: 201, listUsers: 200, createUser: 403 },
    technician: { writeAsset: 201, writeSite: 403, listUsers: 403, createUser: 403 },
    auditor: { writeAsset: 403, writeSite: 403, listUsers: 403, createUser: 403 },
    viewer: { writeAsset: 403, writeSite: 403, listUsers: 403, createUser: 403 },
  };
  let i = 0;
  for (const [role, exp] of Object.entries(expectations)) {
    const { agent } = await userWithRole(admin, role);
    await agent.get('/api/assets').expect(200);
    await agent.get('/api/buildings').expect(200);
    await agent.post('/api/assets').send({ ...asset, serial: `RBAC-${++i}` }).expect(exp.writeAsset);
    await agent.post('/api/buildings').send({ name: `B-${role}` }).expect(exp.writeSite);
    await agent.get('/api/users').expect(exp.listUsers);
    await agent.post('/api/users')
      .send({ name: 'x', email: `x-${role}@example.com`, role: 'viewer', password: 'long enough password' })
      .expect(exp.createUser);
  }
});

test('role changes apply to existing sessions', async () => {
  const { agent: admin } = await newOrg();
  const user = await userWithRole(admin, 'viewer');
  await user.agent.post('/api/assets').send({ ...asset, serial: 'PROMOTE' }).expect(403);
  await admin.patch(`/api/users/${user.id}`).send({ role: 'technician' }).expect(200);
  await user.agent.post('/api/assets').send({ ...asset, serial: 'PROMOTE' }).expect(201);
});

test('the last active admin cannot be demoted or deactivated', async () => {
  const { agent: admin, user } = await newOrg();
  await admin.patch(`/api/users/${user.id}`).send({ role: 'manager' }).expect(409);
  await admin.patch(`/api/users/${user.id}`).send({ isActive: false }).expect(409);
  const second = await userWithRole(admin, 'admin');
  await admin.patch(`/api/users/${second.id}`).send({ role: 'viewer' }).expect(200);
  await admin.patch(`/api/users/${second.id}`).send({ role: 'admin' }).expect(200);
  const res = await admin.patch(`/api/users/${user.id}`).send({ role: 'manager' }).expect(200);
  assert.equal(res.body.role, 'manager');
});

test('admin password reset signs the user out', async () => {
  const { agent: admin } = await newOrg();
  const user = await userWithRole(admin, 'technician');
  await admin.patch(`/api/users/${user.id}`).send({ password: 'a brand new password' }).expect(200);
  await user.agent.get('/api/auth/me').expect(401);
});
