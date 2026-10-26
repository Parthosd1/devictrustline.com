import React, { useEffect, useState } from 'react';
import { ArrowLeftRight, Link2, Pencil, UserCheck, UserMinus } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { AssetIcon, ErrorBanner, Modal, StatusBadge } from '../components.jsx';

const LABELS = {
  name: 'Name', type: 'Type', serial: 'Serial', manufacturer: 'Manufacturer', model: 'Model', status: 'Status',
  buildingId: 'Building', locationId: 'Location', parentId: 'Parent asset', notes: 'Notes', purchaseDate: 'Purchase date',
  warrantyExpires: 'Warranty expires', source: 'Source',
};

const formatTime = (iso) => new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

function describe(event) {
  const c = event.changes;
  const via = c.via ? ' (with its parent)' : '';
  const note = c.note ? `: “${c.note.to}”` : '';
  switch (event.type) {
    case 'created': return 'Registered asset';
    case 'assigned': return `Assigned to ${c.assignedTo.to.name}${via}${note}`;
    case 'returned': return `Returned from ${c.assignedTo.from?.name ?? 'assignment'}${c.status ? `, now ${c.status.to}` : ''}${via}${note}`;
    case 'moved_with_parent': return 'Moved with its parent asset';
    case 'status_changed': {
      const others = Object.keys(c).length - 1;
      return `Status ${c.status.from} → ${c.status.to}${others ? `, plus ${others} more change(s)` : ''}`;
    }
    default: return `Updated ${Object.keys(c).map((f) => LABELS[f] || f).join(', ').toLowerCase()}`;
  }
}

function AssignPanel({ asset, onDone, onCancel }) {
  const [people, setPeople] = useState(null);
  const [form, setForm] = useState({ personId: '', note: '', includeComponents: true, status: 'Available' });
  const [error, setError] = useState(null);
  const returning = !!asset.assignedPersonId;

  useEffect(() => {
    if (!returning) api('/people').then((r) => setPeople(r.items.filter((p) => p.isActive))).catch(setError);
  }, [returning]);

  async function submit(e) {
    e.preventDefault();
    const body = { note: form.note || null, includeComponents: form.includeComponents };
    try {
      const updated = returning
        ? await api(`/assets/${asset.id}/return`, { method: 'POST', body: { ...body, status: form.status } })
        : await api(`/assets/${asset.id}/assign`, { method: 'POST', body: { ...body, personId: form.personId } });
      onDone(updated);
    } catch (err) { setError(err); }
  }

  const set = (key) => (e) => setForm({ ...form, [key]: e.target.type === 'checkbox' ? e.target.checked : e.target.value });
  return (
    <form className="assign-panel" onSubmit={submit}>
      <ErrorBanner error={error} />
      <div className="form-grid">
        {returning ? (
          <div className="field">
            <label htmlFor="return-status">Returned in condition</label>
            <select id="return-status" value={form.status} onChange={set('status')}>
              <option value="Available">Ready to reassign (Available)</option>
              <option value="Maintenance">Needs repair (Maintenance)</option>
            </select>
          </div>
        ) : (
          <div className="field">
            <label htmlFor="assign-person">Assign to</label>
            <select id="assign-person" required value={form.personId} onChange={set('personId')}>
              <option value="">{people === null ? 'Loading…' : 'Choose a person'}</option>
              {people?.map((p) => <option key={p.id} value={p.id}>{p.name}{p.department ? ` · ${p.department}` : ''}</option>)}
            </select>
            {people?.length === 0 && <small className="hint">Add people on the People page first.</small>}
          </div>
        )}
        <div className="field">
          <label htmlFor="assign-note">Note</label>
          <input id="assign-note" value={form.note} onChange={set('note')} placeholder="Optional" />
        </div>
      </div>
      {asset.childCount > 0 && (
        <label className="check">
          <input type="checkbox" checked={form.includeComponents} onChange={set('includeComponents')} />
          Include its {asset.childCount} attached component{asset.childCount === 1 ? '' : 's'}
        </label>
      )}
      <div className="modal-actions">
        <button type="button" className="outline" onClick={onCancel}>Cancel</button>
        <button className="primary">{returning ? 'Check In' : 'Assign'}</button>
      </div>
    </form>
  );
}

