import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Bell, CheckCheck } from 'lucide-react';
import { api } from './api.js';

const EMAIL_STATUS = {
  queued: 'Email queued',
  sent: 'Emailed',
  failed: 'Email could not be delivered',
  unavailable: 'Not emailed: email is not set up',
};

const ago = (iso) => new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

// The bell in the top bar and its panel. Checks for new notifications every minute.
export function NotificationBell({ onOpenAsset, onViewAll }) {
  const [data, setData] = useState({ items: [], unread: 0 });
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  const load = useCallback(() => api('/notifications?limit=30').then(setData).catch(() => {}), []);

  useEffect(() => {
    load();
    const t = setInterval(load, 60000);
    return () => clearInterval(t);
  }, [load]);
  useEffect(() => {
    if (!open) return undefined;
    const close = (e) => { if (!ref.current?.contains(e.target)) setOpen(false); };
    const esc = (e) => e.key === 'Escape' && setOpen(false);
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', esc);
    return () => { document.removeEventListener('mousedown', close); document.removeEventListener('keydown', esc); };
  }, [open]);

  async function read(n) {
    if (!n.readAt) await api(`/notifications/${n.id}/read`, { method: 'POST' }).catch(() => {});
    if (n.assetId) onOpenAsset({ id: n.assetId, name: '', assetTag: '', type: '', status: '' });
    setOpen(false);
    load();
  }

  return (
    <div className="bell" ref={ref}>
      <button className="icon-btn" aria-label={`Notifications${data.unread ? `, ${data.unread} unread` : ''}`} aria-expanded={open}
        onClick={() => { setOpen(!open); if (!open) load(); }}>
        <Bell size={18} />{data.unread > 0 && <span className="badge" aria-hidden="true">{data.unread > 9 ? '9+' : data.unread}</span>}
      </button>
      {open && (
        <div className="bell-panel" role="dialog" aria-label="Notifications">
          <div className="bell-head">
            <strong>Notifications</strong>
            {data.unread > 0 && (
              <button className="text-link" onClick={() => api('/notifications/read-all', { method: 'POST' }).then(load)}><CheckCheck size={14} /> Mark all read</button>
            )}
          </div>
          {data.items.length === 0 && <p className="muted bell-empty">Nothing yet. Reminders for warranties, returns, maintenance, audits and loans appear here.</p>}
          <ul>
            {data.items.map((n) => (
              <li key={n.id}>
                <button className={n.readAt ? '' : 'unread'} onClick={() => read(n)}>
                  <strong>{n.title}</strong>
                  <span>{n.body}</span>
                  <small>{ago(n.createdAt)}{n.emailStatus && ` · ${EMAIL_STATUS[n.emailStatus] || n.emailStatus}`}</small>
                </button>
              </li>
            ))}
          </ul>
          {onViewAll && <button className="text-link bell-all" onClick={() => { setOpen(false); onViewAll(); }}>See all notifications and delivery</button>}
        </div>
      )}
    </div>
  );
}
