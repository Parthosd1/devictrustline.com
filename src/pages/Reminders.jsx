import React, { useCallback, useEffect, useState } from 'react';
import { AlarmClock, BellRing, CheckCircle2, Pause, Pencil, Play, Plus, Trash2 } from 'lucide-react';
import { api, query } from '../api.js';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { fmtDate, fmtDateTime } from '../format.js';
import { Badge } from './Maintenance.jsx';

// Mirrors KINDS, REPEATS and DATE_SOURCES in api/src/user_reminders.rs.
export const REMINDER_KINDS = {
  warranty: 'Warranty ends', return: 'Return window closes', maintenance: 'Maintenance due', renewal: 'Renewal or protection plan',
  inspection: 'Inspection due', loan_return: 'Loaned item due back', custom: 'Something else',
};
const REPEATS = { none: 'Does not repeat', daily: 'Every day', weekly: 'Every week', monthly: 'Every month', yearly: 'Every year' };
const DATE_SOURCES = {
  warranty: 'Its warranty end date', return_deadline: 'Its return deadline', maintenance_due: 'Its maintenance due date', loan_due: 'Its due-back date',
};
const SOURCE_FOR_KIND = { warranty: 'warranty', return: 'return_deadline', maintenance: 'maintenance_due', loan_return: 'loan_due' };
const STATUS = { active: ['Active', 'deployed'], paused: ['Paused', 'retired'], completed: ['Done', 'available'] };
const FILTERS = [['active', 'Active'], ['paused', 'Paused'], ['completed', 'Done'], ['all', 'All']];
const DELIVERY = { sent: 'sent', queued: 'queued', failed: 'failed', unavailable: 'not set up', skipped: 'skipped', pending: 'scheduled' };

const parseDays = (text) => text.split(/[\s,]+/).filter(Boolean).map(Number);

function ReminderForm({ reminder, assets, onClose, onSaved }) {
  const editing = Boolean(reminder?.id);
  const [form, setForm] = useState({
    kind: reminder?.kind ?? 'warranty',
    title: reminder?.title ?? '',
    notes: reminder?.notes ?? '',
    assetId: reminder?.assetId ?? '',
    follow: reminder ? Boolean(reminder.dateSource) : true,
    dueOn: reminder?.dueOn ?? '',
    offsets: (reminder?.offsets ?? [7, 1]).join(', '),
    repeat: reminder?.repeat ?? 'none',
    repeatEvery: String(reminder?.repeatEvery ?? 1),
    repeatUntil: reminder?.repeatUntil ?? '',
  });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  const source = SOURCE_FOR_KIND[form.kind];
  const follows = Boolean(form.assetId && source && form.follow);

  async function submit(e) {
    e.preventDefault();
    const body = {
      title: form.title || null, notes: form.notes || null, offsets: parseDays(form.offsets),
      repeat: form.repeat, repeatEvery: Number(form.repeatEvery) || 1, repeatUntil: form.repeatUntil || null,
    };
    if (editing) {
      // An edit keeps what the reminder follows; a fixed date can be moved.
      if (!body.title) delete body.title;
      if (!reminder.dateSource) body.dueOn = form.dueOn;
    } else {
      Object.assign(body, { kind: form.kind, assetId: form.assetId || null });
      if (follows) body.dateSource = source; else body.dueOn = form.dueOn;
    }
    try {
      onSaved(await api(editing ? `/reminders/${reminder.id}` : '/reminders', { method: editing ? 'PATCH' : 'POST', body }));
    } catch (err) { setError(err); }
  }

  return (
    <Modal title={editing ? 'Edit reminder' : 'New reminder'} subtitle="Only you see your reminders. They arrive by the channels in Settings → Notifications." onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="What for">
            <select value={form.kind} onChange={set('kind')} disabled={editing}>
              {Object.entries(REMINDER_KINDS).map(([k, l]) => <option key={k} value={k}>{l}</option>)}
            </select>
          </Field>
          <Field label="Item">
            <select value={form.assetId} onChange={set('assetId')} disabled={editing}>
              <option value="">No item</option>
              {assets.map((a) => <option key={a.id} value={a.id}>{a.assetTag} · {a.name}</option>)}
            </select>
          </Field>
          <Field label="Title" wide><input value={form.title} onChange={set('title')} maxLength={160} placeholder="Leave empty for a title from the item and kind" /></Field>
          {!editing && form.assetId && source && (
            <label className="check span-2">
              <input type="checkbox" checked={form.follow} onChange={set('follow')} />
              Follow {DATE_SOURCES[source].toLowerCase().replace('its ', 'the item’s ')}, so the reminder moves when that date changes
            </label>
          )}
          {!(editing ? reminder.dateSource : follows) && <Field label="Date"><input type="date" required value={form.dueOn} onChange={set('dueOn')} /></Field>}
          <Field label="Remind me (days before)"><input required value={form.offsets} onChange={set('offsets')} inputMode="numeric" placeholder="e.g. 30, 7, 1, 0" /></Field>
          <Field label="Repeat">
            <select value={form.repeat} onChange={set('repeat')}>{Object.entries(REPEATS).map(([k, l]) => <option key={k} value={k}>{l}</option>)}</select>
          </Field>
          {form.repeat !== 'none' && (
            <>
              <Field label="Every how many"><input type="number" min={1} max={365} value={form.repeatEvery} onChange={set('repeatEvery')} /></Field>
              <Field label="Until (optional)"><input type="date" value={form.repeatUntil} onChange={set('repeatUntil')} /></Field>
            </>
          )}
          <Field label="Notes" wide><textarea rows={2} maxLength={1000} value={form.notes} onChange={set('notes')} /></Field>
        </div>
        <p className="hint">0 means on the day itself. Up to 8 reminders per date.</p>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary">{editing ? 'Save' : <><Plus size={16} /> Add reminder</>}</button>
        </div>
      </form>
    </Modal>
  );
}

