import React, { useCallback, useEffect, useState } from 'react';
import { AlertTriangle, CalendarPlus, ClipboardCheck, Download, Handshake, PackageCheck, Plus, Undo2 } from 'lucide-react';
import { api, query } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { CONDITIONS } from '../constants.js';
import { addDays, fmtDate, fmtDateTime } from '../format.js';
import { Badge } from './Maintenance.jsx';

const BASE = import.meta.env.VITE_API_URL ?? '';
export const LOAN_STATUS = {
  reserved: ['Reserved', 'available'], deployed: ['Out', 'deployed'], returned: ['Awaiting inspection', 'maintenance'],
  inspected: ['Completed', 'retired'], cancelled: ['Cancelled', 'retired'], lost: ['Lost', 'missing'],
};
const FILTERS = [['open', 'Open'], ['overdue', 'Overdue'], ['reserved', 'Reserved'], ['returned', 'Awaiting inspection'], ['inspected', 'Completed'], ['lost', 'Lost'], ['', 'All']];
const EVENT_LABEL = {
  reserved: 'Reserved', deployed: 'Handed out', extended: 'Extended', returned: 'Returned', inspected: 'Inspected', cancelled: 'Cancelled', lost: 'Reported lost', overdue: 'Became overdue',
};

function usePeople() {
  const [list, setList] = useState([]);
  useEffect(() => { api('/people').then((r) => setList(r.items.filter((p) => p.isActive))).catch(() => {}); }, []);
  return list;
}

// Items free to lend: available and unassigned, in a loaner inventory or one inside it.
function useLoanerItems() {
  const [list, setList] = useState([]);
  useEffect(() => {
    api('/inventories').then(async (r) => {
      const pools = r.items.filter((i) => i.kind === 'loaner' && !i.archivedAt && !r.items.some((p) => p.id === i.parentId && p.kind === 'loaner'));
      const [open, ...lists] = await Promise.all([
        api('/loans?status=open&limit=500'),
        ...pools.map((p) => api(`/assets${query({ inventoryId: p.id, includeSub: 'true', status: 'Available', limit: 500 })}`)),
      ]);
      const out = new Set(open.items.map((l) => l.assetId));
      setList(lists.flatMap((l) => l.items).filter((a) => !a.assignedPersonId && !out.has(a.id)));
    }).catch(() => {});
  }, []);
  return list;
}

function NewLoan({ onClose, onCreated }) {
  const people = usePeople();
  const items = useLoanerItems();
  const [form, setForm] = useState({ assetId: '', code: '', personId: '', dueOn: addDays(7), purpose: '', deployNow: true, acknowledgedName: '' });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  async function submit(e) {
    e.preventDefault();
    const body = { personId: form.personId, dueOn: form.dueOn, purpose: form.purpose || null, deployNow: form.deployNow };
    if (form.assetId) body.assetId = form.assetId; else body.code = form.code.trim();
    if (form.deployNow && form.acknowledgedName) body.acknowledgedName = form.acknowledgedName;
    try { onCreated(await api('/loans', { method: 'POST', body })); } catch (err) { setError(err); }
  }
  return (
    <Modal title="Lend a loaner" subtitle="Only available items in a loaner inventory can be lent." onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Item">
            <select value={form.assetId} onChange={set('assetId')}>
              <option value="">Scan or type a tag instead</option>
              {items.map((a) => <option key={a.id} value={a.id}>{a.assetTag} · {a.name}</option>)}
            </select>
          </Field>
          {!form.assetId && <Field label="Asset tag, serial or barcode"><input required value={form.code} onChange={set('code')} autoComplete="off" /></Field>}
          <Field label="Borrower">
            <select required value={form.personId} onChange={set('personId')}>
              <option value="">Choose a person</option>
              {people.map((p) => <option key={p.id} value={p.id}>{p.name}{p.department ? ` · ${p.department}` : ''}</option>)}
            </select>
          </Field>
          <Field label="Due back"><input type="date" required value={form.dueOn} onChange={set('dueOn')} /></Field>
          <Field label="Purpose" wide><input maxLength={500} value={form.purpose} onChange={set('purpose')} placeholder="Travel, repair swap, new starter…" /></Field>
          {form.deployNow && <Field label="Borrower signs as (acknowledgment)" wide><input maxLength={120} value={form.acknowledgedName} onChange={set('acknowledgedName')} placeholder="Their full name, typed by them" /></Field>}
        </div>
        <label className="check"><input type="checkbox" checked={form.deployNow} onChange={set('deployNow')} /> Hand it over now (otherwise it is reserved for later)</label>
        {people.length === 0 && <p className="hint">Add the borrower under People first.</p>}
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><Handshake size={16} /> {form.deployNow ? 'Lend' : 'Reserve'}</button>
        </div>
      </form>
    </Modal>
  );
}

const ACTIONS = {
  deploy: { title: 'Hand over', icon: Handshake },
  extend: { title: 'Extend', icon: CalendarPlus },
  return: { title: 'Check in', icon: Undo2 },
  inspect: { title: 'Inspect', icon: ClipboardCheck },
  cancel: { title: 'Cancel reservation', icon: Undo2 },
  lost: { title: 'Report lost', icon: AlertTriangle },
};

