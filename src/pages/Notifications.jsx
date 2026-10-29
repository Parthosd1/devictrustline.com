import React, { useCallback, useEffect, useState } from 'react';
import { BellOff, CheckCheck, Send, Smartphone, Trash2 } from 'lucide-react';
import { api, query } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field } from '../components.jsx';
import { fmtDateTime } from '../format.js';

const CHANNELS = [['inApp', 'In app'], ['email', 'Email'], ['push', 'Browser push'], ['sms', 'Text message']];
const DELIVERY = { sent: 'sent', queued: 'queued', failed: 'failed', unavailable: 'not set up', skipped: 'skipped', pending: 'waiting', deferred: 'after quiet hours' };

// Every notification, with what happened on each channel. The bell shows the latest few.
export function NotificationCenter({ onOpenAsset }) {
  const [unread, setUnread] = useState(false);
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const load = useCallback(() => api(`/notifications${query({ limit: 200, unread: unread ? 'true' : '' })}`).then(setData).catch(setError), [unread]);
  useEffect(() => { load(); }, [load]);

  async function act(fn) { try { await fn(); setError(null); load(); } catch (err) { setError(err); } }
  return (
    <section className="panel">
      <div className="panel-head wrap">
        <div><h2>Notifications {data && <span className="count">{data.unread} unread</span>}</h2><p>Reminders and workspace events, newest first, with how each was delivered.</p></div>
        {data?.unread > 0 && <button className="outline" onClick={() => act(() => api('/notifications/read-all', { method: 'POST' }))}><CheckCheck size={15} /> Mark all read</button>}
      </div>
      <div className="chips" style={{ paddingTop: 0 }}>
        <button className={!unread ? 'chip on' : 'chip'} onClick={() => setUnread(false)}>All</button>
        <button className={unread ? 'chip on' : 'chip'} onClick={() => setUnread(true)}>Unread</button>
      </div>
      <ErrorBanner error={error} />
      <ul className="note-list">
        {data === null && <li className="empty">Loading…</li>}
        {data?.items.length === 0 && <li className="empty">{unread ? 'Nothing unread.' : 'No notifications yet.'}</li>}
        {data?.items.map((n) => (
          <li key={n.id} className={n.readAt ? '' : 'unread'}>
            <button className="link-like note-main" onClick={() => act(async () => {
              if (!n.readAt) await api(`/notifications/${n.id}/read`, { method: 'POST' });
              if (n.assetId) onOpenAsset({ id: n.assetId, name: '', assetTag: '', type: '', status: '' });
            })}>
              <strong>{n.title}</strong>
              <span>{n.body}</span>
              <small>
                {fmtDateTime(n.createdAt)}
                {(n.deliveries ?? []).map((d) => ` · ${d.channel}: ${DELIVERY[d.status] ?? d.status}${d.status === 'failed' && d.error ? ` (${d.error})` : ''}`).join('')}
              </small>
            </button>
            <button className="link-btn" aria-label={`Delete notification ${n.title}`} title="Delete" onClick={() => act(() => api(`/notifications/${n.id}`, { method: 'DELETE' }))}><Trash2 size={15} /></button>
          </li>
        ))}
      </ul>
    </section>
  );
}

const urlKey = (b64) => {
  const s = atob((b64 + '='.repeat((4 - (b64.length % 4)) % 4)).replace(/-/g, '+').replace(/_/g, '/'));
  return Uint8Array.from(s, (c) => c.charCodeAt(0));
};

