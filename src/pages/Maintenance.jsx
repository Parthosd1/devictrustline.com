import React, { useCallback, useEffect, useState } from 'react';
import { AlertTriangle, CheckCircle2, Clock, Plus, Wrench, XCircle } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { AssetIcon, ErrorBanner, Field, Modal } from '../components.jsx';

export const PRIORITY = { urgent: ['Urgent', 'missing'], high: ['High', 'maintenance'], normal: ['Normal', 'available'], low: ['Low', 'retired'] };
export const WO_STATUS = {
  open: ['Open', 'available'], in_progress: ['In progress', 'deployed'], waiting_parts: ['Waiting on parts', 'maintenance'],
  resolved: ['Resolved', 'deployed'], cancelled: ['Cancelled', 'retired'],
};
const FILTERS = [['active', 'Active'], ['waiting_parts', 'Waiting on parts'], ['resolved', 'Resolved'], ['cancelled', 'Cancelled'], ['', 'All']];
const fmtDate = (d) => (d ? new Date(d.length === 10 ? `${d}T00:00:00` : d).toLocaleDateString() : '—');
const fmtMoney = (n) => (n == null ? '—' : n.toLocaleString(undefined, { style: 'currency', currency: 'USD' }));

export function Badge({ map, value }) {
  const [label, tone] = map[value];
  return <span className={`status ${tone}`}><i />{label}</span>;
}

function useAssignees() {
  const [list, setList] = useState([]);
  useEffect(() => { api('/work-orders/assignees').then((r) => setList(r.items)).catch(() => {}); }, []);
  return list;
}

