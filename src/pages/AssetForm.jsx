import React, { useState } from 'react';
import { Plus, Save } from 'lucide-react';
import { api } from '../api.js';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { ASSET_STATUSES, ASSET_TYPES } from '../constants.js';

const EMPTY = {
  name: '', type: 'Laptop', serial: '', manufacturer: '', model: '', status: 'Available',
  buildingId: '', locationId: '', purchaseDate: '', warrantyExpires: '', notes: '',
};

// Create (asset is null) or edit an asset.
export function AssetForm({ asset, buildings, onClose, onSaved }) {
  const [form, setForm] = useState(() => (asset
    ? Object.fromEntries(Object.keys(EMPTY).map((k) => [k, asset[k] ?? '']))
    : { ...EMPTY, buildingId: buildings[0]?.id ?? '' }));
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const locations = buildings.find((b) => b.id === form.buildingId)?.locations ?? [];
  const set = (key) => (e) => {
    const value = e.target.value;
    setForm((f) => ({ ...f, [key]: value, ...(key === 'buildingId' && { locationId: '' }) }));
  };

  async function submit(e) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const body = Object.fromEntries(Object.entries(form).map(([k, v]) => [k, v === '' ? null : v]));
    body.name = form.name;
    try {
      const saved = asset
        ? await api(`/assets/${asset.id}`, { method: 'PATCH', body })
        : await api('/assets', { method: 'POST', body });
      onSaved(saved);
    } catch (err) {
      setError(err);
      setBusy(false);
    }
  }

  return (
    <Modal title={asset ? `Edit ${asset.assetTag}` : 'Register New Asset'}
      subtitle={asset ? asset.name : 'Add equipment to your inventory. An asset tag is assigned automatically.'} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Asset Name"><input required value={form.name} onChange={set('name')} placeholder="Dell Latitude 5440" /></Field>
          <Field label="Asset Type">
            <select value={form.type} onChange={set('type')}>{ASSET_TYPES.map((t) => <option key={t}>{t}</option>)}</select>
          </Field>
          <Field label="Serial Number"><input value={form.serial} onChange={set('serial')} placeholder="Serial number" /></Field>
          <Field label="Status">
            <select value={form.status} onChange={set('status')}>{ASSET_STATUSES.map((s) => <option key={s}>{s}</option>)}</select>
          </Field>
          <Field label="Manufacturer"><input value={form.manufacturer} onChange={set('manufacturer')} placeholder="Dell" /></Field>
          <Field label="Model"><input value={form.model} onChange={set('model')} placeholder="Latitude 5440" /></Field>
          <Field label="Building">
            <select value={form.buildingId} onChange={set('buildingId')}>
              <option value="">No building</option>
              {buildings.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
            </select>
          </Field>
          <Field label="Location / Station">
            <select value={form.locationId} onChange={set('locationId')} disabled={!form.buildingId}>
              <option value="">Unassigned</option>
              {locations.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
            </select>
          </Field>
          <Field label="Purchase Date"><input type="date" value={form.purchaseDate} onChange={set('purchaseDate')} /></Field>
          <Field label="Warranty Expires"><input type="date" value={form.warrantyExpires} onChange={set('warrantyExpires')} /></Field>
          <Field label="Notes" wide><textarea rows={3} value={form.notes} onChange={set('notes')} /></Field>
        </div>
        {buildings.length === 0 && <p className="hint">Tip: add buildings and stations on the Buildings page to place assets.</p>}
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button type="submit" className="primary" disabled={busy}>
            {asset ? <><Save size={16} /> Save Changes</> : <><Plus size={16} /> Register Asset</>}
          </button>
        </div>
      </form>
    </Modal>
  );
}