function ReminderDetail({ id, onClose, onEdit, onChanged }) {
  const [r, setR] = useState(null);
  const [error, setError] = useState(null);
  const load = useCallback(() => api(`/reminders/${id}`).then(setR).catch(setError), [id]);
  useEffect(() => { load(); }, [load]);
  if (!r) return <Modal title="Reminder" onClose={onClose}><ErrorBanner error={error} /><p className="muted">Loading…</p></Modal>;
  const jobs = r.history ?? [];
  return (
    <Modal title={r.title} subtitle={`${REMINDER_KINDS[r.kind]}${r.assetName ? ` · ${r.assetTag} ${r.assetName}` : ''}`} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <div className="wo-grid">
        <div><span className="rel-label">Status</span><Badge map={STATUS} value={r.status} /></div>
        <div><span className="rel-label">Date</span><strong>{fmtDate(r.dueOn)}{r.dateSource && ' (follows the item)'}</strong></div>
        <div><span className="rel-label">Days before</span><strong>{r.offsets.join(', ')}</strong></div>
        <div><span className="rel-label">Repeat</span><strong>{REPEATS[r.repeat]}{r.repeat !== 'none' && r.repeatEvery > 1 ? ` (×${r.repeatEvery})` : ''}</strong></div>
        <div><span className="rel-label">Snoozed until</span><strong>{fmtDateTime(r.snoozedUntil)}</strong></div>
        <div><span className="rel-label">Last sent</span><strong>{fmtDateTime(r.lastNotifiedAt)}</strong></div>
      </div>
      {r.notes && <p className="pre wo-desc">{r.notes}</p>}
      <h3 className="section-title">Notifications for this reminder</h3>
      <ol className="timeline">
        {jobs.length === 0 && <li><strong className="muted-strong">Nothing sent yet.</strong><small>Notifications are scheduled by the background job and listed here once planned, with each channel’s delivery.</small></li>}
        {jobs.map((j) => (
          <li key={j.id}>
            <strong className="normal">{fmtDateTime(j.fireAt)} · {j.daysBefore === 0 ? 'on the day' : `${j.daysBefore} days before ${fmtDate(j.dueOn)}`} · {j.status}</strong>
            <small>
              {j.error ? `Error: ${j.error} · ` : ''}{(j.deliveries ?? []).length === 0 ? 'Not delivered yet'
                : j.deliveries.map((d) => `${d.channel}: ${DELIVERY[d.status] ?? d.status}${d.error ? ` (${d.error})` : ''}`).join(' · ')}
            </small>
          </li>
        ))}
      </ol>
      <div className="modal-actions">
        <button className="outline" onClick={() => onEdit(r)}><Pencil size={15} /> Edit</button>
        <button className="outline" onClick={async () => {
          if (!window.confirm('Delete this reminder? Notifications already sent stay in your list.')) return;
          try { await api(`/reminders/${r.id}`, { method: 'DELETE' }); onChanged(); onClose(); } catch (err) { setError(err); }
        }}><Trash2 size={15} /> Delete</button>
      </div>
    </Modal>
  );
}

