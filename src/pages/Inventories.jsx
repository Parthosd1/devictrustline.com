import React, { useCallback, useEffect, useState } from 'react';
import { Archive, ArrowRightLeft, Check, KeyRound, Pencil, Plus, RotateCcw, X } from 'lucide-react';
import { api, query } from '../api.js';
import { useAuth } from '../auth.jsx';
import { AssetTable, ErrorBanner, Field, Modal } from '../components.jsx';
import { fmtDateTime, plural } from '../format.js';
import { Badge } from './Maintenance.jsx';

// Mirrors KINDS in api/src/routes/inventories.rs.
export const INVENTORY_KINDS = {
  hub: 'Hub', general: 'General', available: 'Available stock', loaner: 'Loaners', repair: 'RMA / repair', deployment: 'Deployment',
  returned: 'Returned equipment', technician: 'Technician stock', site: 'Site',
};
const TRANSFER_STATUS = {
  pending: ['Waiting for approval', 'maintenance'], completed: ['Completed', 'deployed'], rejected: ['Rejected', 'missing'], cancelled: ['Cancelled', 'retired'],
};
const REQUEST_STATUS = { pending: ['Pending', 'maintenance'], approved: ['Approved', 'deployed'], rejected: ['Rejected', 'missing'], cancelled: ['Cancelled', 'retired'] };

export function useInventories(archived = false) {
  const [list, setList] = useState([]);
  const [error, setError] = useState(null);
  const load = useCallback(() => api(`/inventories${archived ? '?archived=all' : ''}`).then((r) => { setList(r.items); setError(null); }).catch(setError), [archived]);
  useEffect(() => { load(); }, [load]);
  return { list, error, reload: load };
}

function useStaff() {
  const [list, setList] = useState([]);
  useEffect(() => { api('/work-orders/assignees').then((r) => setList(r.items)).catch(() => {}); }, []);
  return list;
}

export const InventoryOptions = ({ list, exclude }) => list.filter((i) => !i.archivedAt && i.id !== exclude).map((i) => (
  <option key={i.id} value={i.id}>{'  '.repeat(i.depth)}{i.name}</option>
));

