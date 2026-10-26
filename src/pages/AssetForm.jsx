import React, { useEffect, useState } from 'react';
import { Plus, Save } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { nouns, useCatalog } from '../catalog.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { ASSET_STATUSES, CONDITIONS } from '../constants.js';

const EMPTY = {
  name: '', type: '', assetTag: '', serial: '', manufacturer: '', model: '', status: 'Available', condition: '', ownerId: '',
  buildingId: '', locationId: '', parentId: '', purchaseDate: '', purchasePrice: '', currency: '', seller: '',
  returnDeadline: '', warrantyExpires: '', maintenanceDue: '', notes: '',
};

// One custom field's input, by its type.
function CustomInput({ field, value, onChange }) {
  const id = `custom-${field.id}`;
  const label = <label htmlFor={id}>{field.label}</label>;
  if (field.type === 'boolean') {
    return (
      <div className="field">
        {label}
        <select id={id} value={value === true ? 'yes' : value === false ? 'no' : ''} onChange={(e) => onChange(e.target.value === '' ? null : e.target.value === 'yes')}>
          <option value="">—</option><option value="yes">Yes</option><option value="no">No</option>
        </select>
      </div>
    );
  }
  if (field.type === 'select') {
    return (
      <div className="field">
        {label}
        <select id={id} value={value ?? ''} onChange={(e) => onChange(e.target.value || null)}>
          <option value="">—</option>{field.options.map((o) => <option key={o}>{o}</option>)}
        </select>
      </div>
    );
  }
  const type = { number: 'number', date: 'date' }[field.type] || 'text';
  return (
    <div className="field">
      {label}
      <input id={id} type={type} step={type === 'number' ? 'any' : undefined} value={value ?? ''}
        onChange={(e) => onChange(e.target.value === '' ? null : type === 'number' ? Number(e.target.value) : e.target.value)} />
    </div>
  );
}

