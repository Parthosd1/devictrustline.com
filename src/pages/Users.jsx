import React, { useCallback, useEffect, useState } from 'react';
import { Plus } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { ROLES, ROLE_LABELS } from '../constants.js';

const ROLE_HELP = {
  admin: 'Everything, including users and roles',
  manager: 'Assets, buildings and locations; can view users',
  technician: 'Add and edit assets',
  auditor: 'View everything; audit tools arrive in a later phase',
  viewer: 'Read-only',
};

function AddUser({ onClose, onSaved }) {
  const [form, setForm] = useState({ name: '', email: '', role: 'technician', password: '' });
  const [error, setError] = useState(null);
  const set = (key) => (e) => setForm({ ...form, [key]: e.target.value });
  async function submit(e) {
    e.preventDefault();
    try {
      await api('/users', { method: 'POST', body: form });
      onSaved();
    } catch (err) { setError(err); }
  }
  return (
    <Modal title="Add User" subtitle="Share the temporary password with them directly. They can change it after signing in." onClose={onClose}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name"><input required value={form.name} onChange={set('name')} /></Field>
          <Field label="Email"><input required type="email" value={form.email} onChange={set('email')} /></Field>
          <Field label="Role">
            <select value={form.role} onChange={set('role')}>{ROLES.map((r) => <option key={r} value={r}>{ROLE_LABELS[r]}</option>)}</select>
          </Field>
          <Field label="Temporary password">
            <input required minLength={12} type="text" autoComplete="off" value={form.password} onChange={set('password')} placeholder="12+ characters" />
          </Field>
        </div>
        <p className="hint">{ROLE_LABELS[form.role]}: {ROLE_HELP[form.role]}.</p>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><Plus size={16} /> Add User</button>
        </div>
      </form>
    </Modal>
  );
}

export function Users() {
  const { user: me, can } = useAuth();
  const [users, setUsers] = useState(null);
  const [error, setError] = useState(null);
  const [adding, setAdding] = useState(false);
  const manage = can('users:manage');

  const load = useCallback(() => api('/users').then((r) => setUsers(r.items)).catch(setError), []);
  useEffect(() => { load(); }, [load]);

  async function update(id, body) {
    try {
      await api(`/users/${id}`, { method: 'PATCH', body });
      setError(null);
      load();
    } catch (err) { setError(err); }
  }

  return (
    <section className="panel">
      <div className="panel-head wrap">
        <div><h2>Team <span className="count">{users?.length ?? 0}</span></h2><p>People with access to this workspace</p></div>
        {manage && <button className="outline" onClick={() => setAdding(true)}><Plus size={16} /> Add User</button>}
      </div>
      <ErrorBanner error={error} />
      <div className="table-scroll">
        <table>
          <thead><tr><th>NAME</th><th>ROLE</th><th>STATUS</th><th>LAST SIGN-IN</th>{manage && <th />}</tr></thead>
          <tbody>
            {users === null && <tr><td colSpan="5" className="empty">Loading…</td></tr>}
            {users?.map((u) => (
              <tr key={u.id}>
                <td><strong className="normal">{u.name}{u.id === me.id && ' (you)'}</strong><small>{u.email}</small></td>
                <td>
                  {manage ? (
                    <select className="compact" value={u.role} aria-label={`Role for ${u.name}`} onChange={(e) => update(u.id, { role: e.target.value })}>
                      {ROLES.map((r) => <option key={r} value={r}>{ROLE_LABELS[r]}</option>)}
                    </select>
                  ) : ROLE_LABELS[u.role]}
                </td>
                <td><span className={`status ${u.isActive ? 'deployed' : 'retired'}`}><i />{u.isActive ? 'Active' : 'Deactivated'}</span></td>
                <td>{u.lastLoginAt ? new Date(u.lastLoginAt).toLocaleDateString() : 'Never'}</td>
                {manage && (
                  <td>
                    {u.id !== me.id && (
                      <button className="outline" onClick={() => update(u.id, { isActive: !u.isActive })}>
                        {u.isActive ? 'Deactivate' : 'Reactivate'}
                      </button>
                    )}
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {adding && <AddUser onClose={() => setAdding(false)} onSaved={() => { setAdding(false); load(); }} />}
    </section>
  );
}