// Your own delivery settings: channels, phone, time zone, quiet hours and default reminder days.
export function NotificationSettings() {
  const [s, setS] = useState(null);
  const [draft, setDraft] = useState(null);
  const [error, setError] = useState(null);
  const [saved, setSaved] = useState(null);
  const [zones, setZones] = useState([]);
  const fill = (v) => {
    setS(v);
    setDraft({
      phone: v.phone ?? '', timezone: v.timezone ?? '', quietStart: v.quietStart ?? '', quietEnd: v.quietEnd ?? '',
      sendHour: String(v.sendHour), defaultOffsets: v.defaultOffsets.join(', '),
    });
  };
  const load = useCallback(() => api('/notification-settings').then(fill).catch(setError), []);
  useEffect(() => {
    load();
    try { setZones(Intl.supportedValuesOf('timeZone')); } catch { setZones([]); }
  }, [load]);

  async function patch(body, message = 'Saved.') {
    setSaved(null);
    try { fill(await api('/notification-settings', { method: 'PATCH', body })); setError(null); setSaved(message); } catch (err) { setError(err); }
  }
  async function enablePush() {
    setSaved(null);
    try {
      if (!('serviceWorker' in navigator) || !('PushManager' in window)) throw new Error('This browser does not support push notifications.');
      if (await Notification.requestPermission() !== 'granted') throw new Error('Notifications are blocked for this site in your browser settings.');
      const reg = await navigator.serviceWorker.register('/sw.js');
      await navigator.serviceWorker.ready;
      const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: urlKey(s.pushPublicKey) });
      await api('/push/subscriptions', { method: 'POST', body: sub.toJSON() });
      await patch({ push: true }, 'Push notifications are on for this browser.');
    } catch (err) { setError(err); }
  }
  async function disablePush() {
    try {
      const reg = await navigator.serviceWorker?.getRegistration('/sw.js');
      const sub = await reg?.pushManager.getSubscription();
      await api('/push/subscriptions', { method: 'DELETE', body: sub ? { endpoint: sub.endpoint } : {} });
      await sub?.unsubscribe();
      await patch({ push: false }, 'Push notifications are off.');
    } catch (err) { setError(err); }
  }

  if (!draft) return <section className="panel settings-panel"><div className="settings-body"><ErrorBanner error={error} /><p className="muted">Loading…</p></div></section>;
  const set = (k) => (e) => setDraft({ ...draft, [k]: e.target.value });
  const offsets = draft.defaultOffsets.split(/[\s,]+/).filter(Boolean).map(Number);
  return (
    <section className="panel settings-panel">
      <div className="panel-head"><div><h2><Smartphone size={17} /> How you are notified</h2><p>Applies to your reminders and to workspace events. Quiet hours hold email, push and text messages until they end.</p></div></div>
      <div className="settings-body">
        <ErrorBanner error={error} />
        <label className="check"><input type="checkbox" checked={s.enabled} onChange={(e) => patch({ enabled: e.target.checked })} /> Send me notifications</label>
        <label className="check"><input type="checkbox" checked={s.email} disabled={!s.enabled} onChange={(e) => patch({ email: e.target.checked })} /> Email {!s.available.email && <span className="muted">(email is not set up on this server yet)</span>}</label>
        <div className="copy-row">
          <span>Browser push: <strong>{s.push && s.pushSubscriptions > 0 ? `on (${s.pushSubscriptions} browser${s.pushSubscriptions === 1 ? '' : 's'})` : 'off'}</strong></span>
          {!s.available.push ? <span className="muted">Not set up on this server yet</span>
            : s.push && s.pushSubscriptions > 0 ? <button className="outline" onClick={disablePush}><BellOff size={15} /> Turn off</button>
              : <button className="outline" disabled={!s.enabled} onClick={enablePush}>Turn on for this browser</button>}
        </div>
        <label className="check"><input type="checkbox" checked={s.sms} disabled={!s.enabled || !s.available.sms || !s.phone || !s.planIncludesSms} onChange={(e) => patch({ sms: e.target.checked })} /> Text messages {!s.planIncludesSms ? <span className="muted">(not part of this workspace’s plan)</span> : !s.available.sms ? <span className="muted">(not set up on this server yet)</span> : !s.phone && <span className="muted">(add a phone number first)</span>}</label>
        <form className="form-grid" onSubmit={(e) => {
          e.preventDefault();
          patch({
            phone: draft.phone || null, timezone: draft.timezone || null, quietStart: draft.quietStart || null, quietEnd: draft.quietEnd || null,
            sendHour: Number(draft.sendHour), defaultOffsets: offsets,
          });
        }}>
          <Field label="Mobile number (international, e.g. +15551234567)"><input type="tel" value={draft.phone} onChange={set('phone')} maxLength={20} /></Field>
          <Field label="Your time zone">
            <select value={draft.timezone} onChange={set('timezone')}>
              <option value="">Same as the workspace ({s.workspaceTimezone})</option>
              {zones.map((z) => <option key={z}>{z}</option>)}
            </select>
          </Field>
          <Field label="Quiet from"><input type="time" value={draft.quietStart} onChange={set('quietStart')} /></Field>
          <Field label="Quiet until"><input type="time" value={draft.quietEnd} onChange={set('quietEnd')} /></Field>
          <Field label="Send reminders at (hour)">
            <select value={draft.sendHour} onChange={set('sendHour')}>{Array.from({ length: 24 }, (_, h) => <option key={h} value={h}>{String(h).padStart(2, '0')}:00</option>)}</select>
          </Field>
          <Field label="Default days before, for new reminders"><input value={draft.defaultOffsets} onChange={set('defaultOffsets')} inputMode="numeric" /></Field>
          <div className="row-end wide">
            {saved && <span className="hint ok" role="status">{saved}</span>}
            <button type="button" className="outline" onClick={() => api('/notification-settings/test', { method: 'POST' }).then(() => setSaved('Test sent on each channel you have on. Check the delivery under Notifications.')).catch(setError)}><Send size={15} /> Send a test</button>
            <button className="primary">Save</button>
          </div>
        </form>
      </div>
    </section>
  );
}