export function AssetDetail({ asset: initial, onClose, onEdit, onSelect, onChanged }) {
  const { can } = useAuth();
  const [asset, setAsset] = useState(initial);
  const [events, setEvents] = useState(null);
  const [error, setError] = useState(null);
  const [assigning, setAssigning] = useState(false);

  useEffect(() => {
    Promise.all([api(`/assets/${initial.id}`), api(`/assets/${initial.id}/events`)])
      .then(([a, e]) => { setAsset(a); setEvents(e.items); })
      .catch(setError);
  }, [initial.id]);

  const rows = [
    ['Asset tag', asset.assetTag], ['Type', asset.type], ['Serial', asset.serial],
    ['Manufacturer', asset.manufacturer], ['Model', asset.model],
    ['Building', asset.buildingName], ['Location', asset.locationName],
    ['Purchase date', asset.purchaseDate], ['Warranty expires', asset.warrantyExpires],
  ];
  const writable = can('assets:write');

  return (
    <Modal title={asset.name} subtitle={`${asset.assetTag} · ${asset.type}`} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <div className="detail-head">
        <AssetIcon type={asset.type} size={22} />
        <StatusBadge status={asset.status} />
        <div className="detail-actions">
          {writable && asset.status !== 'Retired' && !assigning && (
            <button className="outline" onClick={() => setAssigning(true)}>
              {asset.assignedPersonId ? <><UserMinus size={15} /> Check In</> : <><UserCheck size={15} /> Assign</>}
            </button>
          )}
          {writable && <button className="outline" onClick={() => onEdit(asset)}><Pencil size={15} /> Edit</button>}
        </div>
      </div>
      {assigning && <AssignPanel asset={asset} onCancel={() => setAssigning(false)} onDone={onChanged} />}
      <div className="relations">
        <div>
          <span className="rel-label"><UserCheck size={14} /> Assigned to</span>
          <strong>{asset.assignedPersonName || 'Nobody'}</strong>
          {asset.assignedAt && <small>since {new Date(asset.assignedAt).toLocaleDateString()}</small>}
        </div>
        <div>
          <span className="rel-label"><Link2 size={14} /> Part of</span>
          {asset.parentId
            ? <button className="link-text" onClick={() => onSelect({ id: asset.parentId, name: asset.parentName, assetTag: asset.parentTag, type: '', status: '' })}>{asset.parentTag} · {asset.parentName}</button>
            : <strong>Standalone</strong>}
        </div>
      </div>
      <dl className="detail-grid">
        {rows.map(([label, value]) => (
          <div key={label}><dt>{label}</dt><dd>{value || '—'}</dd></div>
        ))}
        {asset.notes && <div className="span-2"><dt>Notes</dt><dd className="pre">{asset.notes}</dd></div>}
      </dl>
      {asset.children?.length > 0 && (
        <>
          <h3 className="section-title"><ArrowLeftRight size={14} /> Attached components ({asset.children.length})</h3>
          <div className="component-list">
            {asset.children.map((c) => (
              <button key={c.id} className="component" onClick={() => onSelect(c)}>
                <AssetIcon type={c.type} size={16} />
                <span><strong>{c.name}</strong><small>{c.assetTag} · {c.type}</small></span>
                <StatusBadge status={c.status} />
              </button>
            ))}
          </div>
        </>
      )}
      <h3 className="section-title">History</h3>
      <ol className="timeline">
        {events === null && !error && <li className="muted">Loading…</li>}
        {events?.map((e) => (
          <li key={e.id}>
            <strong>{describe(e)}</strong>
            <small>{e.actor?.name || 'System'} · {formatTime(e.createdAt)}</small>
          </li>
        ))}
      </ol>
    </Modal>
  );
}
