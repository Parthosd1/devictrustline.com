import React, { useCallback, useEffect, useState } from 'react';
import { Copy, Mail, Plus, X } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { ROLES, ROLE_LABELS } from '../constants.js';

const ROLE_HELP = {
  admin: 'Everything, including users and roles',
  manager: 'Items, places, categories and devices; can view users',
  technician: 'Add and edit items, log maintenance',
  auditor: 'View everything and run audits',
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

function InviteForm({ onClose, onSent }) {
  const [form, setForm] = useState({ email: '', role: 'viewer' });
  const [error, setError] = useState(null);
  const [result, setResult] = useState(null);
  const [busy, setBusy] = useState(false);
  const set = (key) => (e) => setForm({ ...form, [key]: e.target.value });
  async function submit(e) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await api('/invitations', { method: 'POST', body: form });
      setError(null);
      onSent();
      if (r.emailed) onClose(`Invitation queued for email to ${r.email}. Check the pending list below; you can cancel and resend it.`);
      else setResult(r);
    } catch (err) { setError(err); }
    setBusy(false);
  }
  if (result) {
    return (
      <Modal title="Share this invitation link" subtitle={result.email} onClose={() => onClose(null)}>
        <p className="hint">Email is not set up on this server, so nothing was sent. Send this link to {result.email} yourself. It works once and expires in 7 days.</p>
        <div className="copy-row">
          <input className="full-input" readOnly value={result.link} aria-label="Invitation link" onFocus={(e) => e.target.select()} />
          <button className="outline" onClick={() => navigator.clipboard?.writeText(result.link)}><Copy size={15} /> Copy</button>
        </div>
        <div className="modal-actions"><button className="primary" onClick={() => onClose(null)}>Done</button></div>
      </Modal>
    );
  }
  return (
    <Modal title="Invite someone" subtitle="They choose their own password, or sign in with Google, when they accept." onClose={() => onClose(null)}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Email"><input required type="email" value={form.email} onChange={set('email')} /></Field>
          <Field label="Role">
            <select value={form.role} onChange={set('role')}>{ROLES.map((r) => <option key={r} value={r}>{ROLE_LABELS[r]}</option>)}</select>
          </Field>
        </div>
        <p className="hint">{ROLE_LABELS[form.role]}: {ROLE_HELP[form.role]}.</p>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={() => onClose(null)}>Cancel</button>
          <button className="primary" disabled={busy}><Mail size={16} /> {busy ? 'Sending…' : 'Send invitation'}</button>
        </div>
      </form>
    </Modal>
  );
}

function PendingInvitations({ version, onError }) {
  const [items, setItems] = useState([]);
  const load = useCallback(() => api('/invitations').then((r) => setItems(r.items)).catch(onError), [onError]);
  useEffect(() => { load(); }, [load, version]);
  async function revoke(i) {
    if (!window.confirm(`Cancel the invitation for ${i.email}?`)) return;
    try { await api(`/invitations/${i.id}`, { method: 'DELETE' }); load(); } catch (err) { onError(err); }
  }
  if (items.length === 0) return null;
  return (
    <div className="pad">
      <h3 className="section-title">Pending invitations</h3>
      <ul className="name-list">
        {items.map((i) => (
          <li key={i.id}>
            <span><strong className="normal">{i.email}</strong><small>{ROLE_LABELS[i.role]} · invited by {i.invitedBy ?? 'someone'} · expires {new Date(i.expiresAt).toLocaleDateString()}</small></span>
            <button className="link-btn" aria-label={`Cancel invitation for ${i.email}`} onClick={() => revoke(i)}><X size={15} /></button>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function Users() {
  const { user: me, can } = useAuth();
  const [users, setUsers] = useState(null);
  const [error, setError] = useState(null);
  const [adding, setAdding] = useState(false);
  const [inviting, setInviting] = useState(false);
  const [invites, setInvites] = useState(0);
  const [notice, setNotice] = useState(null);
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
        {manage && (
          <div className="row-end">
            <button className="primary" onClick={() => setInviting(true)}><Mail size={16} /> Invite</button>
            <button className="outline" onClick={() => setAdding(true)}><Plus size={16} /> Add with password</button>
          </div>
        )}
      </div>
      <ErrorBanner error={error} />
      {notice && <div className="notice" role="status">{notice}</div>}
      {manage && <PendingInvitations version={invites} onError={setError} />}
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
      {inviting && <InviteForm onSent={() => setInvites((v) => v + 1)} onClose={(msg) => { setInviting(false); setNotice(msg); }} />}
      {adding && <AddUser onClose={() => setAdding(false)} onSaved={() => { setAdding(false); load(); }} />}
    </section>
  );
}
