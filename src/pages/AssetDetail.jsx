import React, { useEffect, useState } from 'react';
import { Pencil } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { AssetIcon, ErrorBanner, Modal, StatusBadge } from '../components.jsx';

const LABELS = {
  name: 'Name', type: 'Type', serial: 'Serial', manufacturer: 'Manufacturer', model: 'Model', status: 'Status',
  buildingId: 'Building', locationId: 'Location', notes: 'Notes', purchaseDate: 'Purchase date',
  warrantyExpires: 'Warranty expires', source: 'Source',
};

const formatTime = (iso) => new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

function describe(event) {
  if (event.type === 'created') return 'Registered asset';
  const fields = Object.keys(event.changes);
  if (event.type === 'status_changed') {
    const s = event.changes.status;
    return `Status ${s.from} → ${s.to}${fields.length > 1 ? `, plus ${fields.length - 1} more change(s)` : ''}`;
  }
  return `Updated ${fields.map((f) => LABELS[f] || f).join(', ').toLowerCase()}`;
}

export function AssetDetail({ asset: initial, onClose, onEdit }) {
  const { can } = useAuth();
  const [asset, setAsset] = useState(initial);
  const [events, setEvents] = useState(null);
  const [error, setError] = useState(null);

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

  return (
    <Modal title={asset.name} subtitle={`${asset.assetTag} · ${asset.type}`} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <div className="detail-head">
        <AssetIcon type={asset.type} size={22} />
        <StatusBadge status={asset.status} />
        {can('assets:write') && (
          <button className="outline" onClick={() => onEdit(asset)}><Pencil size={15} /> Edit</button>
        )}
      </div>
      <dl className="detail-grid">
        {rows.map(([label, value]) => (
          <div key={label}><dt>{label}</dt><dd>{value || '—'}</dd></div>
        ))}
        {asset.notes && <div className="span-2"><dt>Notes</dt><dd className="pre">{asset.notes}</dd></div>}
      </dl>
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