export function NewWorkOrder({ assets, asset, onClose, onCreated }) {
  const assignees = useAssignees();
  const [form, setForm] = useState({
    assetId: asset?.id ?? '', title: '', description: '', priority: 'normal', assigneeId: '', vendor: '', dueDate: '', setAssetToMaintenance: true,
  });
  const [error, setError] = useState(null);
  const set = (key) => (e) => setForm({ ...form, [key]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  async function submit(e) {
    e.preventDefault();
    const body = { ...form, assigneeId: form.assigneeId || null, dueDate: form.dueDate || null };
    try { onCreated(await api('/work-orders', { method: 'POST', body })); } catch (err) { setError(err); }
  }
  const choices = (assets ?? []).filter((a) => a.status !== 'Retired');
  return (
    <Modal title="New Work Order" subtitle={asset ? `${asset.assetTag} · ${asset.name}` : 'Report a problem with a device'} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          {!asset && (
            <Field label="Asset" wide>
              <select required value={form.assetId} onChange={set('assetId')}>
                <option value="">Choose an asset</option>
                {choices.map((a) => <option key={a.id} value={a.id}>{a.assetTag} · {a.name}{a.locationName ? ` · ${a.locationName}` : ''}</option>)}
              </select>
            </Field>
          )}
          <Field label="Problem" wide><input required value={form.title} onChange={set('title')} placeholder="Screen flickers, won't charge, trigger sticks…" /></Field>
          <Field label="Details" wide><textarea rows={3} value={form.description} onChange={set('description')} /></Field>
          <Field label="Priority">
            <select value={form.priority} onChange={set('priority')}>{Object.entries(PRIORITY).map(([k, [l]]) => <option key={k} value={k}>{l}</option>)}</select>
          </Field>
          <Field label="Assign to">
            <select value={form.assigneeId} onChange={set('assigneeId')}>
              <option value="">Unassigned</option>
              {assignees.map((u) => <option key={u.id} value={u.id}>{u.name}</option>)}
            </select>
          </Field>
          <Field label="Vendor"><input value={form.vendor} onChange={set('vendor')} placeholder="Optional" /></Field>
          <Field label="Due date"><input type="date" value={form.dueDate} onChange={set('dueDate')} /></Field>
        </div>
        <label className="check"><input type="checkbox" checked={form.setAssetToMaintenance} onChange={set('setAssetToMaintenance')} /> Mark the asset as In Maintenance until this is closed</label>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><Plus size={16} /> Open Work Order</button>
        </div>
      </form>
    </Modal>
  );
}

function WorkOrderDetail({ id, onClose, onChanged, onSelectAsset }) {
  const { can } = useAuth();
  const assignees = useAssignees();
  const [wo, setWo] = useState(null);
  const [error, setError] = useState(null);
  const [note, setNote] = useState('');
  const [closing, setClosing] = useState(null);
  const [closeForm, setCloseForm] = useState({ text: '', cost: '', assetStatus: '' });

  const load = useCallback(() => api(`/work-orders/${id}`).then(setWo).catch(setError), [id]);
  useEffect(() => { load(); }, [load]);

  async function act(fn) {
    try { await fn(); setError(null); await load(); onChanged(); return true; } catch (err) { setError(err); return false; }
  }
  const patch = (body) => act(() => api(`/work-orders/${id}`, { method: 'PATCH', body }));

  if (!wo) return <Modal title="Work order" onClose={onClose}><ErrorBanner error={error} /><p className="muted">Loading…</p></Modal>;
  const active = !['resolved', 'cancelled'].includes(wo.status);
  const edit = active && can('maintenance:write');

  return (
    <Modal title={`${wo.number} · ${wo.title}`} subtitle={`Opened ${fmtDate(wo.openedAt)} by ${wo.openedBy}`} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <button className="component" onClick={() => { onClose(); onSelectAsset({ id: wo.assetId, name: wo.assetName, assetTag: wo.assetTag, type: wo.assetType, status: '' }); }}>
        <AssetIcon type={wo.assetType} size={16} />
        <span><strong>{wo.assetName}</strong><small>{wo.assetTag} · {wo.locationName || wo.buildingName || 'No location'} · currently {wo.assetStatus}</small></span>
      </button>
      <div className="wo-grid">
        <div><span className="rel-label">Status</span>
          {edit ? (
            <select className="compact" value={wo.status} onChange={(e) => patch({ status: e.target.value })} aria-label="Status">
              {['open', 'in_progress', 'waiting_parts'].map((s) => <option key={s} value={s}>{WO_STATUS[s][0]}</option>)}
            </select>
          ) : <Badge map={WO_STATUS} value={wo.status} />}
        </div>
        <div><span className="rel-label">Priority</span>
          {edit ? (
            <select className="compact" value={wo.priority} onChange={(e) => patch({ priority: e.target.value })} aria-label="Priority">
              {Object.entries(PRIORITY).map(([k, [l]]) => <option key={k} value={k}>{l}</option>)}
            </select>
          ) : <Badge map={PRIORITY} value={wo.priority} />}
        </div>
        <div><span className="rel-label">Assigned to</span>
          {edit ? (
            <select className="compact" value={wo.assigneeId ?? ''} onChange={(e) => patch({ assigneeId: e.target.value || null })} aria-label="Assigned to">
              <option value="">Unassigned</option>
              {assignees.map((u) => <option key={u.id} value={u.id}>{u.name}</option>)}
            </select>
          ) : <strong>{wo.assigneeName || 'Unassigned'}</strong>}
        </div>
        <div><span className="rel-label">Due</span><strong className={wo.overdue ? 'overdue' : ''}>{fmtDate(wo.dueDate)}{wo.overdue && ' · overdue'}</strong></div>
        <div><span className="rel-label">Vendor</span><strong>{wo.vendor || '—'}</strong></div>
        <div><span className="rel-label">Cost</span><strong>{fmtMoney(wo.cost)}</strong></div>
      </div>
      {wo.description && <p className="pre wo-desc">{wo.description}</p>}
      {wo.resolution && <p className="wo-resolution"><strong>{wo.status === 'resolved' ? 'Resolution' : 'Cancelled'}:</strong> {wo.resolution}</p>}

      {edit && !closing && (
        <div className="modal-actions top">
          <button className="primary" onClick={() => setClosing('resolve')}><CheckCircle2 size={16} /> Resolve</button>
          <button className="outline" onClick={() => setClosing('cancel')}><XCircle size={16} /> Cancel work order</button>
        </div>
      )}
      {closing && (
        <form className="assign-panel" onSubmit={(e) => {
          e.preventDefault();
          act(() => (closing === 'resolve'
            ? api(`/work-orders/${id}/resolve`, { method: 'POST', body: { resolution: closeForm.text, cost: closeForm.cost || null, assetStatus: closeForm.assetStatus || null } })
            : api(`/work-orders/${id}/cancel`, { method: 'POST', body: { reason: closeForm.text } }))).then((ok) => ok && setClosing(null));
        }}>
          <div className="form-grid">
            <Field label={closing === 'resolve' ? 'What was done' : 'Why cancel'} wide><textarea required rows={2} value={closeForm.text} onChange={(e) => setCloseForm({ ...closeForm, text: e.target.value })} /></Field>
            {closing === 'resolve' && (
              <>
                <Field label="Final cost (USD)"><input inputMode="decimal" pattern="\d+(\.\d{1,2})?" value={closeForm.cost} onChange={(e) => setCloseForm({ ...closeForm, cost: e.target.value })} placeholder={wo.cost != null ? String(wo.cost) : 'Optional'} /></Field>
                <Field label="Asset becomes">
                  <select value={closeForm.assetStatus} onChange={(e) => setCloseForm({ ...closeForm, assetStatus: e.target.value })}>
                    <option value="">Back in service (automatic)</option>
                    <option value="Available">Available</option>
                    <option value="Deployed">Deployed</option>
                    <option value="Retired">Retired (beyond repair)</option>
                  </select>
                </Field>
              </>
            )}
          </div>
          <div className="modal-actions">
            <button type="button" className="outline" onClick={() => setClosing(null)}>Back</button>
            <button className="primary">{closing === 'resolve' ? 'Resolve' : 'Cancel work order'}</button>
          </div>
        </form>
      )}

      <h3 className="section-title">Activity</h3>
      <ol className="timeline">
        {wo.notes.map((n) => (
          <li key={n.id}><strong className={n.kind === 'status' ? 'muted-strong' : ''}>{n.body}</strong><small>{n.author || 'System'} · {new Date(n.createdAt).toLocaleString()}</small></li>
        ))}
      </ol>
      {can('maintenance:write') && (
        <form className="inline-form" onSubmit={(e) => { e.preventDefault(); act(() => api(`/work-orders/${id}/notes`, { method: 'POST', body: { body: note } })).then((ok) => ok && setNote('')); }}>
          <input required value={note} onChange={(e) => setNote(e.target.value)} placeholder="Add a note" aria-label="Add a note" />
          <button className="outline">Add note</button>
        </form>
      )}
    </Modal>
  );
}

export function WorkOrderList({ items, onOpen, compact }) {
  return (
    <div className="table-scroll">
      <table>
        <thead><tr><th>WORK ORDER</th>{!compact && <th>ASSET</th>}<th>PRIORITY</th><th>STATUS</th><th>ASSIGNED</th><th>DUE</th></tr></thead>
        <tbody>
          {items.length === 0 && <tr><td colSpan={compact ? 5 : 6} className="empty">No work orders.</td></tr>}
          {items.map((w) => (
            <tr key={w.id} className="clickable" tabIndex={0} onClick={() => onOpen(w.id)} onKeyDown={(e) => e.key === 'Enter' && onOpen(w.id)}>
              <td><strong className="normal">{w.title}</strong><small>{w.number} · opened {fmtDate(w.openedAt)}</small></td>
              {!compact && <td><strong className="normal">{w.assetName}</strong><small>{w.assetTag}</small></td>}
              <td><Badge map={PRIORITY} value={w.priority} /></td>
              <td><Badge map={WO_STATUS} value={w.status} /></td>
              <td>{w.assigneeName || <span className="muted">—</span>}</td>
              <td className={w.overdue ? 'overdue' : ''}>{fmtDate(w.dueDate)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function Maintenance({ assets, onSelect, onChanged, version }) {
  const { can } = useAuth();
  const [filter, setFilter] = useState('active');
  const [items, setItems] = useState(null);
  const [all, setAll] = useState([]);
  const [error, setError] = useState(null);
  const [creating, setCreating] = useState(false);
  const [openId, setOpenId] = useState(null);

  const load = useCallback(async () => {
    try {
      const [list, active] = await Promise.all([
        api(`/work-orders${filter ? `?status=${filter}` : ''}`), api('/work-orders?status=active'),
      ]);
      setItems(list.items);
      setAll(active.items);
      setError(null);
    } catch (err) { setError(err); }
  }, [filter]);
  useEffect(() => { load(); }, [load, version]);

  const stats = [
    ['Active', all.length, Wrench, ''],
    ['Urgent or high', all.filter((w) => ['urgent', 'high'].includes(w.priority)).length, AlertTriangle, 'warn'],
    ['Overdue', all.filter((w) => w.overdue).length, Clock, 'bad'],
    ['Waiting on parts', all.filter((w) => w.status === 'waiting_parts').length, Clock, ''],
  ];
  const changed = () => { load(); onChanged(); };

  return (
    <>
      <div className="metrics">
        {stats.map(([label, value, Icon, tone]) => (
          <div className={`metric ${tone}`} key={label}>
            <div className="metric-top"><span>{label}</span><span className="metric-icon"><Icon size={19} /></span></div>
            <strong>{value}</strong>
          </div>
        ))}
      </div>
      <section className="panel">
        <div className="panel-head wrap">
          <div><h2>Work Orders</h2><p>Repairs and investigations, newest and most urgent first</p></div>
          {can('maintenance:write') && <button className="outline" onClick={() => setCreating(true)}><Plus size={16} /> New Work Order</button>}
        </div>
        <div className="chips" style={{ paddingTop: 0 }}>
          {FILTERS.map(([key, label]) => <button key={key} className={filter === key ? 'chip on' : 'chip'} onClick={() => setFilter(key)}>{label}</button>)}
        </div>
        <ErrorBanner error={error} />
        {items === null ? <p className="empty">Loading…</p> : <WorkOrderList items={items} onOpen={setOpenId} />}
      </section>
      {creating && <NewWorkOrder assets={assets} onClose={() => setCreating(false)} onCreated={(wo) => { setCreating(false); setOpenId(wo.id); changed(); }} />}
      {openId && <WorkOrderDetail id={openId} onClose={() => setOpenId(null)} onChanged={changed} onSelectAsset={onSelect} />}
    </>
  );
}

export { WorkOrderDetail };
