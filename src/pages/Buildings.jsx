import React, { useState } from 'react';
import { Building2, Plus, Trash2 } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';

function AddBuilding({ onClose, onSaved }) {
  const [form, setForm] = useState({ name: '', address: '' });
  const [error, setError] = useState(null);
  async function submit(e) {
    e.preventDefault();
    try {
      await api('/buildings', { method: 'POST', body: { name: form.name, address: form.address || null } });
      onSaved();
    } catch (err) { setError(err); }
  }
  return (
    <Modal title="Add Building" subtitle="A facility, floor or site that holds equipment" onClose={onClose}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name"><input required value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="Building A" /></Field>
          <Field label="Address"><input value={form.address} onChange={(e) => setForm({ ...form, address: e.target.value })} placeholder="Optional" /></Field>
        </div>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><Plus size={16} /> Add Building</button>
        </div>
      </form>
    </Modal>
  );
}

function BuildingCard({ building, canEdit, onChanged }) {
  const [name, setName] = useState('');
  const [error, setError] = useState(null);

  async function addLocation(e) {
    e.preventDefault();
    try {
      await api(`/buildings/${building.id}/locations`, { method: 'POST', body: { name } });
      setName('');
      setError(null);
      onChanged();
    } catch (err) { setError(err); }
  }

  async function remove(path, label) {
    if (!window.confirm(`Delete ${label}?`)) return;
    try {
      await api(path, { method: 'DELETE' });
      onChanged();
    } catch (err) { setError(err); }
  }

  return (
    <section className="panel building">
      <div className="building-top">
        <div className="building-icon"><Building2 size={24} /></div>
        {canEdit && building.assetCount === 0 && (
          <button className="icon-btn" aria-label={`Delete ${building.name}`} onClick={() => remove(`/buildings/${building.id}`, building.name)}>
            <Trash2 size={16} />
          </button>
        )}
      </div>
      <h2>{building.name}</h2>
      <p>{building.address || 'Equipment and workstation inventory'}</p>
      <strong>{building.assetCount} assets</strong>
      <ErrorBanner error={error} />
      <div className="location-list">
        {building.locations.length === 0 && <div><span>No locations yet</span></div>}
        {building.locations.map((l) => (
          <div key={l.id}>
            <span>{l.name}</span>
            <span className="row-end">
              <b>{l.assetCount}</b>
              {canEdit && l.assetCount === 0 && (
                <button className="link-btn" aria-label={`Delete ${l.name}`} onClick={() => remove(`/locations/${l.id}`, l.name)}><Trash2 size={14} /></button>
              )}
            </span>
          </div>
        ))}
      </div>
      {canEdit && (
        <form className="inline-form" onSubmit={addLocation}>
          <input required value={name} onChange={(e) => setName(e.target.value)} placeholder="New location or station" aria-label="New location name" />
          <button className="outline"><Plus size={15} /> Add</button>
        </form>
      )}
    </section>
  );
}

export function Buildings({ buildings, onChanged }) {
  const { can } = useAuth();
  const [adding, setAdding] = useState(false);
  const canEdit = can('sites:write');
  return (
    <>
      {canEdit && (
        <div className="toolbar"><button className="outline" onClick={() => setAdding(true)}><Plus size={16} /> Add Building</button></div>
      )}
      {buildings.length === 0 && <section className="panel"><p className="empty">No buildings yet.</p></section>}
      <div className="building-grid">
        {buildings.map((b) => <BuildingCard key={b.id} building={b} canEdit={canEdit} onChanged={onChanged} />)}
      </div>
      {adding && <AddBuilding onClose={() => setAdding(false)} onSaved={() => { setAdding(false); onChanged(); }} />}
    </>
  );
}
