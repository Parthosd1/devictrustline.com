import React, { useEffect, useRef, useState } from 'react';
import { Check, ChevronsUpDown, Plus } from 'lucide-react';
import { api } from './api.js';
import { useAuth } from './auth.jsx';
import { ErrorBanner, Field, Modal } from './components.jsx';
import { ROLE_LABELS } from './constants.js';

const initials = (name) => name.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join('') || '?';

export function NewWorkspace({ onClose, onCreated }) {
  const [form, setForm] = useState({ name: '', kind: 'personal' });
  const [error, setError] = useState(null);
  async function submit(e) {
    e.preventDefault();
    try { onCreated(await api('/workspaces', { method: 'POST', body: form })); } catch (err) { setError(err); }
  }
  return (
    <Modal title="New workspace" subtitle="A separate space with its own items, people and settings. You will be its admin." onClose={onClose}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name"><input required value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="Home, or your company name" /></Field>
          <Field label="Type">
            <select value={form.kind} onChange={(e) => setForm({ ...form, kind: e.target.value })}>
              <option value="personal">Personal</option>
              <option value="business">Business</option>
            </select>
          </Field>
        </div>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><Plus size={16} /> Create</button>
        </div>
      </form>
    </Modal>
  );
}

// The workspace card at the bottom of the sidebar: shows the current one and switches.
export function WorkspaceSwitcher({ onSwitched }) {
  const { user, switchWorkspace } = useAuth();
  const [open, setOpen] = useState(false);
  const [spaces, setSpaces] = useState([]);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState(null);
  const ref = useRef(null);

  useEffect(() => { if (open) api('/workspaces').then((r) => setSpaces(r.items)).catch(setError); }, [open]);
  useEffect(() => {
    if (!open) return undefined;
    const close = (e) => { if (!ref.current?.contains(e.target)) setOpen(false); };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open]);

  async function pick(id) {
    setOpen(false);
    if (id === user.organization.id) return;
    try { await switchWorkspace(id); onSwitched(); } catch (err) { setError(err); }
  }

  return (
    <div className="sidebar-bottom" ref={ref}>
      <button className="workspace-btn" onClick={() => setOpen(!open)} aria-expanded={open} aria-haspopup="menu">
        <div className="workspace-avatar">{initials(user.organization.name)}</div>
        <div><strong>{user.organization.name}</strong><small>{user.organization.kind === 'personal' ? 'Personal' : 'Business'} · {ROLE_LABELS[user.role]}</small></div>
        <ChevronsUpDown size={15} />
      </button>
      {error && <ErrorBanner error={error} />}
      {open && (
        <div className="workspace-menu" role="menu">
          {spaces.map((s) => (
            <button key={s.id} role="menuitem" onClick={() => pick(s.id)}>
              <span><strong>{s.name}</strong><small>{s.kind === 'personal' ? 'Personal' : 'Business'} · {ROLE_LABELS[s.role]}</small></span>
              {s.current && <Check size={15} />}
            </button>
          ))}
          <button role="menuitem" className="add" onClick={() => { setOpen(false); setCreating(true); }}><Plus size={15} /> New workspace</button>
        </div>
      )}
      {creating && <NewWorkspace onClose={() => setCreating(false)} onCreated={async (w) => { setCreating(false); await pick(w.id); }} />}
    </div>
  );
}
