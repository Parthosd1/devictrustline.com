import React, { useCallback, useEffect, useRef, useState } from 'react';
import { CheckCircle2, ClipboardCheck, Lock, MapPinOff, Plus, RotateCcw, ScanLine, XCircle } from 'lucide-react';
import { api, query } from '../api.js';
import { useAuth } from '../auth.jsx';
import { CameraScanner, useDedupe } from '../camera.jsx';
import { AssetIcon, ErrorBanner } from '../components.jsx';

const PERIODS = [['weekly', 'Weekly'], ['monthly', 'Monthly']];
const RESULT = {
  pending: ['Not checked', 'retired'],
  verified: ['Verified', 'deployed'],
  wrong_location: ['Wrong location', 'maintenance'],
  missing: ['Missing', 'missing'],
};
const FILTERS = [['all', 'All'], ['pending', 'Not checked'], ['verified', 'Verified'], ['wrong_location', 'Wrong location'], ['missing', 'Missing'], ['unexpected', 'Unexpected']];

const formatDate = (iso) => new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
const scopeLabel = (run) => (run.locationName ? `${run.buildingName} / ${run.locationName}` : run.buildingName || 'All buildings');
export const progressOf = (run) => {
  const c = run.summary || run.counts;
  const found = c.verified + c.wrongLocation;
  return { found, expected: c.expected, pct: c.expected ? Math.round((found / c.expected) * 100) : 0 };
};

function StartAudit({ period, buildings, onStarted, onCancel }) {
  const [buildingId, setBuildingId] = useState('');
  const [locationId, setLocationId] = useState('');
  const [error, setError] = useState(null);
  const locations = buildings.find((b) => b.id === buildingId)?.locations ?? [];
  async function start(e) {
    e.preventDefault();
    try {
      onStarted(await api('/audits', { method: 'POST', body: { period, buildingId: buildingId || null, locationId: locationId || null } }));
    } catch (err) { setError(err); }
  }
  return (
    <section className="panel start-audit">
      <div className="panel-head"><div>
        <h2>Start a {period} audit</h2>
        <p>Everything currently in the chosen scope is expected. {period === 'weekly' ? 'Due in 7 days.' : 'Due at the end of this month.'} Weekly and monthly audits never share results.</p>
      </div></div>
      <ErrorBanner error={error} />
      <form className="filters" onSubmit={start}>
        <select value={buildingId} onChange={(e) => { setBuildingId(e.target.value); setLocationId(''); }} aria-label="Building to audit">
          <option value="">All buildings</option>
          {buildings.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
        </select>
        <select value={locationId} onChange={(e) => setLocationId(e.target.value)} disabled={!buildingId} aria-label="Location to audit">
          <option value="">Whole building</option>
          {locations.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
        </select>
        <button className="primary"><ClipboardCheck size={16} /> Start audit</button>
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
            <h2>{run.name} {!open && <span className="count"><Lock size={11} /> Closed</span>}</h2>
            <p>{scopeLabel(run)} · Started {formatDate(run.startedAt)} by {run.startedBy} · {open ? `Due ${formatDate(run.dueAt)}` : `Closed ${formatDate(run.closedAt)} by ${run.closedBy}`}</p>
          </div>
          {perform && <button className="outline" onClick={close}><Lock size={15} /> Close audit</button>}
        </div>
        <div className="audit-stats">
          <div><strong>{c.expected}</strong><span>Expected</span></div>
          <div className="good"><strong>{c.verified}</strong><span>Verified</span></div>
          <div className="warn"><strong>{c.wrongLocation}</strong><span>Wrong location</span></div>
          <div className="bad"><strong>{open ? c.pending : c.missing}</strong><span>{open ? 'Not checked' : 'Missing'}</span></div>
          <div><strong>{c.unexpected}</strong><span>Unexpected</span></div>
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
                {!flash.item.expected ? 'was not expected in this audit; recorded as found here.'
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
                  <strong>{i.name}{!i.expected && <span className="tag">Unexpected</span>}</strong>
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
                    {i.result !== 'pending' && <button className="link-btn" title="Reset" aria-label={`Reset ${i.assetTag}`} onClick={() => mark(i.assetId, 'pending')}><RotateCcw size={16} /></button>}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </section>
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
    load().then((items) => setSelected(items?.find((r) => r.status === 'open')?.id ?? null));
  }, [load]);

  const openRuns = runs?.filter((r) => r.status === 'open') ?? [];
  const closedRuns = runs?.filter((r) => r.status === 'closed') ?? [];
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
              {scopeLabel(r)} · {progressOf(r).pct}%
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
        <section className="panel"><p className="empty">No {period} audit is open right now.</p></section>
      )}
      {selected && !starting && <RunView key={selected} runId={selected} buildings={buildings} onSelectAsset={onSelect} onChanged={() => { load(); onChanged(); }} />}
      <section className="panel">
        <div className="panel-head"><div><h2>Past {period} audits <span className="count">{closedRuns.length}</span></h2><p>Closed audits are permanent records.</p></div></div>
        <div className="table-scroll">
          <table>
            <thead><tr><th>AUDIT</th><th>SCOPE</th><th>CLOSED</th><th>VERIFIED</th><th>WRONG LOCATION</th><th>MISSING</th><th>UNEXPECTED</th></tr></thead>
            <tbody>
              {closedRuns.length === 0 && <tr><td colSpan="7" className="empty">No closed {period} audits yet.</td></tr>}
              {closedRuns.map((r) => (
                <tr key={r.id} className="clickable" tabIndex={0} onClick={() => { setSelected(r.id); setStarting(false); window.scrollTo({ top: 0, behavior: 'smooth' }); }}
                  onKeyDown={(e) => e.key === 'Enter' && setSelected(r.id)}>
                  <td><strong className="normal">{r.name}</strong><small>by {r.closedBy}</small></td>
                  <td>{scopeLabel(r)}</td>
                  <td>{formatDate(r.closedAt)}</td>
                  <td>{r.summary.verified} / {r.summary.expected}</td>
                  <td>{r.summary.wrongLocation}</td>
                  <td>{r.summary.missing}</td>
                  <td>{r.summary.unexpected}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}