function InventoryForm({ inventory, parentId, inventories, buildings, onClose, onSaved }) {
  const staff = useStaff();
  const editing = Boolean(inventory?.id);
  const [form, setForm] = useState({
    name: inventory?.name ?? '', kind: inventory?.kind ?? 'general', parentId: inventory?.parentId ?? parentId ?? '',
    description: inventory?.description ?? '', buildingId: inventory?.buildingId ?? '', locationId: inventory?.locationId ?? '',
    custodianId: inventory?.custodianId ?? '', requiresApproval: inventory?.requiresApproval ?? false,
  });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  const locations = buildings.find((b) => b.id === form.buildingId)?.locations ?? [];
  async function submit(e) {
    e.preventDefault();
    const body = {
      ...form, parentId: form.parentId || null, description: form.description || null, buildingId: form.buildingId || null,
      locationId: form.locationId || null, custodianId: form.custodianId || null,
    };
    try { onSaved(await api(editing ? `/inventories/${inventory.id}` : '/inventories', { method: editing ? 'PATCH' : 'POST', body })); } catch (err) { setError(err); }
  }
  return (
    <Modal title={editing ? `Edit ${inventory.name}` : 'New inventory'} subtitle="Inventories can sit inside other inventories, like a hub with stock rooms." onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name"><input required maxLength={120} value={form.name} onChange={set('name')} /></Field>
          <Field label="Kind">
            <select value={form.kind} onChange={set('kind')}>{Object.entries(INVENTORY_KINDS).map(([k, l]) => <option key={k} value={k}>{l}</option>)}</select>
          </Field>
          <Field label="Inside">
            <select value={form.parentId} onChange={set('parentId')}>
              <option value="">Top level</option>
              <InventoryOptions list={inventories} exclude={inventory?.id} />
            </select>
          </Field>
          <Field label="Custodian">
            <select value={form.custodianId} onChange={set('custodianId')}>
              <option value="">Nobody in particular</option>
              {staff.map((u) => <option key={u.id} value={u.id}>{u.name}</option>)}
            </select>
          </Field>
          <Field label="Building">
            <select value={form.buildingId} onChange={(e) => setForm({ ...form, buildingId: e.target.value, locationId: '' })}>
              <option value="">None</option>
              {buildings.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
            </select>
          </Field>
          <Field label="Location">
            <select value={form.locationId} onChange={set('locationId')} disabled={!form.buildingId}>
              <option value="">Whole building</option>
              {locations.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
            </select>
          </Field>
          <Field label="Description" wide><textarea rows={2} value={form.description} onChange={set('description')} /></Field>
        </div>
        <label className="check"><input type="checkbox" checked={form.requiresApproval} onChange={set('requiresApproval')} /> Transfers into or out of this inventory need a manager’s approval</label>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary">{editing ? 'Save' : <><Plus size={16} /> Create</>}</button>
        </div>
      </form>
    </Modal>
  );
}

export function TransferForm({ inventories, assetIds, toInventoryId, onClose, onDone }) {
  const [form, setForm] = useState({ codes: '', toInventoryId: toInventoryId ?? '', reason: '', includeComponents: true });
  const [error, setError] = useState(null);
  const [result, setResult] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  async function submit(e) {
    e.preventDefault();
    const body = { toInventoryId: form.toInventoryId, reason: form.reason || null, includeComponents: form.includeComponents };
    if (assetIds?.length) body.assetIds = assetIds; else body.codes = form.codes.split(/[\s,]+/).filter(Boolean);
    try { setResult(await api('/transfers', { method: 'POST', body })); setError(null); onDone(); } catch (err) { setError(err); }
  }
  if (result) {
    return (
      <Modal title="Transfer recorded" onClose={onClose}>
        <ul className="name-list">
          {result.completed.length > 0 && <li><span><strong className="normal">Moved: {plural(result.completed.length, 'item')}</strong><small>{result.completed.map((t) => t.assetTag).join(', ')}</small></span></li>}
          {result.pending.length > 0 && <li><span><strong className="normal">Waiting for approval: {plural(result.pending.length, 'item')}</strong><small>{result.pending.map((t) => t.assetTag).join(', ')}</small></span></li>}
          {result.skipped.map((s) => <li key={s.assetId}><span><strong className="normal">Not moved: {s.assetTag}</strong><small>{s.reason}</small></span></li>)}
        </ul>
        <div className="modal-actions"><button className="primary" onClick={onClose}>Done</button></div>
      </Modal>
    );
  }
  return (
    <Modal title="Transfer items" subtitle={assetIds?.length ? plural(assetIds.length, 'item') : 'Scan or type asset tags, serial numbers or barcodes, one per line.'} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          {!assetIds?.length && <Field label="Items" wide><textarea required rows={4} value={form.codes} onChange={set('codes')} placeholder={'DT-1001\nDT-1002'} autoFocus /></Field>}
          <Field label="To inventory">
            <select required value={form.toInventoryId} onChange={set('toInventoryId')}>
              <option value="">Choose</option>
              <InventoryOptions list={inventories} />
            </select>
          </Field>
          <Field label="Reason"><input maxLength={500} value={form.reason} onChange={set('reason')} placeholder="Optional" /></Field>
        </div>
        <label className="check"><input type="checkbox" checked={form.includeComponents} onChange={set('includeComponents')} /> Move attached components with their parent item</label>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><ArrowRightLeft size={16} /> Transfer</button>
        </div>
      </form>
    </Modal>
  );
}

function InventoryDetail({ id, inventories, buildings, onClose, onChanged, onSelect }) {
  const { can } = useAuth();
  const [inv, setInv] = useState(null);
  const [items, setItems] = useState(null);
  const [error, setError] = useState(null);
  const [editing, setEditing] = useState(false);
  const [moving, setMoving] = useState(false);
  const load = useCallback(async () => {
    try {
      const [i, a] = await Promise.all([api(`/inventories/${id}`), api(`/assets${query({ inventoryId: id, includeSub: 'true', limit: 500 })}`)]);
      setInv(i); setItems(a.items); setError(null);
    } catch (err) { setError(err); }
  }, [id]);
  useEffect(() => { load(); }, [load]);
  const act = async (path) => { try { await api(path, { method: 'POST' }); load(); onChanged(); } catch (err) { setError(err); } };
  if (!inv) return <Modal title="Inventory" onClose={onClose}><ErrorBanner error={error} /><p className="muted">Loading…</p></Modal>;
  return (
    <Modal title={inv.name} subtitle={`${inv.path} · ${INVENTORY_KINDS[inv.kind]}${inv.archivedAt ? ' · archived' : ''}`} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <div className="wo-grid">
        <div><span className="rel-label">Items (with sub-inventories)</span><strong>{items?.length ?? '…'}</strong></div>
        <div><span className="rel-label">By status</span><strong>{Object.entries(inv.byStatus ?? {}).map(([k, v]) => `${k} ${v}`).join(' · ') || '—'}</strong></div>
        <div><span className="rel-label">Custodian</span><strong>{inv.custodianName || '—'}</strong></div>
        <div><span className="rel-label">Place</span><strong>{[inv.buildingName, inv.locationName].filter(Boolean).join(' / ') || '—'}</strong></div>
        <div><span className="rel-label">Transfers</span><strong>{inv.requiresApproval ? 'Need approval' : 'Immediate'}</strong></div>
        <div><span className="rel-label">Pending transfers</span><strong>{inv.pendingTransfers}</strong></div>
      </div>
      {inv.description && <p className="pre wo-desc">{inv.description}</p>}
      {inv.children?.length > 0 && <p className="hint">Inside: {inv.children.map((c) => c.name).join(', ')}</p>}
      <div className="modal-actions top">
        {can('inventory:transfer') && !inv.archivedAt && <button className="primary" onClick={() => setMoving(true)}><ArrowRightLeft size={15} /> Transfer items here</button>}
        {can('inventory:manage') && !inv.archivedAt && <button className="outline" onClick={() => setEditing(true)}><Pencil size={15} /> Edit</button>}
        {can('inventory:manage') && (inv.archivedAt
          ? <button className="outline" onClick={() => act(`/inventories/${id}/restore`)}><RotateCcw size={15} /> Restore</button>
          : <button className="outline" onClick={() => window.confirm(`Archive ${inv.name}? Only empty inventories can be archived.`) && act(`/inventories/${id}/archive`)}><Archive size={15} /> Archive</button>)}
      </div>
      <h3 className="section-title">Items</h3>
      <AssetTable assets={items ?? []} loading={!items} onSelect={(a) => { onClose(); onSelect(a); }} />
      {editing && <InventoryForm inventory={inv} inventories={inventories} buildings={buildings} onClose={() => setEditing(false)} onSaved={() => { setEditing(false); load(); onChanged(); }} />}
      {moving && <TransferForm inventories={inventories} toInventoryId={id} onClose={() => setMoving(false)} onDone={() => { load(); onChanged(); }} />}
    </Modal>
  );
}

function Transfers({ version, onChanged, onSelect }) {
  const { can, user } = useAuth();
  const [status, setStatus] = useState('pending');
  const [items, setItems] = useState(null);
  const [error, setError] = useState(null);
  const load = useCallback(() => api(`/transfers${query({ status: status === 'all' ? '' : status, limit: 200 })}`).then((r) => { setItems(r.items); setError(null); }).catch(setError), [status]);
  useEffect(() => { load(); }, [load, version]);
  async function act(t, action) {
    let body;
    if (action === 'reject') {
      const reason = window.prompt(`Why reject moving ${t.assetTag} to ${t.toInventoryName}?`);
      if (!reason) return;
      body = { reason };
    }
    try { await api(`/transfers/${t.id}/${action}`, { method: 'POST', body }); load(); onChanged(); } catch (err) { setError(err); }
  }
  return (
    <section className="panel">
      <div className="panel-head wrap"><div><h2>Transfers</h2><p>Every move between inventories, with who asked and who approved.</p></div></div>
      <div className="chips" style={{ paddingTop: 0 }}>
        {[['pending', 'Waiting'], ['completed', 'Completed'], ['rejected', 'Rejected'], ['cancelled', 'Cancelled'], ['all', 'All']].map(([k, l]) => (
          <button key={k} className={status === k ? 'chip on' : 'chip'} onClick={() => setStatus(k)}>{l}</button>
        ))}
      </div>
      <ErrorBanner error={error} />
      <div className="table-scroll">
        <table>
          <thead><tr><th>ITEM</th><th>FROM → TO</th><th>REQUESTED</th><th>STATUS</th><th /></tr></thead>
          <tbody>
            {items === null && <tr><td colSpan="5" className="empty">Loading…</td></tr>}
            {items?.length === 0 && <tr><td colSpan="5" className="empty">No transfers here.</td></tr>}
            {items?.map((t) => (
              <tr key={t.id}>
                <td><button className="link-like" onClick={() => onSelect({ id: t.assetId, name: t.assetName, assetTag: t.assetTag, type: '', status: '' })}><strong className="normal">{t.assetName}</strong><small>{t.assetTag}{t.viaAssetTag && ` · with ${t.viaAssetTag}`}</small></button></td>
                <td>{t.fromInventoryName || 'No inventory'} → <strong className="normal">{t.toInventoryName}</strong>{t.reason && <small>{t.reason}</small>}</td>
                <td>{t.requestedBy}<small>{fmtDateTime(t.requestedAt)}</small></td>
                <td><Badge map={TRANSFER_STATUS} value={t.status} />{t.decidedBy && t.status !== 'pending' && <small>{t.decidedBy}{t.decisionNote && `: ${t.decisionNote}`}</small>}</td>
                <td>
                  {t.status === 'pending' && (
                    <span className="row-end">
                      {can('transfers:approve') && t.requestedById !== user.id && <>
                        <button className="link-btn" title="Approve" aria-label={`Approve transfer of ${t.assetTag}`} onClick={() => act(t, 'approve')}><Check size={17} /></button>
                        <button className="link-btn" title="Reject" aria-label={`Reject transfer of ${t.assetTag}`} onClick={() => act(t, 'reject')}><X size={17} /></button>
                      </>}
                      {(t.requestedById === user.id || can('transfers:approve')) && <button className="text-link" onClick={() => act(t, 'cancel')}>Cancel</button>}
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

// Limited users ask for an inventory; managers and admins with full access decide.
export function AccessRequests({ version, onChanged }) {
  const { user } = useAuth();
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [form, setForm] = useState({ inventoryId: '', note: '' });
  const load = useCallback(() => api('/access-requests').then((d) => { setData(d); setError(null); }).catch(setError), []);
  useEffect(() => { load(); }, [load, version]);
  async function act(path, body) { try { await api(path, { method: 'POST', body }); load(); onChanged?.(); } catch (err) { setError(err); } }
  if (!data) return null;
  const pending = data.items.filter((r) => r.status === 'pending');
  if (!data.canDecide && data.requestable.length === 0 && data.items.length === 0) return null;
  return (
    <section className="panel">
      <div className="panel-head wrap"><div><h2><KeyRound size={17} /> Access requests {pending.length > 0 && <span className="count">{pending.length} pending</span>}</h2>
        <p>{data.canDecide ? 'People with limited access asking to see more inventories.' : 'Ask for access to an inventory you cannot see yet.'}</p></div></div>
      <ErrorBanner error={error} />
      {data.requestable.length > 0 && (
        <form className="inline-form" onSubmit={(e) => { e.preventDefault(); act('/access-requests', { inventoryId: form.inventoryId, note: form.note || null }).then(() => setForm({ inventoryId: '', note: '' })); }}>
          <select required value={form.inventoryId} onChange={(e) => setForm({ ...form, inventoryId: e.target.value })} aria-label="Inventory to ask for">
            <option value="">Inventory to ask for</option>
            {data.requestable.map((i) => <option key={i.id} value={i.id}>{i.name}</option>)}
          </select>
          <input value={form.note} onChange={(e) => setForm({ ...form, note: e.target.value })} placeholder="Why (optional)" aria-label="Why" maxLength={500} />
          <button className="outline">Ask for access</button>
        </form>
      )}
      <div className="table-scroll">
        <table>
          <thead><tr><th>PERSON</th><th>INVENTORY</th><th>ASKED</th><th>STATUS</th><th /></tr></thead>
          <tbody>
            {data.items.length === 0 && <tr><td colSpan="5" className="empty">No requests.</td></tr>}
            {data.items.map((r) => (
              <tr key={r.id}>
                <td><strong className="normal">{r.userName}</strong>{r.note && <small>“{r.note}”</small>}</td>
                <td>{r.inventoryName}</td>
                <td>{fmtDateTime(r.createdAt)}</td>
                <td><Badge map={REQUEST_STATUS} value={r.status} />{r.decidedByName && <small>{r.decidedByName}{r.decisionNote && `: ${r.decisionNote}`}</small>}</td>
                <td>
                  {r.status === 'pending' && (
                    <span className="row-end">
                      {data.canDecide && r.userId !== user.id && <>
                        <button className="link-btn" title="Approve" aria-label={`Approve ${r.userName}`} onClick={() => act(`/access-requests/${r.id}/approve`, {})}><Check size={17} /></button>
                        <button className="link-btn" title="Reject" aria-label={`Reject ${r.userName}`} onClick={() => { const note = window.prompt('Reason (optional)'); if (note !== null) act(`/access-requests/${r.id}/reject`, { note: note || null }); }}><X size={17} /></button>
                      </>}
                      {r.userId === user.id && <button className="text-link" onClick={() => act(`/access-requests/${r.id}/cancel`)}>Cancel</button>}
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

export function Inventories({ buildings, onSelect, onChanged }) {
  const { can } = useAuth();
  const [archived, setArchived] = useState(false);
  const { list, error, reload } = useInventories(archived);
  const [creating, setCreating] = useState(null);
  const [openId, setOpenId] = useState(null);
  const [moving, setMoving] = useState(false);
  const [version, setVersion] = useState(0);
  const changed = () => { reload(); setVersion((v) => v + 1); onChanged(); };
  const active = list.filter((i) => !i.archivedAt);
  return (
    <>
      <div className="metrics">
        {[['Inventories', active.length], ['Items placed', active.reduce((n, i) => n + i.assetCount, 0)],
          ['Pending transfers', active.reduce((n, i) => n + i.pendingTransfers, 0)], ['Need approval', active.filter((i) => i.requiresApproval).length]].map(([label, value]) => (
          <div className="metric" key={label}><div className="metric-top"><span>{label}</span></div><strong>{value}</strong></div>
        ))}
      </div>
      <section className="panel">
        <div className="panel-head wrap">
          <div><h2>Inventories</h2><p>Hubs and the stock rooms inside them. Each item sits in one inventory.</p></div>
          <div className="row-end">
            {can('inventory:transfer') && <button className="primary" onClick={() => setMoving(true)}><ArrowRightLeft size={16} /> Transfer</button>}
            {can('inventory:manage') && <button className="outline" onClick={() => setCreating({})}><Plus size={16} /> New inventory</button>}
          </div>
        </div>
        <label className="check pad-x"><input type="checkbox" checked={archived} onChange={(e) => setArchived(e.target.checked)} /> Show archived</label>
        <ErrorBanner error={error} />
        <div className="table-scroll">
          <table>
            <thead><tr><th>INVENTORY</th><th>KIND</th><th>CUSTODIAN</th><th>ITEMS</th><th>PENDING</th><th /></tr></thead>
            <tbody>
              {list.length === 0 && <tr><td colSpan="6" className="empty">No inventories yet.</td></tr>}
              {list.map((i) => (
                <tr key={i.id} className="clickable" tabIndex={0} onClick={() => setOpenId(i.id)} onKeyDown={(e) => e.key === 'Enter' && setOpenId(i.id)}>
                  <td><span className="tree-name" style={{ paddingLeft: `${i.depth * 18}px` }}><strong className="normal">{i.name}</strong>{i.archivedAt && <span className="tag">Archived</span>}{i.requiresApproval && <span className="tag">Approval</span>}</span></td>
                  <td>{INVENTORY_KINDS[i.kind] ?? i.kind}</td>
                  <td>{i.custodianName || <span className="muted">—</span>}</td>
                  <td>{i.assetCount}</td>
                  <td>{i.pendingTransfers || <span className="muted">—</span>}</td>
                  <td onClick={(e) => e.stopPropagation()}>{can('inventory:manage') && !i.archivedAt && <button className="text-link" onClick={() => setCreating({ parentId: i.id })}><Plus size={13} /> Inside</button>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <Transfers version={version} onChanged={changed} onSelect={onSelect} />
      <AccessRequests version={version} onChanged={changed} />
      {creating && <InventoryForm parentId={creating.parentId} inventories={list} buildings={buildings} onClose={() => setCreating(null)} onSaved={() => { setCreating(null); changed(); }} />}
      {openId && <InventoryDetail id={openId} inventories={list} buildings={buildings} onClose={() => setOpenId(null)} onChanged={changed} onSelect={onSelect} />}
      {moving && <TransferForm inventories={list} onClose={() => setMoving(false)} onDone={changed} />}
    </>
  );
}