function LoanAction({ loan, action, onDone, onBack }) {
  const [form, setForm] = useState({ dueOn: addDays(7), reason: '', condition: '', damaged: false, note: '', result: 'ready', acknowledgedName: '' });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  const bodies = {
    deploy: () => ({ acknowledgedName: form.acknowledgedName || null, condition: form.condition || null, note: form.note || null }),
    extend: () => ({ dueOn: form.dueOn, reason: form.reason }),
    return: () => ({ condition: form.condition || null, damaged: form.damaged, note: form.note || null }),
    inspect: () => ({ result: form.result, note: form.note || null }),
    cancel: () => ({ reason: form.note || null }),
    lost: () => ({ note: form.note }),
  };
  async function submit(e) {
    e.preventDefault();
    try { await api(`/loans/${loan.id}/${action}`, { method: 'POST', body: bodies[action]() }); onDone(); } catch (err) { setError(err); }
  }
  const noteRequired = action === 'lost' || (action === 'inspect' && form.result === 'repair') || (action === 'return' && form.damaged);
  return (
    <form className="assign-panel" onSubmit={submit}>
      <ErrorBanner error={error} />
      <div className="form-grid">
        {action === 'deploy' && <Field label="Borrower signs as"><input maxLength={120} value={form.acknowledgedName} onChange={set('acknowledgedName')} placeholder={loan.personName} /></Field>}
        {['deploy', 'return'].includes(action) && (
          <Field label="Condition">
            <select value={form.condition} onChange={set('condition')}><option value="">Not recorded</option>{CONDITIONS.map((c) => <option key={c}>{c}</option>)}</select>
          </Field>
        )}
        {action === 'extend' && <>
          <Field label="New due date"><input type="date" required min={loan.dueOn} value={form.dueOn} onChange={set('dueOn')} /></Field>
          <Field label="Reason"><input required maxLength={500} value={form.reason} onChange={set('reason')} /></Field>
        </>}
        {action === 'inspect' && (
          <Field label="Result">
            <select value={form.result} onChange={set('result')}><option value="ready">Ready to lend again</option><option value="repair">Needs repair</option></select>
          </Field>
        )}
        {action !== 'extend' && <Field label={action === 'lost' ? 'What happened' : 'Note'} wide><textarea rows={2} required={noteRequired} maxLength={1000} value={form.note} onChange={set('note')} /></Field>}
      </div>
      {action === 'return' && <label className="check"><input type="checkbox" checked={form.damaged} onChange={set('damaged')} /> Returned damaged (the item goes to maintenance)</label>}
      <div className="modal-actions">
        <button type="button" className="outline" onClick={onBack}>Back</button>
        <button className="primary">{ACTIONS[action].title}</button>
      </div>
    </form>
  );
}

function LoanDetail({ id, onClose, onChanged, onSelectAsset }) {
  const { can } = useAuth();
  const [loan, setLoan] = useState(null);
  const [error, setError] = useState(null);
  const [action, setAction] = useState(null);
  const load = useCallback(() => api(`/loans/${id}`).then(setLoan).catch(setError), [id]);
  useEffect(() => { load(); }, [load]);
  if (!loan) return <Modal title="Loan" onClose={onClose}><ErrorBanner error={error} /><p className="muted">Loading…</p></Modal>;
  const available = {
    reserved: ['deploy', 'cancel'], deployed: ['extend', 'return', 'lost'], returned: ['inspect'],
  }[loan.status] ?? [];
  const allowed = available.filter((a) => (a === 'lost' ? can('loans:resolve') : can('loans:manage')));
  return (
    <Modal title={`${loan.number} · ${loan.assetName}`} subtitle={`${loan.assetTag} · lent to ${loan.personName}`} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <div className="wo-grid">
        <div><span className="rel-label">Status</span><Badge map={LOAN_STATUS} value={loan.status} /></div>
        <div><span className="rel-label">Due back</span><strong className={loan.overdue ? 'overdue' : ''}>{fmtDate(loan.dueOn)}{loan.overdue && ` · ${loan.daysOverdue} days overdue`}</strong></div>
        <div><span className="rel-label">From</span><strong>{fmtDate(loan.startsOn)}</strong></div>
        <div><span className="rel-label">Signed by</span><strong>{loan.acknowledgedName ? `${loan.acknowledgedName} · ${fmtDateTime(loan.acknowledgedAt)}` : '—'}</strong></div>
        <div><span className="rel-label">Condition out / back</span><strong>{loan.checkoutCondition || '—'} / {loan.returnCondition || '—'}</strong></div>
        <div><span className="rel-label">Extensions</span><strong>{loan.extensions}</strong></div>
      </div>
      {loan.purpose && <p className="wo-desc">{loan.purpose}</p>}
      {loan.damageReported && <p className="wo-resolution"><strong>Returned damaged:</strong> {loan.returnNote}</p>}
      {loan.inspectionResult && <p className="wo-resolution"><strong>Inspection:</strong> {loan.inspectionResult === 'ready' ? 'ready to lend again' : 'sent for repair'}{loan.inspectionNote && ` · ${loan.inspectionNote}`}</p>}
      {!action && (
        <div className="modal-actions top">
          {allowed.map((a) => { const { title, icon: Icon } = ACTIONS[a]; return <button key={a} className={a === allowed[0] ? 'primary' : 'outline'} onClick={() => setAction(a)}><Icon size={15} /> {title}</button>; })}
          <button className="outline" onClick={() => { onClose(); onSelectAsset({ id: loan.assetId, name: loan.assetName, assetTag: loan.assetTag, type: '', status: '' }); }}>Open item</button>
        </div>
      )}
      {action && <LoanAction loan={loan} action={action} onBack={() => setAction(null)} onDone={() => { setAction(null); load(); onChanged(); }} />}
      <h3 className="section-title">History</h3>
      <ol className="timeline">
        {loan.events.map((e) => (
          <li key={e.id}>
            <strong className="normal">{EVENT_LABEL[e.kind] ?? e.kind}</strong>
            <small>{e.actorName || 'System'} · {fmtDateTime(e.createdAt)}{e.details?.reason && ` · ${e.details.reason}`}{e.details?.note && ` · ${e.details.note}`}{e.kind === 'extended' && e.details?.to && ` · now due ${fmtDate(e.details.to)}`}</small>
          </li>
        ))}
      </ol>
    </Modal>
  );
}

