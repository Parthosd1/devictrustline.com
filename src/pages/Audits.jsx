import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertOctagon, Check, CheckCircle2, ClipboardCheck, Download, Lock, MapPinOff, Play, Plus, RotateCcw, ScanLine, Send, X, XCircle } from 'lucide-react';
import { api, query } from '../api.js';
import { useAuth } from '../auth.jsx';
import { CameraScanner, useDedupe } from '../camera.jsx';
import { AssetIcon, ErrorBanner } from '../components.jsx';
import { fmtDateTime } from '../format.js';
import { InventoryOptions, useInventories } from './Inventories.jsx';
import { Badge } from './Maintenance.jsx';

const BASE = import.meta.env.VITE_API_URL ?? '';

const PERIODS = [['weekly', 'Weekly'], ['monthly', 'Monthly'], ['quarterly', 'Quarterly'], ['annual', 'Annual'], ['manual', 'One-off']];
// Mirrors TYPES in api/src/routes/audits.rs. Soft audits close without review; the others need one.
export const AUDIT_TYPES = {
  soft: 'Soft count (no review)', full: 'Full hub audit', spot: 'Spot check', transfer: 'Transfer audit', return: 'Returned equipment',
};
export const AUDIT_STATUS = {
  draft: ['Draft', 'retired'], open: ['In progress', 'available'], submitted: ['Waiting for review', 'maintenance'],
  approved: ['Approved', 'deployed'], rejected: ['Sent back', 'missing'], closed: ['Closed', 'deployed'],
};
const ACTIVE = ['draft', 'open', 'submitted', 'rejected'];
const REVIEW_LABEL = { started: 'Started', submitted: 'Submitted for review', approved: 'Approved', rejected: 'Sent back', reopened: 'Reopened', closed: 'Closed' };
const RESULT = {
  pending: ['Not checked', 'retired'],
  verified: ['Verified', 'deployed'],
  wrong_location: ['Wrong location', 'maintenance'],
  missing: ['Missing', 'missing'],
};
const FILTERS = [['all', 'All'], ['pending', 'Not checked'], ['verified', 'Verified'], ['wrong_location', 'Wrong location'], ['missing', 'Missing'], ['unexpected', 'Unexpected']];

const formatDate = (iso) => new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
const scopeLabel = (run) => run.inventoryName || (run.locationName ? `${run.buildingName} / ${run.locationName}` : run.buildingName || 'All buildings');
export const progressOf = (run) => {
  const c = run.summary || run.counts;
  const found = c.verified + c.wrongLocation;
  return { found, expected: c.expected, pct: c.expected ? Math.round((found / c.expected) * 100) : 0 };
};

