import React, { useEffect, useState } from 'react';
import { AlertTriangle, ArrowRight, Boxes, CalendarClock, CalendarDays, CheckCircle2, Lightbulb, Monitor, ShieldCheck, UserCheck } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { nouns } from '../catalog.jsx';
import { AssetTable } from '../components.jsx';
import { progressOf } from './Audits.jsx';

const DEADLINE_KINDS = { warranty: 'Warranty', return_window: 'Return', maintenance: 'Maintenance', audit: 'Audit', loan: 'Loan' };
const open = (onSelect, id) => onSelect({ id, name: '', assetTag: '', type: '', status: '' });

function when(days) {
  if (days === 0) return 'Today';
  if (days === 1) return 'Tomorrow';
  if (days === -1) return 'Yesterday';
  return days < 0 ? `${-days} days ago` : `In ${days} days`;
}

function Deadlines({ items, onSelect }) {
  return (
    <section className="panel">
      <div className="panel-head"><div><h2>Coming up</h2><p>Warranties, returns, maintenance and loans in the next 90 days</p></div><CalendarClock size={20} /></div>
      {items === null && <p className="empty">Loading…</p>}
      {items?.length === 0 && <p className="empty">Nothing due. Add warranty, return or maintenance dates to items to see them here.</p>}
      <ul className="deadline-list">
        {items?.slice(0, 8).map((d) => (
          <li key={`${d.kind}-${d.sourceId}-${d.dueOn}`}>
            <button className="row-btn" onClick={() => open(onSelect, d.assetId)}>
              <span className={`pill ${d.daysLeft < 0 ? 'late' : d.daysLeft <= 7 ? 'soon' : ''}`}>{DEADLINE_KINDS[d.kind] ?? d.kind}</span>
              <span className="grow">{d.title}</span>
              <small>{when(d.daysLeft)}</small>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Insights({ items, onSelect }) {
  if (!items?.length) return null;
  return (
    <section className="panel">
      <div className="panel-head"><div><h2>Worth a look</h2><p>Suggestions only. Nothing is changed, sold or listed for you.</p></div><Lightbulb size={20} /></div>
      <div className="insights">
        {items.map((i) => (
          <details key={i.kind}>
            <summary><strong>{i.title}</strong> <span className="count">{i.items.length}</span></summary>
            <p className="hint">{i.explanation}</p>
            <ul className="name-list">
              {i.items.slice(0, 10).map((a) => (
                <li key={a.id}><button className="link-btn" onClick={() => open(onSelect, a.id)}>{a.name}</button><small>{a.assetTag}{a.endsOn ? ` · ends ${a.endsOn}` : ''}</small></li>
              ))}
            </ul>
          </details>
        ))}
      </div>
    </section>
  );
}

function useDeadlinesAndInsights(version) {
  const [deadlines, setDeadlines] = useState(null);
  const [insights, setInsights] = useState([]);
  useEffect(() => {
    api('/deadlines').then((r) => setDeadlines(r.items)).catch(() => setDeadlines([]));
    api('/insights').then((r) => setInsights(r.items.filter((i) => i.items.length))).catch(() => setInsights([]));
  }, [version]);
  return [deadlines, insights];
}

function Metrics({ metrics, loading }) {
  return (
    <div className="metrics">
      {metrics.map(({ label, value, icon: Icon, sub }) => (
        <div className="metric" key={label}>
          <div className="metric-top"><span>{label}</span><span className="metric-icon"><Icon size={19} /></span></div>
          <strong>{loading ? '–' : value}</strong>
          <small>{sub}</small>
        </div>
      ))}
    </div>
  );
}

function ByCategory({ active, title }) {
  const counts = {};
  for (const a of active) counts[a.type] = (counts[a.type] ?? 0) + 1;
  const rows = Object.entries(counts).sort((x, y) => y[1] - x[1]);
  return (
    <section className="panel">
      <div className="panel-head"><div><h2>{title}</h2><p>How your {active.length} active entries break down</p></div></div>
      <div className="type-list">
        {rows.length === 0 && <p className="empty">Nothing here yet.</p>}
        {rows.map(([t, n]) => (
          <div className="type-row" key={t}>
            <div><span>{t}</span><b>{n}</b></div>
            <div className="track"><div style={{ width: `${(n / active.length) * 100}%` }} /></div>
          </div>
        ))}
      </div>
    </section>
  );
}

function PersonalDashboard({ assets, loading, onSelect, go, version }) {
  const [deadlines, insights] = useDeadlinesAndInsights(version);
  const active = assets.filter((a) => a.status !== 'Retired');
  const soon = active.filter((a) => a.warrantyStatus === 'expiring_soon').length;
  const covered = active.filter((a) => a.warrantyStatus === 'active' || a.warrantyStatus === 'expiring_soon').length;
  const metrics = [
    { label: 'Items', value: active.length, icon: Boxes, sub: 'Everything you are keeping track of' },
    { label: 'Under warranty', value: covered, icon: ShieldCheck, sub: `${soon} ending soon` },
    { label: 'Coming up', value: deadlines?.filter((d) => d.daysLeft >= 0 && d.daysLeft <= 30).length ?? '–', icon: CalendarClock, sub: 'Dates in the next 30 days' },
    { label: 'Lent out', value: active.filter((a) => a.assignedPersonId).length, icon: UserCheck, sub: 'With someone else right now' },
  ];
  return (
    <>
      <Metrics metrics={metrics} loading={loading} />
      {!loading && active.length === 0 && (
        <section className="panel pad">
          <h2>Start your inventory</h2>
          <p className="hint">Add the things you own with their receipts and warranties, and DeviceTrustline reminds you before a warranty ends or a return window closes. On a Windows PC, the device companion under Devices can find your computer's hardware for you.</p>
          <button className="outline" onClick={() => go('Devices')}>Set up the device companion <ArrowRight size={15} /></button>
        </section>
      )}
      <div className="two-col">
        <Deadlines items={deadlines} onSelect={onSelect} />
        <ByCategory active={active} title="By category" />
      </div>
      <Insights items={insights} onSelect={onSelect} />
      <section className="panel">
        <div className="panel-head">
          <div><h2>Recently added</h2></div>
          <button className="outline" onClick={() => go('Inventory')}>View all <ArrowRight size={15} /></button>
        </div>
        <AssetTable assets={assets.slice(0, 5)} loading={loading} onSelect={onSelect} />
      </section>
    </>
  );
}

export function Dashboard(props) {
  const { user } = useAuth();
  return user.organization.kind === 'personal' ? <PersonalDashboard {...props} /> : <BusinessDashboard {...props} />;
}

function BusinessDashboard({ assets, loading, onSelect, go, version }) {
  const n = nouns('business');
  const [deadlines, insights] = useDeadlinesAndInsights(version);
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

  return (
    <>
      <Metrics metrics={metrics} loading={loading} />
      <div className="two-col">
        <ByCategory active={active} title={`${n.Many} by category`} />
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
      <div className="two-col">
        <Deadlines items={deadlines} onSelect={onSelect} />
        <Insights items={insights} onSelect={onSelect} />
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