// Create (asset is null) or edit an item.
export function AssetForm({ asset, buildings, assets, onClose, onSaved }) {
  const { user, can } = useAuth();
  const { categories, fields } = useCatalog();
  const n = nouns(user.organization.kind);
  const [form, setForm] = useState(() => (asset
    ? Object.fromEntries(Object.keys(EMPTY).map((k) => [k, asset[k] ?? '']))
    : { ...EMPTY, type: categories[0]?.name ?? '', buildingId: buildings[0]?.id ?? '' }));
  const [tags, setTags] = useState((asset?.tags ?? []).join(', '));
  const [custom, setCustom] = useState(asset?.custom ?? {});
  const [members, setMembers] = useState(null);
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const [more, setMore] = useState(!!asset);

  useEffect(() => {
    if (can('users:read')) api('/users').then((r) => setMembers(r.items.filter((u) => u.isActive))).catch(() => {});
    if (!asset) api('/settings').then((s) => s.currency && setForm((f) => ({ ...f, currency: f.currency || s.currency }))).catch(() => {});
  }, [asset, can]);
  useEffect(() => {
    if (!form.type && categories.length) setForm((f) => ({ ...f, type: categories[0].name }));
  }, [categories, form.type]);

  const locations = buildings.find((b) => b.id === form.buildingId)?.locations ?? [];
  const parents = assets.filter((a) => a.id !== asset?.id && a.status !== 'Retired');
  const placedByParent = !!form.parentId && form.parentId !== (asset?.parentId ?? '');
  const category = categories.find((c) => c.name.toLowerCase() === form.type.toLowerCase());
  const shownFields = fields.filter((f) => !f.categoryId || f.categoryId === category?.id);
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
    if (!asset && !form.assetTag) delete body.assetTag;
    if (asset && !form.assetTag) delete body.assetTag;
    if (!body.status) delete body.status;
    body.tags = tags.split(',').map((t) => t.trim()).filter(Boolean);
    // Only send values of fields that apply, and clear removed ones.
    body.custom = Object.fromEntries(Object.entries(custom).filter(([id]) => fields.some((f) => f.id === id)));
    // Joining a parent: let the server place this asset where its parent is.
    if (placedByParent) { delete body.buildingId; delete body.locationId; }
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
    <Modal title={asset ? `Edit ${asset.assetTag}` : `Add ${n.one}`}
      subtitle={asset ? asset.name : 'Only a name and category are needed. A tag is assigned automatically unless you enter one.'} onClose={onClose} wide>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name"><input required value={form.name} onChange={set('name')} placeholder={user.organization.kind === 'personal' ? 'Espresso machine' : 'Dell Latitude 5440'} /></Field>
          <Field label="Category">
            <select required value={form.type} onChange={set('type')}>
              {categories.map((c) => <option key={c.id}>{c.name}</option>)}
              {form.type && !category && <option>{form.type}</option>}
            </select>
          </Field>
          <Field label="Manufacturer"><input value={form.manufacturer} onChange={set('manufacturer')} /></Field>
          <Field label="Model"><input value={form.model} onChange={set('model')} /></Field>
          <Field label="Serial number"><input value={form.serial} onChange={set('serial')} /></Field>
          <Field label="Tag"><input value={form.assetTag} onChange={set('assetTag')} placeholder="Assigned automatically" maxLength={40} /></Field>
          <Field label="Purchase date"><input type="date" value={form.purchaseDate} onChange={set('purchaseDate')} /></Field>
          <Field label="Warranty ends"><input type="date" value={form.warrantyExpires} onChange={set('warrantyExpires')} /></Field>
        </div>
        <button type="button" className="text-link more-toggle" aria-expanded={more} onClick={() => setMore(!more)}>
          {more ? 'Fewer details' : 'More details: price, status, location, dates, tags…'}
        </button>
        {more && (
          <div className="form-grid">
            <Field label="Price"><input type="number" min="0" step="0.01" value={form.purchasePrice} onChange={set('purchasePrice')} /></Field>
            <Field label="Currency"><input value={form.currency} onChange={set('currency')} maxLength={3} placeholder="USD" /></Field>
            <Field label="Bought from"><input value={form.seller} onChange={set('seller')} /></Field>
            <Field label="Return by"><input type="date" value={form.returnDeadline} onChange={set('returnDeadline')} /></Field>
            <Field label="Status">
              <select value={form.status} onChange={set('status')}>{ASSET_STATUSES.map((s) => <option key={s}>{s}</option>)}</select>
            </Field>
            <Field label="Condition">
              <select value={form.condition} onChange={set('condition')}><option value="">—</option>{CONDITIONS.map((c) => <option key={c}>{c}</option>)}</select>
            </Field>
            <Field label="Next maintenance"><input type="date" value={form.maintenanceDue} onChange={set('maintenanceDue')} /></Field>
            {members && (
              <Field label="Owner">
                <select value={form.ownerId} onChange={set('ownerId')}>
                  <option value="">Nobody in particular</option>
                  {members.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
                </select>
              </Field>
            )}
            <Field label={n.place}>
              <select value={form.buildingId} onChange={set('buildingId')} disabled={placedByParent}>
                <option value="">None</option>
                {buildings.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
              </select>
            </Field>
            <Field label={n.spot}>
              <select value={form.locationId} onChange={set('locationId')} disabled={!form.buildingId || placedByParent}>
                <option value="">Unassigned</option>
                {locations.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
              </select>
            </Field>
            <Field label="Part of" wide>
              <select value={form.parentId} onChange={set('parentId')}>
                <option value="">Standalone</option>
                {parents.map((a) => <option key={a.id} value={a.id}>{a.assetTag} · {a.name}{a.locationName ? ` · ${a.locationName}` : ''}</option>)}
              </select>
            </Field>
            <Field label="Tags (separate with commas)" wide><input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="kitchen, insured" /></Field>
            {shownFields.map((f) => (
              <CustomInput key={f.id} field={f} value={custom[f.id]} onChange={(v) => setCustom((c) => ({ ...c, [f.id]: v }))} />
            ))}
            <Field label="Notes" wide><textarea rows={3} value={form.notes} onChange={set('notes')} /></Field>
          </div>
        )}
        {placedByParent && <p className="hint">This {n.one} will move to its parent's place and follow the parent when it moves.</p>}
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button type="submit" className="primary" disabled={busy}>
            {asset ? <><Save size={16} /> Save changes</> : <><Plus size={16} /> Add {n.one}</>}
          </button>
        </div>
      </form>
    </Modal>
  );
}
