import React, { useCallback, useEffect, useState } from 'react';
import { Plus } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { AssetTable, ErrorBanner, Field, Modal } from '../components.jsx';

function AddPerson({ onClose, onSaved }) {
  const [form, setForm] = useState({ name: '', email: '', employeeId: '', department: '' });
  const [error, setError] = useState(null);
  const set = (key) => (e) => setForm({ ...form, [key]: e.target.value });
  async function submit(e) {
    e.preventDefault();
    try {
      await api('/people', { method: 'POST', body: form });
      onSaved();
    } catch (err) { setError(err); }
  }
  return (
    <Modal title="Add Person" subtitle="Someone equipment can be assigned to. They do not need a login." onClose={onClose}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name"><input required value={form.name} onChange={set('name')} /></Field>
          <Field label="Email"><input type="email" value={form.email} onChange={set('email')} placeholder="Optional" /></Field>
          <Field label="Employee ID"><input value={form.employeeId} onChange={set('employeeId')} placeholder="Optional" /></Field>
          <Field label="Department"><input value={form.department} onChange={set('department')} placeholder="Optional" /></Field>
        </div>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><Plus size={16} /> Add Person</button>
        </div>
      </form>
    </Modal>
  );
}

function PersonAssets({ person, onClose, onSelect }) {
  const [items, setItems] = useState(null);
  const [error, setError] = useState(null);
  useEffect(() => {
    api(`/assets?personId=${person.id}&limit=500`).then((r) => setItems(r.items)).catch(setError);
  }, [person.id]);
  return (
    <Modal title={person.name} subtitle={[person.department, person.email].filter(Boolean).join(' · ') || 'Assigned equipment'} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <AssetTable assets={items ?? []} loading={items === null} onSelect={(a) => { onClose(); onSelect(a); }} />
    </Modal>
  );
}

export function People({ onSelect, version }) {
  const { can } = useAuth();
  const [people, setPeople] = useState(null);
  const [error, setError] = useState(null);
  const [adding, setAdding] = useState(false);
  const [viewing, setViewing] = useState(null);
  const editable = can('people:write');

  const load = useCallback(() => api('/people').then((r) => setPeople(r.items)).catch(setError), []);
  useEffect(() => { load(); }, [load, version]);

  async function toggle(p) {
    try {
      await api(`/people/${p.id}`, { method: 'PATCH', body: { isActive: !p.isActive } });
      load();
    } catch (err) { setError(err); }
  }

  return (
    <section className="panel">
      <div className="panel-head wrap">
        <div><h2>People <span className="count">{people?.length ?? 0}</span></h2><p>Click someone to see what they have</p></div>
        {editable && <button className="outline" onClick={() => setAdding(true)}><Plus size={16} /> Add Person</button>}
      </div>
      <ErrorBanner error={error} />
      <div className="table-scroll">
        <table>
          <thead><tr><th>NAME</th><th>DEPARTMENT</th><th>EMPLOYEE ID</th><th>ASSETS</th><th>STATUS</th>{editable && <th />}</tr></thead>
          <tbody>
            {people === null && <tr><td colSpan="6" className="empty">Loading…</td></tr>}
            {people?.length === 0 && <tr><td colSpan="6" className="empty">No people yet. Add the staff you hand equipment to.</td></tr>}
            {people?.map((p) => (
              <tr key={p.id} className="clickable" tabIndex={0} onClick={() => setViewing(p)} onKeyDown={(e) => e.key === 'Enter' && setViewing(p)}>
                <td><strong className="normal">{p.name}</strong><small>{p.email || '—'}</small></td>
                <td>{p.department || '—'}</td>
                <td className="mono">{p.employeeId || '—'}</td>
                <td><b>{p.assetCount}</b></td>
                <td><span className={`status ${p.isActive ? 'deployed' : 'retired'}`}><i />{p.isActive ? 'Active' : 'Inactive'}</span></td>
                {editable && (
                  <td><button className="outline" onClick={(e) => { e.stopPropagation(); toggle(p); }}>{p.isActive ? 'Mark Inactive' : 'Reactivate'}</button></td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {adding && <AddPerson onClose={() => setAdding(false)} onSaved={() => { setAdding(false); load(); }} />}
      {viewing && <PersonAssets person={viewing} onClose={() => setViewing(null)} onSelect={onSelect} />}
    </section>
  );
}