export function Loaners({ onSelect, onChanged }) {
  const { can } = useAuth();
  const [filter, setFilter] = useState('open');
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [creating, setCreating] = useState(false);
  const [openId, setOpenId] = useState(null);
  const params = filter === 'overdue' ? { overdue: 'true' } : { status: filter };
  const load = useCallback(() => api(`/loans${query({ ...params, limit: 200 })}`).then((d) => { setData(d); setError(null); }).catch(setError), [filter]);
  useEffect(() => { load(); }, [load]);
  const changed = () => { load(); onChanged(); };
  const s = data?.summary;
  return (
    <>
      <div className="metrics">
        {[['Out now', s?.deployed, Handshake, ''], ['Overdue', s?.overdue, AlertTriangle, 'bad'], ['Awaiting inspection', s?.awaitingInspection, PackageCheck, 'warn'], ['Reserved', s?.reserved, CalendarPlus, '']].map(([label, value, Icon, tone]) => (
          <div className={`metric ${tone}`} key={label}>
            <div className="metric-top"><span>{label}</span><span className="metric-icon"><Icon size={19} /></span></div>
            <strong>{value ?? '…'}</strong>
          </div>
        ))}
      </div>
      <section className="panel">
        <div className="panel-head wrap">
          <div><h2>Loans</h2><p>{s ? `${s.completedLast30Days} completed in the last 30 days${s.averageLoanDays != null ? ` · ${Number(s.averageLoanDays).toFixed(1)} days on average` : ''} · ${s.damagedReturns} damaged returns · ${s.lost} lost` : 'Reserve, hand over, check in and inspect loaner equipment.'}</p></div>
          <div className="row-end">
            <a className="outline button-link" href={`${BASE}/api/loans/export${query(params)}`}><Download size={15} /> CSV</a>
            {can('loans:manage') && <button className="primary" onClick={() => setCreating(true)}><Plus size={16} /> Lend</button>}
          </div>
        </div>
        <div className="chips" style={{ paddingTop: 0 }}>
          {FILTERS.map(([k, l]) => <button key={k} className={filter === k ? 'chip on' : 'chip'} onClick={() => setFilter(k)}>{l}</button>)}
        </div>
        <ErrorBanner error={error} />
        <div className="table-scroll">
          <table>
            <thead><tr><th>LOAN</th><th>BORROWER</th><th>DUE BACK</th><th>STATUS</th></tr></thead>
            <tbody>
              {data === null && <tr><td colSpan="4" className="empty">Loading…</td></tr>}
              {data?.items.length === 0 && <tr><td colSpan="4" className="empty">No loans here.</td></tr>}
              {data?.items.map((l) => (
                <tr key={l.id} className="clickable" tabIndex={0} onClick={() => setOpenId(l.id)} onKeyDown={(e) => e.key === 'Enter' && setOpenId(l.id)}>
                  <td><strong className="normal">{l.assetName}</strong><small>{l.number} · {l.assetTag}</small></td>
                  <td>{l.personName}{l.purpose && <small>{l.purpose}</small>}</td>
                  <td className={l.overdue ? 'overdue' : ''}>{fmtDate(l.dueOn)}{l.overdue && <small>{l.daysOverdue} days overdue</small>}</td>
                  <td><Badge map={LOAN_STATUS} value={l.status} />{l.damageReported && <small>damaged</small>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      {creating && <NewLoan onClose={() => setCreating(false)} onCreated={(l) => { setCreating(false); setOpenId(l.id); changed(); }} />}
      {openId && <LoanDetail id={openId} onClose={() => setOpenId(null)} onChanged={changed} onSelectAsset={onSelect} />}
    </>
  );
}
