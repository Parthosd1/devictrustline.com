import React, { useEffect, useState } from 'react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field } from '../components.jsx';

function AutoAudits() {
  const { can } = useAuth();
  const [settings, setSettings] = useState(null);
  const [error, setError] = useState(null);
  const [saved, setSaved] = useState(false);
  const editable = can('settings:manage');

  useEffect(() => { api('/settings').then(setSettings).catch(setError); }, []);

  async function toggle(period, value) {
    setSaved(false);
    const previous = settings;
    setSettings({ ...settings, autoAudits: { ...settings.autoAudits, [period]: value } });
    try {
      setSettings(await api('/settings', { method: 'PATCH', body: { autoAudits: { [period]: value } } }));
      setSaved(true);
      setError(null);
    } catch (err) { setSettings(previous); setError(err); }
  }

  return (
    <section className="panel settings-panel">
      <div className="panel-head"><div><h2>Scheduled Audits</h2><p>Start an organization-wide audit automatically at the start of each period. Weekly and monthly audits run independently.</p></div></div>
      <div className="settings-body">
        <ErrorBanner error={error} />
        {settings && ['weekly', 'monthly'].map((p) => (
          <label className="check" key={p}>
            <input type="checkbox" checked={settings.autoAudits[p]} disabled={!editable} onChange={(e) => toggle(p, e.target.checked)} />
            Start a {p} audit every {p === 'weekly' ? 'Monday' : '1st of the month'}
          </label>
        ))}
        {!editable && <p className="hint">Only administrators can change this.</p>}
        {saved && <p className="hint ok" role="status">Saved.</p>}
      </div>
    </section>
  );
}

function ChangePassword() {
  const [form, setForm] = useState({ currentPassword: '', newPassword: '', confirm: '' });
  const [error, setError] = useState(null);
  const [done, setDone] = useState(false);
  const [busy, setBusy] = useState(false);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.value });

  async function submit(e) {
    e.preventDefault();
    setDone(false);
    if (form.newPassword !== form.confirm) { setError(new Error('The new passwords do not match')); return; }
    setBusy(true);
    try {
      await api('/auth/change-password', { method: 'POST', body: { currentPassword: form.currentPassword, newPassword: form.newPassword } });
      setForm({ currentPassword: '', newPassword: '', confirm: '' });
      setError(null);
      setDone(true);
    } catch (err) { setError(err); }
    setBusy(false);
  }

  return (
    <section className="panel settings-panel">
      <div className="panel-head"><div><h2>Change Password</h2><p>Signs you out everywhere else.</p></div></div>
      <form className="settings-body form-grid" onSubmit={submit}>
        <ErrorBanner error={error} />
        <Field label="Current password" wide><input type="password" autoComplete="current-password" required value={form.currentPassword} onChange={set('currentPassword')} /></Field>
        <Field label="New password"><input type="password" autoComplete="new-password" required minLength={12} value={form.newPassword} onChange={set('newPassword')} /></Field>
        <Field label="Confirm new password"><input type="password" autoComplete="new-password" required value={form.confirm} onChange={set('confirm')} /></Field>
        <div className="row-end wide">
          {done && <span className="hint ok" role="status">Password changed.</span>}
          <button className="primary" disabled={busy}>Change password</button>
        </div>
      </form>
    </section>
  );
}

export function Settings() {
  return <div className="settings-grid"><AutoAudits /><ChangePassword /></div>;
}
