import React, { useCallback, useEffect, useState } from 'react';
import { Check, ListPlus, X } from 'lucide-react';
import { api, query } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { fmtDate, fmtDateTime } from '../format.js';
import { AUDIT_TYPES } from './Audits.jsx';
import { Badge } from './Maintenance.jsx';

const TASK_STATUS = { open: ['Open', 'available'], done: ['Done', 'deployed'], cancelled: ['Cancelled', 'retired'] };
// Mirrors KINDS in api/src/routes/approvals.rs.
const TASK_KINDS = {
  general: 'General', missing: 'Missing item', damaged: 'Damaged item', discrepancy: 'Discrepancy', overdue_loan: 'Overdue loan',
  warranty: 'Warranty', maintenance: 'Maintenance', audit: 'Audit', transfer: 'Transfer',
};
const SECTIONS = {
  transfers: 'Transfers waiting for approval', audits: 'Audits waiting for review', accessRequests: 'Access requests', overdueLoans: 'Overdue loans',
  loansToInspect: 'Loans to inspect', missing: 'Missing after the last audit', discrepancies: 'Discrepancies (last 90 days)',
  damaged: 'Damaged (last 90 days)', warranties: 'Warranties ending', maintenance: 'Maintenance due this week',
};

const asset = (r) => ({ id: r.assetId, name: r.assetName, assetTag: r.assetTag, type: '', status: '' });

function TaskForm({ task, seed, onClose, onSaved }) {
  const [staff, setStaff] = useState([]);
  useEffect(() => { api('/work-orders/assignees').then((r) => setStaff(r.items)).catch(() => {}); }, []);
  const editing = Boolean(task?.id);
  const [form, setForm] = useState({
    title: task?.title ?? seed?.title ?? '', kind: task?.kind ?? seed?.kind ?? 'general', notes: task?.notes ?? '',
    assigneeId: task?.assigneeId ?? '', dueOn: task?.dueOn ?? '',
  });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.value });
  async function submit(e) {
    e.preventDefault();
    const body = { title: form.title, notes: form.notes || null, assigneeId: form.assigneeId || null, dueOn: form.dueOn || null };
    if (!editing) {
      Object.assign(body, { kind: form.kind, assetId: seed?.assetId ?? null });
      if (seed?.sourceType) Object.assign(body, { sourceType: seed.sourceType, sourceId: seed.sourceId });
    }
    try { onSaved(await api(editing ? `/tasks/${task.id}` : '/tasks', { method: editing ? 'PATCH' : 'POST', body })); } catch (err) { setError(err); }
  }
  return (
    <Modal title={editing ? 'Edit follow-up' : 'New follow-up task'} subtitle={seed?.assetTag ? `${seed.assetTag} · ${seed.assetName}` : 'Someone to look into it, by a date.'} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="What to do" wide><input required maxLength={200} value={form.title} onChange={set('title')} /></Field>
          <Field label="Kind">
            <select value={form.kind} onChange={set('kind')} disabled={editing}>{Object.entries(TASK_KINDS).map(([k, l]) => <option key={k} value={k}>{l}</option>)}</select>
          </Field>
          <Field label="Assign to">
            <select value={form.assigneeId} onChange={set('assigneeId')}>
              <option value="">Unassigned</option>
              {staff.map((u) => <option key={u.id} value={u.id}>{u.name}</option>)}
            </select>
          </Field>
          <Field label="Due"><input type="date" value={form.dueOn} onChange={set('dueOn')} /></Field>
          <Field label="Notes" wide><textarea rows={3} maxLength={2000} value={form.notes} onChange={set('notes')} /></Field>
        </div>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary">{editing ? 'Save' : 'Create task'}</button>
        </div>
      </form>
    </Modal>
  );
}