function ChannelGrid({ groups, value, onChange, disabled }) {
  return (
    <div className="table-scroll">
      <table className="channel-grid">
        <thead><tr><th>EVENTS</th>{CHANNELS.map(([k, l]) => <th key={k}>{l.toUpperCase()}</th>)}<th /></tr></thead>
        <tbody>
          {groups.map((g) => {
            const v = value(g);
            return (
              <tr key={g.group}>
                <td><strong className="normal">{g.label}</strong>{g.note && <small>{g.note}</small>}</td>
                {CHANNELS.map(([k, l]) => (
                  <td key={k}><input type="checkbox" aria-label={`${g.label}: ${l}`} checked={v.channels[k]} disabled={disabled}
                    onChange={(e) => onChange(g, { ...v.channels, [k]: e.target.checked })} /></td>
                ))}
                <td>{v.reset}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// Which workspace events reach you, and on which channels. Unset groups follow the workspace default.
export function EventPreferences() {
  const { user } = useAuth();
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const load = useCallback(() => api('/notification-settings/events').then(setData).catch(setError), []);
  useEffect(() => { load(); }, [load]);
  async function put(body) { try { setData(await api('/notification-settings/events', { method: 'PUT', body })); setError(null); } catch (err) { setError(err); } }
  if (user.organization.kind !== 'business') return null;
  return (
    <section className="panel settings-panel span-all">
      <div className="panel-head"><div><h2>Workspace events</h2><p>Transfers, access requests, audits, missing items, loaners, assignments, tasks and invitations. You only hear about items you can see.</p></div></div>
      <ErrorBanner error={error} />
      {data && (
        <ChannelGrid groups={data.groups} onChange={(g, channels) => put({ [g.group]: channels })}
          value={(g) => ({
            channels: g.effective,
            reset: g.own ? <button className="text-link" onClick={() => put({ [g.group]: null })}>Use workspace default</button> : <span className="muted small">Workspace default</span>,
          })} />
      )}
    </section>
  );
}

// Admins: the default channels for each event group, for everyone who has not chosen their own.
export function WorkspaceNotificationDefaults() {
  const { can, user } = useAuth();
  const [settings, setSettings] = useState(null);
  const [groups, setGroups] = useState([]);
  const [error, setError] = useState(null);
  useEffect(() => {
    api('/settings').then(setSettings).catch(setError);
    api('/notification-settings/events').then((d) => setGroups(d.groups)).catch(() => {});
  }, []);
  if (user.organization.kind !== 'business' || !can('settings:manage')) return null;
  async function change(g, channels) {
    try { setSettings(await api('/settings', { method: 'PATCH', body: { notifications: { [g.group]: channels } } })); setError(null); } catch (err) { setError(err); }
  }
  return (
    <section className="panel settings-panel span-all">
      <div className="panel-head"><div><h2>Workspace notification defaults</h2><p>The channels each event group uses for people who have not chosen their own. Each person can still turn channels off for themselves.</p></div></div>
      <ErrorBanner error={error} />
      {settings && groups.length > 0 && (
        <ChannelGrid groups={groups} onChange={change}
          value={(g) => ({ channels: settings.notifications?.[g.group] ?? { inApp: true, email: true, push: true, sms: true }, reset: null })} />
      )}
    </section>
  );
}