function StartAudit({ period, buildings, onStarted, onCancel }) {
  const { list: inventories } = useInventories();
  const [form, setForm] = useState({ auditType: period === 'manual' ? 'spot' : 'full', inventoryId: '', buildingId: '', locationId: '', draft: false });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  const locations = buildings.find((b) => b.id === form.buildingId)?.locations ?? [];
  async function start(e) {
    e.preventDefault();
    try {
      onStarted(await api('/audits', {
        method: 'POST',
        body: {
          period, auditType: form.auditType, draft: form.draft, inventoryId: form.inventoryId || null,
          buildingId: form.inventoryId ? null : form.buildingId || null, locationId: form.inventoryId ? null : form.locationId || null,
        },
      }));
    } catch (err) { setError(err); }
  }
  return (
    <section className="panel start-audit">
      <div className="panel-head"><div>
        <h2>Start {period === 'manual' ? 'a one-off' : `a ${period}`} audit</h2>
        <p>Everything currently in the chosen inventory or place is expected. {form.auditType === 'soft' ? 'A soft count closes without review.' : 'When finished it is submitted to a manager, who approves it or sends it back. Nobody approves their own audit.'}</p>
      </div></div>
      <ErrorBanner error={error} />
      <form className="filters" onSubmit={start}>
        <select value={form.auditType} onChange={set('auditType')} aria-label="Audit type">
          {Object.entries(AUDIT_TYPES).map(([k, l]) => <option key={k} value={k}>{l}</option>)}
        </select>
        <select value={form.inventoryId} onChange={set('inventoryId')} aria-label="Inventory to audit" required={form.auditType === 'spot'}>
          <option value="">{form.auditType === 'return' ? 'Returned equipment (default)' : form.auditType === 'spot' ? 'Choose the inventory to spot-check' : 'By place instead of inventory'}</option>
          <InventoryOptions list={inventories} />
        </select>
        {!form.inventoryId && !['return', 'spot'].includes(form.auditType) && <>
          <select value={form.buildingId} onChange={(e) => setForm({ ...form, buildingId: e.target.value, locationId: '' })} aria-label="Building to audit">
            <option value="">All buildings</option>
            {buildings.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
          </select>
          <select value={form.locationId} onChange={set('locationId')} disabled={!form.buildingId} aria-label="Location to audit">
            <option value="">Whole building</option>
            {locations.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
          </select>
        </>}
        <label className="check"><input type="checkbox" checked={form.draft} onChange={set('draft')} /> Save as draft</label>
        <button className="primary"><ClipboardCheck size={16} /> {form.draft ? 'Create draft' : 'Start audit'}</button>
        {onCancel && <button type="button" className="outline" onClick={onCancel}>Cancel</button>}
      </form>
    </section>
  );
}

function RunView({ runId, buildings, onSelectAsset, onChanged }) {
  const { can } = useAuth();
  const [run, setRun] = useState(null);
  const [error, setError] = useState(null);
  const [code, setCode] = useState('');
  const [hereId, setHereId] = useState('');
  const [filter, setFilter] = useState('all');
  const [flash, setFlash] = useState(null);
  const inputRef = useRef(null);
  const isNew = useDedupe();

  const load = useCallback(() => api(`/audits/${runId}`).then((r) => { setRun(r); setError(null); }).catch(setError), [runId]);
  useEffect(() => { setRun(null); load(); }, [load]);

  if (!run) return <section className="panel"><ErrorBanner error={error} /><p className="empty">Loading audit…</p></section>;

  const open = run.status === 'open';
  const perform = open && can('audits:perform');
  const review = run.status === 'submitted' && can('audits:approve');
  const c = run.summary || run.counts;
  const { pct } = progressOf(run);
  const hereOptions = run.buildingId ? (buildings.find((b) => b.id === run.buildingId)?.locations ?? []) : buildings.flatMap((b) => b.locations.map((l) => ({ ...l, name: `${b.name} / ${l.name}` })));
  const items = run.items.filter((i) => (filter === 'all' ? true : filter === 'unexpected' ? !i.expected : i.result === filter && i.expected));

  async function scan(raw) {
    const value = raw.trim();
    if (!value || !isNew(value)) return;
    try {
      const item = await api(`/audits/${run.id}/scan`, { method: 'POST', body: { code: value, locationId: hereId || null } });
      setFlash({ ok: item.result === 'verified' && item.expected, item });
      setError(null);
      await load();
      onChanged();
    } catch (err) {
      setFlash(null);
      setError(err);
    }
  }

  async function mark(assetId, result) {
    try {
      await api(`/audits/${run.id}/items/${assetId}`, { method: 'PATCH', body: { result } });
      await load();
      onChanged();
    } catch (err) { setError(err); }
  }

  async function step(action, ask) {
    let body;
    if (ask) {
      const note = window.prompt(ask);
      if (note === null) return;
      body = { note: note || null };
    }
    try {
      await api(`/audits/${run.id}/${action}`, { method: 'POST', body });
      await load();
      onChanged();
    } catch (err) { setError(err); }
  }

  async function damaged(i) {
    const note = window.prompt(`What is damaged on ${i.assetTag}?`);
    if (!note) return;
    try {
      await api(`/audits/${run.id}/items/${i.assetId}`, { method: 'PATCH', body: { result: 'verified', damaged: true, note } });
      await load();
      onChanged();
    } catch (err) { setError(err); }
  }

  async function close() {
    const pending = c.pending;
    if (!window.confirm(`Close "${run.name}"?${pending ? ` ${pending} unchecked asset${pending === 1 ? '' : 's'} will be recorded as missing.` : ''} A closed audit can't be changed.`)) return;
    try {
      await api(`/audits/${run.id}/close`, { method: 'POST' });
      await load();
      onChanged();
    } catch (err) { setError(err); }
  }

  return (
    <>
      <section className="panel run-head">
        <div className="panel-head wrap">
          <div>
            <h2>{run.name} <Badge map={AUDIT_STATUS} value={run.status} /></h2>
            <p>
              {AUDIT_TYPES[run.auditType] ?? 'Audit'} · {scopeLabel(run)} · Started {formatDate(run.startedAt)} by {run.startedBy}
              {run.status === 'closed' || run.status === 'approved' ? ` · ${run.status === 'approved' ? `Approved ${formatDate(run.reviewedAt)} by ${run.reviewedBy}` : `Closed ${formatDate(run.closedAt)} by ${run.closedBy}`}`
                : run.status === 'submitted' ? ` · Submitted ${formatDate(run.submittedAt)} by ${run.submittedBy}` : ` · Due ${formatDate(run.dueAt)}`}
            </p>
          </div>
          <div className="row-end">
            <a className="outline button-link" href={`${BASE}/api/audits/${run.id}/export`}><Download size={15} /> CSV</a>
            {run.status === 'draft' && can('audits:perform') && <button className="primary" onClick={() => step('begin')}><Play size={15} /> Begin</button>}
            {perform && run.requiresReview && <button className="primary" onClick={() => step('submit', 'Anything the reviewer should know? (optional)')}><Send size={15} /> Submit for review</button>}
            {perform && !run.requiresReview && <button className="outline" onClick={close}><Lock size={15} /> Close audit</button>}
            {review && <>
              <button className="primary" onClick={() => step('approve', 'Approval note (optional)')}><Check size={15} /> Approve</button>
              <button className="outline" onClick={() => {
                const note = window.prompt('What needs another look? (required)');
                if (note) api(`/audits/${run.id}/reject`, { method: 'POST', body: { note } }).then(load).then(onChanged).catch(setError);
              }}><X size={15} /> Send back</button>
            </>}
            {run.status === 'rejected' && can('audits:perform') && <button className="primary" onClick={() => step('reopen')}><RotateCcw size={15} /> Reopen and fix</button>}
          </div>
        </div>
        {run.reviewNote && <p className="wo-resolution pad-x"><strong>Reviewer:</strong> {run.reviewNote}</p>}
        <div className="audit-stats">
          <div><strong>{c.expected}</strong><span>Expected</span></div>
          <div className="good"><strong>{c.verified}</strong><span>Verified</span></div>
          <div className="warn"><strong>{c.wrongLocation}</strong><span>Wrong location</span></div>
          <div className="bad"><strong>{open ? c.pending : c.missing}</strong><span>{open ? 'Not checked' : 'Missing'}</span></div>
          <div><strong>{c.unexpected}</strong><span>Unexpected</span></div>
          <div className="bad"><strong>{c.damaged ?? 0}</strong><span>Damaged</span></div>
          <div><strong>{c.duplicateScans ?? 0}</strong><span>Scanned twice</span></div>
        </div>
        <div className="progress-track run-progress"><div style={{ width: `${pct}%` }} /></div>
      </section>

      {perform && (
        <section className="panel">
          <form className="scan-form audit-scan" onSubmit={(e) => { e.preventDefault(); scan(code); setCode(''); inputRef.current?.focus(); }}>
            <div className="search">
              <ScanLine size={18} />
              <input ref={inputRef} autoFocus value={code} onChange={(e) => setCode(e.target.value)} placeholder="Scan a label or barcode" aria-label="Audit scan" autoComplete="off" />
            </div>
            {!run.locationId && hereOptions.length > 0 && (
              <select value={hereId} onChange={(e) => setHereId(e.target.value)} aria-label="Where you are scanning">
                <option value="">Location not specified</option>
                {hereOptions.map((l) => <option key={l.id} value={l.id}>I'm at {l.name}</option>)}
              </select>
            )}
            <button className="primary">Verify</button>
          </form>
          <CameraScanner onCode={scan} />
          {flash && (
            <div className={`scan-flash ${flash.ok ? 'ok' : 'warn'}`} role="status">
              {flash.ok ? <CheckCircle2 size={18} /> : <MapPinOff size={18} />}
              <span><strong>{flash.item.assetTag} · {flash.item.name}</strong>{' '}
                {flash.item.duplicate ? 'was already scanned; counted as a duplicate scan.'
                  : !flash.item.expected ? 'was not expected in this audit; recorded as found here.'
                  : flash.item.result === 'wrong_location' ? `expected at ${flash.item.expectedLocationName}, found at ${flash.item.foundLocationName}.`
                    : 'verified.'}
              </span>
            </div>
          )}
        </section>
      )}

      <section className="panel">
        <div className="chips">
          {FILTERS.map(([key, label]) => (
            <button key={key} className={filter === key ? 'chip on' : 'chip'} onClick={() => setFilter(key)}>{label}</button>
          ))}
        </div>
        <ErrorBanner error={error} />
        <div className="audit-items">
          {items.length === 0 && <p className="empty">Nothing here.</p>}
          {items.map((i) => {
            const [label, tone] = RESULT[i.result];
            return (
              <div className="audit-item" key={i.assetId}>
                <AssetIcon type={i.type} size={19} />
                <button className="audit-info link-like" onClick={() => onSelectAsset({ id: i.assetId, name: i.name, assetTag: i.assetTag, type: i.type, status: '' })}>
                  <strong>{i.name}{!i.expected && <span className="tag">Unexpected</span>}{i.damaged && <span className="tag">Damaged</span>}{i.scanCount > 1 && <span className="tag">Scanned {i.scanCount}×</span>}</strong>
                  <small>
                    {i.assetTag} · expected {i.expected ? (i.expectedLocationName || i.expectedBuildingName || 'anywhere') : 'elsewhere'}
                    {i.foundLocationName && ` · found at ${i.foundLocationName}`}
                    {i.checkedBy && ` · ${i.method === 'scan' ? 'scanned' : 'marked'} by ${i.checkedBy}`}
                    {i.note && ` · “${i.note}”`}
                  </small>
                </button>
                <span className={`status ${tone}`}><i />{label}</span>
                {perform && i.expected && (
                  <span className="row-end">
                    {i.result !== 'verified' && <button className="link-btn" title="Mark verified" aria-label={`Mark ${i.assetTag} verified`} onClick={() => mark(i.assetId, 'verified')}><CheckCircle2 size={17} /></button>}
                    {i.result !== 'missing' && <button className="link-btn" title="Mark missing" aria-label={`Mark ${i.assetTag} missing`} onClick={() => mark(i.assetId, 'missing')}><XCircle size={17} /></button>}
                    {!i.damaged && <button className="link-btn" title="Mark damaged" aria-label={`Mark ${i.assetTag} damaged`} onClick={() => damaged(i)}><AlertOctagon size={16} /></button>}
                    {i.result !== 'pending' && <button className="link-btn" title="Reset" aria-label={`Reset ${i.assetTag}`} onClick={() => mark(i.assetId, 'pending')}><RotateCcw size={16} /></button>}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </section>
      {run.reviews?.length > 0 && (
        <section className="panel">
          <div className="panel-head"><div><h2>Review history</h2><p>Every start, submission and decision, kept permanently.</p></div></div>
          <ol className="timeline pad">
            {run.reviews.map((v, n) => (
              <li key={n}><strong className="normal">{REVIEW_LABEL[v.action] ?? v.action}</strong><small>{v.actorName || 'System'} · {fmtDateTime(v.createdAt)}{v.note && ` · “${v.note}”`}</small></li>
            ))}
          </ol>
        </section>
      )}
    </>
  );
}

export function Audits({ buildings, onSelect, onChanged }) {
  const { can } = useAuth();
  const [period, setPeriod] = useState('weekly');
  const [runs, setRuns] = useState(null);
  const [selected, setSelected] = useState(null);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState(null);

  const load = useCallback(() => api(`/audits${query({ period, limit: 50 })}`)
    .then((r) => { setRuns(r.items); setError(null); return r.items; })
    .catch(setError), [period]);

  useEffect(() => {
    setRuns(null);
    setStarting(false);
    load().then((items) => setSelected(items?.find((r) => ACTIVE.includes(r.status))?.id ?? null));
  }, [load]);

  const openRuns = runs?.filter((r) => ACTIVE.includes(r.status)) ?? [];
  const closedRuns = runs?.filter((r) => !ACTIVE.includes(r.status)) ?? [];
  const showStart = can('audits:perform') && (starting || (runs && openRuns.length === 0));

  return (
    <>
      <div className="tabs">
        {PERIODS.map(([key, label]) => (
          <button key={key} className={period === key ? 'selected' : ''} onClick={() => setPeriod(key)}>{label} Audits</button>
        ))}
      </div>
      <ErrorBanner error={error} />
      {openRuns.length > 0 && (
        <div className="run-switcher">
          {openRuns.map((r) => (
            <button key={r.id} className={selected === r.id ? 'chip on' : 'chip'} onClick={() => { setSelected(r.id); setStarting(false); }}>
              {scopeLabel(r)} · {r.status === 'open' ? `${progressOf(r).pct}%` : AUDIT_STATUS[r.status][0]}
            </button>
          ))}
          {can('audits:perform') && !starting && <button className="chip" onClick={() => setStarting(true)}><Plus size={13} /> Another scope</button>}
        </div>
      )}
      {showStart && (
        <StartAudit period={period} buildings={buildings} onCancel={openRuns.length ? () => setStarting(false) : null}
          onStarted={(run) => { setStarting(false); setSelected(run.id); load(); onChanged(); }} />
      )}
      {runs && openRuns.length === 0 && !can('audits:perform') && (
        <section className="panel"><p className="empty">No audit of this kind is open right now.</p></section>
      )}
      {selected && !starting && <RunView key={selected} runId={selected} buildings={buildings} onSelectAsset={onSelect} onChanged={() => { load(); onChanged(); }} />}
      <section className="panel">
        <div className="panel-head"><div><h2>Past {period === 'manual' ? 'one-off' : period} audits <span className="count">{closedRuns.length}</span></h2><p>Closed and approved audits are permanent records.</p></div></div>
        <div className="table-scroll">
          <table>
            <thead><tr><th>AUDIT</th><th>SCOPE</th><th>FINISHED</th><th>VERIFIED</th><th>WRONG LOCATION</th><th>MISSING</th><th>UNEXPECTED</th></tr></thead>
            <tbody>
              {closedRuns.length === 0 && <tr><td colSpan="7" className="empty">No finished audits here yet.</td></tr>}
              {closedRuns.map((r) => (
                <tr key={r.id} className="clickable" tabIndex={0} onClick={() => { setSelected(r.id); setStarting(false); window.scrollTo({ top: 0, behavior: 'smooth' }); }}
                  onKeyDown={(e) => e.key === 'Enter' && setSelected(r.id)}>
                  <td><strong className="normal">{r.name}</strong><small>{r.status === 'approved' ? `approved by ${r.reviewedBy}` : `by ${r.closedBy}`}</small></td>
                  <td>{scopeLabel(r)}</td>
                  <td>{formatDate(r.closedAt ?? r.reviewedAt)}</td>
                  <td>{(r.summary || r.counts).verified} / {(r.summary || r.counts).expected}</td>
                  <td>{(r.summary || r.counts).wrongLocation}</td>
                  <td>{(r.summary || r.counts).missing}</td>
                  <td>{(r.summary || r.counts).unexpected}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}
