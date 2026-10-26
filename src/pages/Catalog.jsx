import React, { useState } from 'react';
import { Pencil, Plus, Trash2 } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { useCatalog } from '../catalog.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';

const FIELD_TYPES = { text: 'Text', number: 'Number', date: 'Date', boolean: 'Yes / no', select: 'Choice from a list' };

function useAction(reload) {
  const [error, setError] = useState(null);
  const run = async (fn) => {
    try { await fn(); setError(null); await reload(); } catch (err) { setError(err); }
  };
  return [error, run];
}

function NameList({ title, blurb, items, onAdd, onRename, onDelete, editable, deleteHint }) {
  const [name, setName] = useState('');
  return (
    <section className="panel">
      <div className="panel-head"><div><h2>{title} <span className="count">{items.length}</span></h2><p>{blurb}</p></div></div>
      <ul className="name-list">
        {items.map((c) => (
          <li key={c.id}>
            <span>{c.name}</span>
            <small>{c.itemCount} item{c.itemCount === 1 ? '' : 's'}</small>
            {editable && (
              <span className="row-end">
                <button className="link-btn" aria-label={`Rename ${c.name}`} onClick={() => {
                  const next = window.prompt(`Rename "${c.name}" to:`, c.name);
                  if (next && next.trim() && next !== c.name) onRename(c, next.trim());
                }}><Pencil size={15} /></button>
                <button className="link-btn" aria-label={`Delete ${c.name}`} title={c.itemCount ? deleteHint : 'Delete'} onClick={() => {
                  if (window.confirm(`Delete "${c.name}"?`)) onDelete(c);
                }}><Trash2 size={15} /></button>
              </span>
            )}
          </li>
        ))}
        {items.length === 0 && <li className="muted">None yet.</li>}
      </ul>
      {editable && onAdd && (
        <form className="inline-form pad" onSubmit={(e) => { e.preventDefault(); if (name.trim()) { onAdd(name.trim()); setName(''); } }}>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder={`New ${title.toLowerCase().replace(/s$/, '')}`} aria-label={`New ${title}`} maxLength={80} />
          <button className="outline"><Plus size={15} /> Add</button>
        </form>
      )}
    </section>
  );
}

function FieldForm({ field, categories, onClose, onSaved }) {
  const [form, setForm] = useState({
    label: field?.label ?? '', type: field?.type ?? 'text', options: (field?.options ?? []).join('\n'), categoryId: field?.categoryId ?? '',
  });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.value });
  async function submit(e) {
    e.preventDefault();
    const body = { label: form.label, categoryId: form.categoryId || null };
    if (form.type === 'select') body.options = form.options.split('\n').map((o) => o.trim()).filter(Boolean);
    if (!field) body.type = form.type;
    try {
      await api(field ? `/custom-fields/${field.id}` : '/custom-fields', { method: field ? 'PATCH' : 'POST', body });
      onSaved();
    } catch (err) { setError(err); }
  }
  return (
    <Modal title={field ? `Edit ${field.label}` : 'New field'} subtitle="Fields appear on every item, or only on items in one category." onClose={onClose}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Label"><input required maxLength={80} value={form.label} onChange={set('label')} placeholder="Color, RAM, Insurance policy…" /></Field>
          <Field label="Type">
            <select value={form.type} onChange={set('type')} disabled={!!field}>
              {Object.entries(FIELD_TYPES).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
            </select>
          </Field>
          <Field label="Shown for" wide>
            <select value={form.categoryId} onChange={set('categoryId')}>
              <option value="">Every category</option>
              {categories.map((c) => <option key={c.id} value={c.id}>{c.name} only</option>)}
            </select>
          </Field>
          {form.type === 'select' && (
            <Field label="Choices (one per line)" wide><textarea rows={4} required value={form.options} onChange={set('options')} /></Field>
          )}
        </div>
        {field && <p className="hint">A field's type cannot change after it is created, so existing values stay valid.</p>}
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary">Save</button>
        </div>
      </form>
    </Modal>
  );
}

export function Catalog() {
  const { can } = useAuth();
  const { categories, tags, fields, reload } = useCatalog();
  const [error, run] = useAction(reload);
  const [editingField, setEditingField] = useState(null);
  const editable = can('catalog:manage');

  return (
    <>
      <ErrorBanner error={error} />
      {!editable && <div className="notice">Only admins and managers can change categories, tags and fields.</div>}
      <div className="two-col">
        <NameList title="Categories" blurb="Every item belongs to one. Renaming a category renames it on every item." items={categories} editable={editable}
          deleteHint="Move its items to another category first"
          onAdd={(name) => run(() => api('/categories', { method: 'POST', body: { name } }))}
          onRename={(c, name) => run(() => api(`/categories/${c.id}`, { method: 'PATCH', body: { name } }))}
          onDelete={(c) => run(() => api(`/categories/${c.id}`, { method: 'DELETE' }))} />
        <NameList title="Tags" blurb="Added from an item's form as you use them. Rename or remove them here." items={tags} editable={editable}
          onRename={(t, name) => run(() => api(`/tags/${t.id}`, { method: 'PATCH', body: { name } }))}
          onDelete={(t) => run(() => api(`/tags/${t.id}`, { method: 'DELETE' }))} />
      </div>
      <section className="panel">
        <div className="panel-head wrap">
          <div><h2>Custom fields <span className="count">{fields.length}</span></h2><p>Your own details on items, checked by type.</p></div>
          {editable && <button className="outline" onClick={() => setEditingField({})}><Plus size={16} /> New field</button>}
        </div>
        <div className="table-scroll">
          <table>
            <thead><tr><th>LABEL</th><th>TYPE</th><th>SHOWN FOR</th>{editable && <th />}</tr></thead>
            <tbody>
              {fields.length === 0 && <tr><td colSpan="4" className="empty">No custom fields yet.</td></tr>}
              {fields.map((f) => (
                <tr key={f.id}>
                  <td><strong className="normal">{f.label}</strong>{f.type === 'select' && <small>{f.options.join(', ')}</small>}</td>
                  <td>{FIELD_TYPES[f.type]}</td>
                  <td>{f.categoryName || 'Every category'}</td>
                  {editable && (
                    <td className="row-end">
                      <button className="link-btn" aria-label={`Edit ${f.label}`} onClick={() => setEditingField(f)}><Pencil size={15} /></button>
                      <button className="link-btn" aria-label={`Delete ${f.label}`} onClick={() => {
                        if (window.confirm(`Delete "${f.label}"? Its values are removed from every item.`)) run(() => api(`/custom-fields/${f.id}`, { method: 'DELETE' }));
                      }}><Trash2 size={15} /></button>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      {editingField && (
        <FieldForm field={editingField.id ? editingField : null} categories={categories} onClose={() => setEditingField(null)}
          onSaved={() => { setEditingField(null); reload(); }} />
      )}
    </>
  );
}
