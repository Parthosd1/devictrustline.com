import React, { useEffect, useState } from 'react';
import { AlertTriangle, ArrowRight, Boxes, CalendarDays, CheckCircle2, Monitor } from 'lucide-react';
import { api } from '../api.js';
import { AssetTable } from '../components.jsx';
import { progressOf } from './Audits.jsx';

const SHOWN_TYPES = ['Laptop', 'Desktop', 'Scanner', 'Monitor', 'Accessory'];

export function Dashboard({ assets, loading, onSelect, go, version }) {
  const [audits, setAudits] = useState([]);
  useEffect(() => { api('/audits?limit=50').then((r) => setAudits(r.items)).catch(() => setAudits([])); }, [version]);
  const active = assets.filter((a) => a.status !== 'Retired');
  const count = (status) => assets.filter((a) => a.status === status).length;
  const metrics = [
    { label: 'Total Assets', value: active.length, icon: Boxes, sub: 'Across all buildings, excluding retired' },
    { label: 'Deployed', value: count('Deployed'), icon: Monitor, sub: 'Active equipment' },
    { label: 'Available', value: count('Available'), icon: CheckCircle2, sub: 'Ready for assignment' },
    { label: 'Needs Attention', value: count('Maintenance'), icon: AlertTriangle, sub: 'In maintenance' },
  ];
  const types = [...SHOWN_TYPES, ...new Set(active.map((a) => a.type).filter((t) => !SHOWN_TYPES.includes(t)))];

  return (
    <>
      <div className="metrics">
        {metrics.map(({ label, value, icon: Icon, sub }) => (
          <div className="metric" key={label}>
            <div className="metric-top"><span>{label}</span><span className="metric-icon"><Icon size={19} /></span></div>
            <strong>{loading ? '–' : value}</strong>
            <small>{sub}</small>
          </div>
        ))}
      </div>
      <div className="two-col">
        <section className="panel">
          <div className="panel-head"><div><h2>Inventory by Type</h2><p>Distribution across your portfolio</p></div></div>
          <div className="type-list">
            {types.map((t) => {
              const n = active.filter((a) => a.type === t).length;
              return (
                <div className="type-row" key={t}>
                  <div><span>{t}</span><b>{n}</b></div>
                  <div className="track"><div style={{ width: `${active.length ? (n / active.length) * 100 : 0}%` }} /></div>
                </div>
              );
            })}
          </div>
        </section>
        <section className="panel">
          <div className="panel-head"><div><h2>Audit Snapshot</h2><p>Current verification progress</p></div><CalendarDays size={20} /></div>
          {[['weekly', 'Weekly'], ['monthly', 'Monthly']].map(([key, label]) => {
            const open = audits.filter((r) => r.period === key && r.status === 'open');
            const last = audits.find((r) => r.period === key && r.status === 'closed');
            const totals = open.reduce((t, r) => {
              const p = progressOf(r);
              return { found: t.found + p.found, expected: t.expected + p.expected };
            }, { found: 0, expected: 0 });
            return (
              <div className="audit-snapshot" key={key}>
                <div>
                  <strong>{label} Audit</strong>
                  <small>
                    {open.length ? `${totals.found} of ${totals.expected} verified`
                      : last ? `Last closed ${new Date(last.closedAt).toLocaleDateString()} · ${last.summary.missing} missing` : 'Not started yet'}
                  </small>
                </div>
                <div className="progress-track"><div style={{ width: `${totals.expected ? (totals.found / totals.expected) * 100 : 0}%` }} /></div>
              </div>
            );
          })}
          <button className="text-link" onClick={() => go('Audits')}>Open Audit Center <ArrowRight size={16} /></button>
        </section>
      </div>
      <section className="panel">
        <div className="panel-head">
          <div><h2>Recent Inventory</h2><p>Latest registered equipment</p></div>
          <button className="outline" onClick={() => go('Inventory')}>View all <ArrowRight size={15} /></button>
        </div>
        <AssetTable assets={assets.slice(0, 5)} loading={loading} onSelect={onSelect} />
      </section>
    </>
  );
}