export function Reminders({ assets }) {
  const [filter, setFilter] = useState('active');
  const [items, setItems] = useState(null);
  const [error, setError] = useState(null);
  const [form, setForm] = useState(null);
  const [openId, setOpenId] = useState(null);

  const load = useCallback(() => api(`/reminders${query({ status: filter })}`).then((r) => { setItems(r.items); setError(null); }).catch(setError), [filter]);
  useEffect(() => { load(); }, [load]);

  async function act(path, body, method = 'POST') {
    try { await api(path, { method, body }); load(); } catch (err) { setError(err); }
  }

  const active = items?.filter((r) => r.status === 'active') ?? [];
  return (
    <>
      <section className="panel">
        <div className="panel-head wrap">
          <div><h2>Your reminders</h2><p>Warranty, return, maintenance and renewal dates you want to hear about, on your own schedule.</p></div>
          <button className="primary" onClick={() => setForm({})}><Plus size={16} /> New reminder</button>
        </div>
        <div className="chips" style={{ paddingTop: 0 }}>
          {FILTERS.map(([key, label]) => <button key={key} className={filter === key ? 'chip on' : 'chip'} onClick={() => setFilter(key)}>{label}</button>)}
        </div>
        <ErrorBanner error={error} />
        <div className="table-scroll">
          <table>
            <thead><tr><th>REMINDER</th><th>DATE</th><th>DAYS BEFORE</th><th>NEXT</th><th>STATUS</th><th /></tr></thead>
            <tbody>
              {items === null && <tr><td colSpan="6" className="empty">Loading…</td></tr>}
              {items?.length === 0 && <tr><td colSpan="6" className="empty">No reminders here. Add one for a warranty, a return window or anything with a date.</td></tr>}
              {items?.map((r) => (
                <tr key={r.id} className="clickable" tabIndex={0} onClick={() => setOpenId(r.id)} onKeyDown={(e) => e.key === 'Enter' && setOpenId(r.id)}>
                  <td><strong className="normal">{r.title}</strong><small>{REMINDER_KINDS[r.kind]}{r.assetTag && ` · ${r.assetTag}`}{r.repeat !== 'none' && ` · ${REPEATS[r.repeat].toLowerCase()}`}</small></td>
                  <td className={r.daysLeft != null && r.daysLeft < 0 && r.status === 'active' ? 'overdue' : ''}>
                    {fmtDate(r.dueOn)}{r.daysLeft != null && r.status === 'active' && <small>{r.daysLeft < 0 ? `${-r.daysLeft} days ago` : r.daysLeft === 0 ? 'today' : `in ${r.daysLeft} days`}</small>}
                  </td>
                  <td>{r.offsets.join(', ')}</td>
                  <td>{r.snoozedUntil && new Date(r.snoozedUntil) > new Date() ? <>Snoozed<small>until {fmtDateTime(r.snoozedUntil)}</small></> : fmtDateTime(r.nextNotificationAt)}</td>
                  <td><Badge map={STATUS} value={r.status} /></td>
                  <td onClick={(e) => e.stopPropagation()}>
                    <span className="row-end">
                      {r.status === 'active' && (
                        <>
                          <button className="link-btn" title="Snooze one day" aria-label={`Snooze ${r.title} one day`} onClick={() => act(`/reminders/${r.id}/snooze`, { hours: 24 })}><AlarmClock size={16} /></button>
                          <button className="link-btn" title="Pause" aria-label={`Pause ${r.title}`} onClick={() => act(`/reminders/${r.id}`, { status: 'paused' }, 'PATCH')}><Pause size={16} /></button>
                          <button className="link-btn" title="Mark done" aria-label={`Mark ${r.title} done`} onClick={() => act(`/reminders/${r.id}/complete`)}><CheckCircle2 size={16} /></button>
                        </>
                      )}
                      {r.status === 'paused' && <button className="link-btn" title="Resume" aria-label={`Resume ${r.title}`} onClick={() => act(`/reminders/${r.id}`, { status: 'active' }, 'PATCH')}><Play size={16} /></button>}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      {filter === 'active' && items && active.length > 0 && (
        <p className="hint"><BellRing size={13} /> Workspace reminders (from each item’s dates) still arrive too, unless you set your own reminder for the same item and date.</p>
      )}
      {form && (
        <ReminderForm reminder={form.id ? form : null} assets={assets} onClose={() => setForm(null)}
          onSaved={() => { setForm(null); load(); }} />
      )}
      {openId && !form && <ReminderDetail id={openId} onClose={() => setOpenId(null)} onEdit={(r) => { setOpenId(null); setForm(r); }} onChanged={load} />}
    </>
  );
}
