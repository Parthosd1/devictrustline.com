import React, { useEffect, useState } from 'react';
import { Bell, Globe, UserRound } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field } from '../components.jsx';
import { EventPreferences, NotificationSettings, WorkspaceNotificationDefaults } from './Notifications.jsx';

const REMINDER_ROWS = [
  ['warrantyDays', 'Warranty ending'], ['returnDays', 'Return window closing'], ['maintenanceDays', 'Maintenance due'],
  ['loanDays', 'Loan due back'], ['auditDays', 'Audit due'],
];
const parseDays = (text) => text.split(/[\s,]+/).filter(Boolean).map(Number);

function Account() {
  const { refresh } = useAuth();
  const [account, setAccount] = useState(null);
  const [config, setConfig] = useState({});
  const [name, setName] = useState('');
  const [error, setError] = useState(null);
  const [saved, setSaved] = useState(null);
  useEffect(() => {
    api('/account').then((a) => { setAccount(a); setName(a.name); }).catch(setError);
    api('/auth/config').then(setConfig).catch(() => {});
  }, []);

  async function save(body, message) {
    setSaved(null);
    try {
      const a = await api('/account', { method: 'PATCH', body });
      setAccount(a);
      setError(null);
      setSaved(message);
      refresh();
    } catch (err) { setError(err); }
  }
  async function unlink() {
    if (!window.confirm('Stop signing in with Google? You will need your password.')) return;
    try { await api('/auth/google', { method: 'DELETE' }); setAccount({ ...account, googleLinked: false }); setSaved('Google unlinked.'); } catch (err) { setError(err); }
  }
  if (!account) return <section className="panel settings-panel"><div className="settings-body"><ErrorBanner error={error} /></div></section>;
  return (
    <section className="panel settings-panel">
      <div className="panel-head"><div><h2><UserRound size={17} /> Your account</h2><p>{account.email}{account.emailVerified ? '' : ' · not confirmed yet'}</p></div></div>
      <div className="settings-body">
        <ErrorBanner error={error} />
        <form className="copy-row" onSubmit={(e) => { e.preventDefault(); save({ name }, 'Name saved.'); }}>
          <Field label="Your name" wide><input required maxLength={120} value={name} onChange={(e) => setName(e.target.value)} /></Field>
          <button className="outline" disabled={name === account.name}>Save</button>
        </form>
        {config.google && (
          <div className="copy-row">
            <span>Google sign-in: <strong>{account.googleLinked ? 'linked' : 'not linked'}</strong></span>
            {account.googleLinked
              ? account.hasPassword && <button className="outline" onClick={unlink}>Unlink Google</button>
              : <a className="outline button-link" href="/api/auth/google/start?mode=link">Link Google</a>}
          </div>
        )}
        {config.google && account.googleLinked && !account.hasPassword && <p className="hint">Set a password with "Forgot password" on the sign-in page before unlinking Google.</p>}
        {saved && <p className="hint ok" role="status">{saved}</p>}
      </div>
    </section>
  );
}

function Workspace() {
  const { can, user } = useAuth();
  const rows = REMINDER_ROWS.filter(([k]) => k !== 'auditDays' || user.organization.kind === 'business');
  const [settings, setSettings] = useState(null);
  const [draft, setDraft] = useState(null);
  const [zones, setZones] = useState([]);
  const [error, setError] = useState(null);
  const [saved, setSaved] = useState(false);
  const editable = can('settings:manage');
  const fill = (s) => {
    setSettings(s);
    setDraft({
      timezone: s.timezone, currency: s.currency ?? '', expiringSoonDays: String(s.reminders.expiringSoonDays),
      ...Object.fromEntries(REMINDER_ROWS.map(([k]) => [k, (s.reminders[k] ?? []).join(', ')])),
    });
  };
  useEffect(() => {
    api('/settings').then(fill).catch(setError);
    try { setZones(Intl.supportedValuesOf('timeZone')); } catch { setZones([]); }
  }, []);

  async function submit(e) {
    e.preventDefault();
    setSaved(false);
    const reminders = { expiringSoonDays: Number(draft.expiringSoonDays) };
    for (const [k] of REMINDER_ROWS) reminders[k] = parseDays(draft[k]);
    const body = { timezone: draft.timezone, reminders };
    if (draft.currency) body.currency = draft.currency;
    try { fill(await api('/settings', { method: 'PATCH', body })); setError(null); setSaved(true); } catch (err) { setError(err); }
  }
  if (!draft) return <section className="panel settings-panel"><div className="settings-body"><ErrorBanner error={error} /></div></section>;
  const set = (k) => (e) => setDraft({ ...draft, [k]: e.target.value });
  return (
    <section className="panel settings-panel">
      <div className="panel-head"><div><h2><Bell size={17} /> Reminders & region</h2><p>When reminders go out, counted in days before each date. Reminders fire in the morning of the workspace time zone.</p></div></div>
      <form className="settings-body form-grid" onSubmit={submit}>
        <ErrorBanner error={error} />
        <Field label="Time zone">
          {zones.length ? (
            <select value={draft.timezone} onChange={set('timezone')} disabled={!editable}>
              {!zones.includes(draft.timezone) && <option>{draft.timezone}</option>}
              {zones.map((z) => <option key={z}>{z}</option>)}
            </select>
          ) : <input value={draft.timezone} onChange={set('timezone')} disabled={!editable} />}
        </Field>
        <Field label="Default currency">
          <input value={draft.currency} onChange={set('currency')} maxLength={3} placeholder="USD" disabled={!editable} />
        </Field>
        {rows.map(([k, label]) => (
          <Field key={k} label={`${label} (days before)`}>
            <input value={draft[k]} onChange={set(k)} inputMode="numeric" placeholder="e.g. 30, 7, 1" disabled={!editable} />
          </Field>
        ))}
        <Field label="Count a warranty as ending soon within (days)">
          <input type="number" min={1} max={365} value={draft.expiringSoonDays} onChange={set('expiringSoonDays')} disabled={!editable} />
        </Field>
        <div className="row-end wide">
          {!editable && <span className="hint">Only administrators can change this.</span>}
          {saved && <span className="hint ok" role="status"><Globe size={14} /> Saved.</span>}
          {editable && <button className="primary">Save</button>}
        </div>
      </form>
    </section>
  );
}

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
      <div className="panel-head"><div><h2>Scheduled Audits</h2><p>Start an organization-wide audit automatically at the start of each period. Each frequency runs independently.</p></div></div>
      <div className="settings-body">
        <ErrorBanner error={error} />
        {settings && [['weekly', 'Monday'], ['monthly', '1st of the month'], ['quarterly', 'quarter (Jan, Apr, Jul, Oct)'], ['annual', 'January 1st']].map(([p, when]) => (
          <label className="check" key={p}>
            <input type="checkbox" checked={Boolean(settings.autoAudits[p])} disabled={!editable} onChange={(e) => toggle(p, e.target.checked)} />
            Start {p === 'annual' ? 'an' : 'a'} {p} audit every {when}
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
  const { user } = useAuth();
  return (
    <div className="settings-grid">
      <Account />
      <NotificationSettings />
      <Workspace />
      {user.organization.kind === 'business' && <AutoAudits />}
      <ChangePassword />
      <EventPreferences />
      <WorkspaceNotificationDefaults />
    </div>
  );
}