function Tasks({ version, onSelect }) {
  const { can, user } = useAuth();
  const [filter, setFilter] = useState('open');
  const [mine, setMine] = useState(false);
  const [items, setItems] = useState(null);
  const [error, setError] = useState(null);
  const [editing, setEditing] = useState(null);
  const load = useCallback(() => api(`/tasks${query({ status: filter === 'all' ? '' : filter, mine: mine ? 'true' : '' })}`)
    .then((r) => { setItems(r.items); setError(null); }).catch(setError), [filter, mine]);
  useEffect(() => { load(); }, [load, version]);
  async function close(t, status) {
    const resolution = window.prompt(status === 'done' ? 'What was done?' : 'Why cancel? (optional)');
    if (resolution === null || (status === 'done' && !resolution)) return;
    try { await api(`/tasks/${t.id}`, { method: 'PATCH', body: { status, resolution: resolution || null } }); load(); } catch (err) { setError(err); }
  }
  return (
    <section className="panel">
      <div className="panel-head wrap">
        <div><h2>Follow-up tasks</h2><p>Who is looking into what, and by when.</p></div>
        {can('tasks:manage') && <button className="outline" onClick={() => setEditing({})}><ListPlus size={16} /> New task</button>}
      </div>
      <div className="chips" style={{ paddingTop: 0 }}>
        {[['open', 'Open'], ['done', 'Done'], ['cancelled', 'Cancelled'], ['all', 'All']].map(([k, l]) => <button key={k} className={filter === k ? 'chip on' : 'chip'} onClick={() => setFilter(k)}>{l}</button>)}
        <button className={mine ? 'chip on' : 'chip'} onClick={() => setMine(!mine)}>Assigned to me</button>
      </div>
      <ErrorBanner error={error} />
      <div className="table-scroll">
        <table>
          <thead><tr><th>TASK</th><th>ITEM</th><th>ASSIGNED</th><th>DUE</th><th>STATUS</th><th /></tr></thead>
          <tbody>
            {items === null && <tr><td colSpan="6" className="empty">Loading…</td></tr>}
            {items?.length === 0 && <tr><td colSpan="6" className="empty">No tasks here.</td></tr>}
            {items?.map((t) => {
              const overdue = t.status === 'open' && t.dueOn && t.dueOn < new Date().toLocaleDateString('en-CA');
              const closable = t.status === 'open' && (can('tasks:manage') || t.assigneeId === user.id);
              return (
                <tr key={t.id}>
                  <td><strong className="normal">{t.title}</strong><small>{TASK_KINDS[t.kind]} · by {t.createdBy}{t.notes && ` · ${t.notes}`}</small></td>
                  <td>{t.assetId ? <button className="link-like" onClick={() => onSelect(asset(t))}>{t.assetTag}<small>{t.assetName}</small></button> : <span className="muted">—</span>}</td>
                  <td>{t.assigneeName || <span className="muted">Unassigned</span>}</td>
                  <td className={overdue ? 'overdue' : ''}>{fmtDate(t.dueOn)}</td>
                  <td><Badge map={TASK_STATUS} value={t.status} />{t.resolution && <small>{t.resolution}</small>}</td>
                  <td>
                    <span className="row-end">
                      {closable && <button className="link-btn" title="Done" aria-label={`Mark ${t.title} done`} onClick={() => close(t, 'done')}><Check size={17} /></button>}
                      {t.status === 'open' && can('tasks:manage') && <>
                        <button className="link-btn" title="Cancel" aria-label={`Cancel ${t.title}`} onClick={() => close(t, 'cancelled')}><X size={17} /></button>
                        <button className="text-link" onClick={() => setEditing(t)}>Edit</button>
                      </>}
                    </span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {editing && <TaskForm task={editing.id ? editing : null} onClose={() => setEditing(null)} onSaved={() => { setEditing(null); load(); }} />}
    </section>
  );
}

function Section({ name, rows, children }) {
  if (!rows || rows.length === 0) return null;
  return (
    <section className="panel">
      <div className="panel-head"><div><h2>{SECTIONS[name]} <span className="count">{rows.length}</span></h2></div></div>
      <ul className="queue-list">{children}</ul>
    </section>
  );
}

export function Approvals({ onSelect, onChanged, go }) {
  const { can } = useAuth();
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [seed, setSeed] = useState(null);
  const [version, setVersion] = useState(0);
  const load = useCallback(() => api('/approvals').then((d) => { setData(d); setError(null); }).catch(setError), []);
  useEffect(() => { load(); }, [load]);
  const changed = () => { load(); setVersion((v) => v + 1); onChanged(); };
  async function post(path, body) { try { await api(path, { method: 'POST', body }); changed(); } catch (err) { setError(err); } }
  const reject = (path, field, question) => { const v = window.prompt(question); if (v) post(path, { [field]: v }); };
  const S = data?.sections ?? {};
  const followUp = (r, kind, title, sourceType, sourceId) => (can('tasks:manage') && (r.hasTask
    ? <span className="tag">Task open</span>
    : <button className="text-link" onClick={() => setSeed({ kind, title, assetId: r.assetId, assetTag: r.assetTag, assetName: r.assetName, sourceType, sourceId })}><ListPlus size={13} /> Follow up</button>));
  const Item = ({ r, children, meta }) => (
    <li>
      <span>
        {r.assetId ? <button className="link-like" onClick={() => onSelect(asset(r))}><strong className="normal">{r.assetTag} · {r.assetName}</strong></button> : null}
        {children}
        {meta && <small>{meta}</small>}
      </span>
    </li>
  );
  const total = data ? Object.values(data.counts).reduce((a, b) => a + b, 0) : 0;
  return (
    <>
      <div className="metrics">
        {[['Waiting on you', data ? (data.counts.transfers ?? 0) + (data.counts.audits ?? 0) + (data.counts.accessRequests ?? 0) : null, ''],
          ['Loans needing action', data ? (data.counts.overdueLoans ?? 0) + (data.counts.loansToInspect ?? 0) : null, 'bad'],
          ['Exceptions', data ? (data.counts.missing ?? 0) + (data.counts.discrepancies ?? 0) + (data.counts.damaged ?? 0) : null, 'warn'],
          ['Open follow-ups', data?.openTasks, '']].map(([label, value, tone]) => (
          <div className={`metric ${tone}`} key={label}><div className="metric-top"><span>{label}</span></div><strong>{value ?? '…'}</strong></div>
        ))}
      </div>
      <ErrorBanner error={error} />
      {data && total === 0 && <section className="panel"><p className="empty">Nothing needs a decision right now.</p></section>}
      {data && total > 0 && Object.keys(S).some((k) => S[k].length === 0) && (
        <p className="hint all-clear">All clear: {Object.keys(S).filter((k) => S[k].length === 0).map((k) => SECTIONS[k].replace(/ \(.*\)$/, '').toLowerCase()).join(', ')}.</p>
      )}
      <div className="queue-grid">
        <Section name="transfers" rows={S.transfers}>
          {S.transfers?.map((r) => (
            <Item key={r.id} r={r} meta={`${r.fromName || 'No inventory'} → ${r.toName} · ${r.requestedBy} · ${fmtDateTime(r.requestedAt)}${r.reason ? ` · ${r.reason}` : ''}`}>
              <span className="row-end">
                {!r.mine && <button className="outline q-btn" onClick={() => post(`/transfers/${r.id}/approve`, {})}><Check size={14} /> Approve</button>}
                {!r.mine && <button className="outline q-btn" onClick={() => reject(`/transfers/${r.id}/reject`, 'reason', 'Why reject this transfer?')}><X size={14} /> Reject</button>}
                {r.mine && <span className="muted small">Your request, for someone else to approve</span>}
              </span>
            </Item>
          ))}
        </Section>
        <Section name="audits" rows={S.audits}>
          {S.audits?.map((r) => (
            <Item key={r.id} r={r} meta={`${AUDIT_TYPES[r.auditType] ?? 'Audit'}${r.inventoryName ? ` · ${r.inventoryName}` : ''} · submitted by ${r.submittedBy} ${fmtDateTime(r.submittedAt)}`}>
              <strong className="normal">{r.name}</strong>
              <span className="row-end">
                <button className="outline q-btn" onClick={() => go('Audits')}>Open</button>
                {r.canReview ? <>
                  <button className="outline q-btn" onClick={() => post(`/audits/${r.id}/approve`, {})}><Check size={14} /> Approve</button>
                  <button className="outline q-btn" onClick={() => reject(`/audits/${r.id}/reject`, 'note', 'What needs another look?')}><X size={14} /> Send back</button>
                </> : <span className="muted small">You started or submitted it</span>}
              </span>
            </Item>
          ))}
        </Section>
        <Section name="accessRequests" rows={S.accessRequests}>
          {S.accessRequests?.map((r) => (
            <Item key={r.id} r={r} meta={`${fmtDateTime(r.createdAt)}${r.note ? ` · “${r.note}”` : ''}`}>
              <strong className="normal">{r.userName} asks for {r.inventoryName}</strong>
              <span className="row-end">
                <button className="outline q-btn" onClick={() => post(`/access-requests/${r.id}/approve`, {})}><Check size={14} /> Approve</button>
                <button className="outline q-btn" onClick={() => post(`/access-requests/${r.id}/reject`, {})}><X size={14} /> Reject</button>
              </span>
            </Item>
          ))}
        </Section>
        <Section name="overdueLoans" rows={S.overdueLoans}>
          {S.overdueLoans?.map((r) => (
            <Item key={r.id} r={r} meta={`${r.number} · ${r.personName} · due ${fmtDate(r.dueOn)} · ${r.daysOverdue} days overdue`}>
              <span className="row-end"><button className="outline q-btn" onClick={() => go('Loaners')}>Loaners</button>{followUp(r, 'overdue_loan', `Get ${r.assetTag} back from ${r.personName}`, 'loan', r.id)}</span>
            </Item>
          ))}
        </Section>
        <Section name="loansToInspect" rows={S.loansToInspect}>
          {S.loansToInspect?.map((r) => (
            <Item key={r.id} r={r} meta={`${r.number} · returned ${fmtDateTime(r.returnedAt)} by ${r.personName}${r.damageReported ? ` · damaged: ${r.returnNote}` : ''}`}>
              <span className="row-end"><button className="outline q-btn" onClick={() => go('Loaners')}>Inspect</button></span>
            </Item>
          ))}
        </Section>
        <Section name="missing" rows={S.missing}>
          {S.missing?.map((r) => (
            <Item key={`${r.assetId}`} r={r} meta={`${r.auditName} · since ${fmtDate(r.since)}`}>
              <span className="row-end">{followUp(r, 'missing', `Find ${r.assetTag}`, 'audit', r.auditId)}</span>
            </Item>
          ))}
        </Section>
        <Section name="discrepancies" rows={S.discrepancies}>
          {S.discrepancies?.map((r) => (
            <Item key={`${r.auditId}-${r.assetId}`} r={r} meta={`${r.auditName} · expected ${r.expectedLocation || 'elsewhere'}, found ${r.foundLocation || 'elsewhere'}`}>
              <span className="row-end">{followUp(r, 'discrepancy', `Put ${r.assetTag} back where it belongs`, 'audit', r.auditId)}</span>
            </Item>
          ))}
        </Section>
        <Section name="damaged" rows={S.damaged}>
          {S.damaged?.map((r, n) => (
            <Item key={`${r.assetId}-${n}`} r={r} meta={`${r.source === 'audit' ? 'Audit' : 'Loan return'}: ${r.sourceName} · ${fmtDate(r.at)}${r.note ? ` · ${r.note}` : ''}`}>
              <span className="row-end">{followUp(r, 'damaged', `Repair or replace ${r.assetTag}`)}</span>
            </Item>
          ))}
        </Section>
        <Section name="warranties" rows={S.warranties}>
          {S.warranties?.map((r) => (
            <Item key={r.id} r={r} meta={`${r.provider || r.kind} · ${r.daysLeft < 0 ? `ended ${fmtDate(r.endsOn)}` : `ends ${fmtDate(r.endsOn)} (${r.daysLeft} days)`}`}>
              <span className="row-end">{followUp(r, 'warranty', `Renew or replace the warranty for ${r.assetTag}`, 'warranty', r.id)}</span>
            </Item>
          ))}
        </Section>
        <Section name="maintenance" rows={S.maintenance}>
          {S.maintenance?.map((r) => (
            <Item key={`${r.source}-${r.id}`} r={r} meta={`${r.number ? `${r.number} · ` : ''}${r.title} · due ${fmtDate(r.dueOn)}`}>
              <span className="row-end">
                <button className="outline q-btn" onClick={() => go('Maintenance')}>Maintenance</button>
                {followUp(r, 'maintenance', `Service ${r.assetTag}`, r.source === 'work_order' ? 'work_order' : undefined, r.source === 'work_order' ? r.id : undefined)}
              </span>
            </Item>
          ))}
        </Section>
      </div>
      {can('tasks:read') && <Tasks version={version} onSelect={onSelect} />}
      {seed && <TaskForm seed={seed} onClose={() => setSeed(null)} onSaved={() => { setSeed(null); changed(); }} />}
    </>
  );
}
