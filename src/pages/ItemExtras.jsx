import React, { useCallback, useEffect, useState } from 'react';
import { FileText, Paperclip, Pencil, Plus, ShieldCheck, Trash2, Upload } from 'lucide-react';
import { api, fileUrl } from '../api.js';
import { ErrorBanner, Field, Modal } from '../components.jsx';
import { ATTACHMENT_KINDS, WARRANTY_KINDS, WARRANTY_STATES } from '../constants.js';

const STATE_CLASS = { active: 'deployed', expiring_soon: 'maintenance', expired: 'missing', unknown: 'retired' };

export function WarrantyBadge({ status }) {
  if (!status) return null;
  return <span className={`status ${STATE_CLASS[status]}`}><i />{WARRANTY_STATES[status]}</span>;
}

const size = (b) => (b > 1048576 ? `${(b / 1048576).toFixed(1)} MB` : `${Math.ceil(b / 1024)} KB`);

export function Attachments({ asset, writable, onChanged }) {
  const [items, setItems] = useState(null);
  const [kind, setKind] = useState('photo');
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const load = useCallback(() => api(`/assets/${asset.id}/attachments`).then((r) => setItems(r.items)).catch(setError), [asset.id]);
  useEffect(() => { load(); }, [load]);

  async function upload(e) {
    const file = e.target.files[0];
    e.target.value = '';
    if (!file) return;
    const form = new FormData();
    form.append('kind', kind);
    form.append('file', file);
    setBusy(true);
    try {
      await api(`/assets/${asset.id}/attachments`, { method: 'POST', form });
      setError(null);
      load();
      onChanged?.();
    } catch (err) { setError(err); }
    setBusy(false);
  }

  async function remove(f) {
    if (!window.confirm(`Delete ${f.filename}?`)) return;
    try { await api(`/attachments/${f.id}`, { method: 'DELETE' }); load(); onChanged?.(); } catch (err) { setError(err); }
  }

  const photos = items?.filter((f) => f.contentType.startsWith('image/') && f.contentType !== 'image/heic') ?? [];
  const others = items?.filter((f) => !photos.includes(f)) ?? [];
  return (
    <>
      <h3 className="section-title"><Paperclip size={14} /> Photos & documents {items?.length ? `(${items.length})` : ''}</h3>
      <ErrorBanner error={error} />
      {photos.length > 0 && (
        <div className="photo-grid">
          {photos.map((f) => (
            <figure key={f.id}>
              <a href={fileUrl(f.id)} target="_blank" rel="noreferrer"><img src={fileUrl(f.id)} alt={f.filename} loading="lazy" /></a>
              <figcaption>{ATTACHMENT_KINDS[f.kind]}{writable && <button className="link-btn" aria-label={`Delete ${f.filename}`} onClick={() => remove(f)}><Trash2 size={13} /></button>}</figcaption>
            </figure>
          ))}
        </div>
      )}
      {others.length > 0 && (
        <ul className="file-list">
          {others.map((f) => (
            <li key={f.id}>
              <FileText size={16} />
              <a href={fileUrl(f.id)} target="_blank" rel="noreferrer">{f.filename}</a>
              <small>{ATTACHMENT_KINDS[f.kind]} · {size(f.size)}</small>
              {writable && <button className="link-btn" aria-label={`Delete ${f.filename}`} onClick={() => remove(f)}><Trash2 size={14} /></button>}
            </li>
          ))}
        </ul>
      )}
      {items?.length === 0 && <p className="muted small">No photos, receipts or documents yet.</p>}
      {writable && (
        <div className="upload-row">
          <select value={kind} onChange={(e) => setKind(e.target.value)} aria-label="File kind">
            {Object.entries(ATTACHMENT_KINDS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
          </select>
          <label className={`outline file-btn ${busy ? 'busy' : ''}`}>
            <Upload size={14} /> {busy ? 'Uploading…' : 'Upload'}
            <input type="file" accept="image/*,application/pdf" capture={kind === 'photo' ? 'environment' : undefined} onChange={upload} disabled={busy} />
          </label>
          <small className="muted">Photos or PDFs, up to 10 MB.</small>
        </div>
      )}
    </>
  );
}

function WarrantyForm({ asset, warranty, files, onClose, onSaved }) {
  const [form, setForm] = useState({
    kind: warranty?.kind ?? 'manufacturer', provider: warranty?.provider ?? '', startsOn: warranty?.startsOn ?? asset.purchaseDate ?? '',
    endsOn: warranty?.endsOn ?? '', coverage: warranty?.coverage ?? '', claimReference: warranty?.claimReference ?? '',
    proofAttachmentId: warranty?.proofAttachmentId ?? '',
  });
  const [error, setError] = useState(null);
  const set = (k) => (e) => setForm({ ...form, [k]: e.target.value });
  async function submit(e) {
    e.preventDefault();
    const body = Object.fromEntries(Object.entries(form).map(([k, v]) => [k, v === '' ? null : v]));
    body.kind = form.kind;
    try {
      await api(warranty ? `/warranties/${warranty.id}` : `/assets/${asset.id}/warranties`, { method: warranty ? 'PATCH' : 'POST', body });
      onSaved();
    } catch (err) { setError(err); }
  }
  return (
    <Modal title={warranty ? 'Edit warranty' : 'Add warranty'} subtitle={asset.name} onClose={onClose}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Type">
            <select value={form.kind} onChange={set('kind')}>{Object.entries(WARRANTY_KINDS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}</select>
          </Field>
          <Field label="Provider"><input value={form.provider} onChange={set('provider')} placeholder="Manufacturer, store or insurer" /></Field>
          <Field label="Starts"><input type="date" value={form.startsOn} onChange={set('startsOn')} /></Field>
          <Field label="Ends"><input type="date" value={form.endsOn} onChange={set('endsOn')} /></Field>
          <Field label="What it covers" wide><textarea rows={2} value={form.coverage} onChange={set('coverage')} /></Field>
          <Field label="Claim or policy number"><input value={form.claimReference} onChange={set('claimReference')} /></Field>
          <Field label="Proof of purchase">
            <select value={form.proofAttachmentId} onChange={set('proofAttachmentId')}>
              <option value="">None</option>
              {files.map((f) => <option key={f.id} value={f.id}>{f.filename}</option>)}
            </select>
          </Field>
        </div>
        {files.length === 0 && <p className="hint">Upload a receipt under Photos & documents to attach it as proof.</p>}
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary">Save</button>
        </div>
      </form>
    </Modal>
  );
}

export function Warranties({ asset, writable, onChanged }) {
  const [items, setItems] = useState(null);
  const [files, setFiles] = useState([]);
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState(null);
  const load = useCallback(() => api(`/assets/${asset.id}/warranties`).then((r) => setItems(r.items)).catch(setError), [asset.id]);
  useEffect(() => { load(); }, [load]);

  async function openForm(w) {
    const r = await api(`/assets/${asset.id}/attachments`).catch(() => ({ items: [] }));
    setFiles(r.items);
    setEditing(w);
  }
  async function remove(w) {
    if (!window.confirm('Delete this warranty?')) return;
    try { await api(`/warranties/${w.id}`, { method: 'DELETE' }); load(); onChanged?.(); } catch (err) { setError(err); }
  }

  return (
    <>
      <h3 className="section-title"><ShieldCheck size={14} /> Warranties {items?.length ? `(${items.length})` : ''}</h3>
      <ErrorBanner error={error} />
      {items?.length === 0 && <p className="muted small">No warranty recorded.</p>}
      <ul className="warranty-list">
        {items?.map((w) => (
          <li key={w.id}>
            <div>
              <strong>{WARRANTY_KINDS[w.kind]}{w.provider ? ` · ${w.provider}` : ''}</strong>
              <small>
                {w.startsOn ? `${w.startsOn} → ` : ''}{w.endsOn ?? 'end date unknown'}
                {w.daysLeft != null && w.daysLeft >= 0 && ` · ${w.daysLeft} days left`}
                {w.claimReference && ` · Ref ${w.claimReference}`}
              </small>
              {w.coverage && <small>{w.coverage}</small>}
              {w.proofAttachmentId && <small>Proof: <a href={fileUrl(w.proofAttachmentId)} target="_blank" rel="noreferrer">{w.proofFilename}</a></small>}
            </div>
            <WarrantyBadge status={w.status} />
            {writable && (
              <span className="row-end">
                <button className="link-btn" aria-label="Edit warranty" onClick={() => openForm(w)}><Pencil size={14} /></button>
                <button className="link-btn" aria-label="Delete warranty" onClick={() => remove(w)}><Trash2 size={14} /></button>
              </span>
            )}
          </li>
        ))}
      </ul>
      {writable && <button className="outline small-btn" onClick={() => openForm(null)}><Plus size={14} /> Add warranty</button>}
      {editing !== false && (
        <WarrantyForm asset={asset} warranty={editing} files={files} onClose={() => setEditing(false)}
          onSaved={() => { setEditing(false); load(); onChanged?.(); }} />
      )}
    </>
  );
}
