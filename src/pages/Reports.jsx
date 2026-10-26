import React, { useEffect, useState } from 'react';
import { Download } from 'lucide-react';
import { api } from '../api.js';
import { ErrorBanner } from '../components.jsx';
import { downloadRows } from '../csv.js';

const money = (n) => `$${Number(n || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })}`;
const date = (d) => (d ? new Date(d).toLocaleDateString() : '');
const monthLabel = (m) => new Date(`${m}-01T00:00:00`).toLocaleDateString(undefined, { month: 'short' });

function Bars({ rows, total }) {
  if (!rows.length) return <div className="empty">No assets yet.</div>;
  return (
    <div className="type-list">
      {rows.map((r) => (
        <div className="type-row" key={r.key} title={`${r.key}: ${r.count}`}>
          <div><span>{r.key}</span><b>{r.count}</b></div>
          <div className="track"><div style={{ width: `${total ? (r.count / total) * 100 : 0}%` }} /></div>
        </div>
      ))}
    </div>
  );
}

function Panel({ title, sub, onExport, children }) {
  return (
    <section className="panel">
      <div className="panel-head">
        <div><h2>{title}</h2>{sub && <p>{sub}</p>}</div>
        {onExport && <button className="outline" onClick={onExport} aria-label={`Download ${title} CSV`}><Download size={15} /> CSV</button>}
      </div>
      {children}
    </section>
  );
}

function CostChart({ months }) {
  const max = Math.max(1, ...months.map((m) => m.cost));
  return (
    <div className="col-chart" role="img" aria-label="Maintenance cost by month">
      {months.map((m) => (
        <div className="col" key={m.month} tabIndex={0}>
          <span className="col-tip">{monthLabel(m.month)} {m.month.slice(0, 4)}: {money(m.cost)} · {m.resolved} resolved</span>
          <div className="col-bar"><div style={{ height: `${(m.cost / max) * 100}%` }} /></div>
          <small>{monthLabel(m.month)}</small>
        </div>
      ))}
    </div>
  );
}

export function Reports({ onSelect, version }) {
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => { api('/reports/summary').then(setData).catch(setError); }, [version]);

  if (error) return <ErrorBanner error={error} />;
  if (!data) return <div className="empty">Loading reports…</div>;

  const active = data.byType.reduce((n, r) => n + r.count, 0);
  const all = data.byStatus.reduce((n, r) => n + r.count, 0);
  const m = data.maintenance;
  const yearCost = data.maintenanceCostByMonth.reduce((n, r) => n + r.cost, 0);
  const open = (id) => api(`/assets/${id}`).then(onSelect).catch(setError);
  const counts = (name, rows) => () => downloadRows(`devicetrustline-${name}.csv`, [['key', name === 'by-status' ? 'Status' : name === 'by-type' ? 'Type' : 'Building'], ['count', 'Assets']], rows);

  return (
    <>
      <div className="metrics">
        <div className="metric"><span>Active assets</span><strong>{active}</strong><small>{all - active} retired</small></div>
        <div className="metric"><span>Assigned to people</span><strong>{data.assignment.assigned}</strong><small>{data.assignment.unassigned} unassigned</small></div>
        <div className="metric"><span>Open work orders</span><strong>{m.active}</strong><small>{m.overdue} overdue · {m.resolved30d} resolved in 30 days</small></div>
        <div className="metric"><span>Maintenance cost, 12 months</span><strong>{money(yearCost)}</strong><small>{m.avgDaysToResolve != null ? `${m.avgDaysToResolve} days average to resolve` : 'No repairs resolved yet'}</small></div>
      </div>
      <div className="two-col">
        <Panel title="Assets by Status" sub="Including retired" onExport={counts('by-status', data.byStatus)}><Bars rows={data.byStatus} total={all} /></Panel>
        <Panel title="Assets by Type" sub="Active equipment" onExport={counts('by-type', data.byType)}><Bars rows={data.byType} total={active} /></Panel>
      </div>
      <div className="two-col">
        <Panel title="Assets by Building" sub="Active equipment" onExport={counts('by-building', data.byBuilding)}><Bars rows={data.byBuilding} total={active} /></Panel>
        <Panel title="Maintenance Cost" sub="Resolved work orders, last 12 months"
          onExport={() => downloadRows('devicetrustline-maintenance-cost.csv', [['month', 'Month'], ['cost', 'Cost (USD)'], ['resolved', 'Resolved']], data.maintenanceCostByMonth)}>
          <CostChart months={data.maintenanceCostByMonth} />
        </Panel>
      </div>
      <Panel title="Warranties" sub="Expired or expiring in the next 90 days"
        onExport={data.warranty.length ? () => downloadRows('devicetrustline-warranties.csv', [['assetTag', 'Asset tag'], ['name', 'Name'], ['type', 'Type'], ['warrantyExpires', 'Warranty expires'], [(r) => (r.expired ? 'Expired' : 'Expiring'), 'State']], data.warranty) : null}>
        {data.warranty.length ? (
          <div className="table-scroll"><table>
            <thead><tr><th>ASSET</th><th>TYPE</th><th>WARRANTY</th><th>STATE</th></tr></thead>
            <tbody>{data.warranty.map((w) => (
              <tr key={w.id} className="clickable" onClick={() => open(w.id)}>
                <td><strong className="normal">{w.name}</strong><small className="mono">{w.assetTag}</small></td>
                <td>{w.type}</td><td>{date(w.warrantyExpires)}</td>
                <td><span className={`status ${w.expired ? 'retired' : 'maintenance'}`}><i />{w.expired ? 'Expired' : 'Expiring soon'}</span></td>
              </tr>
            ))}</tbody>
          </table></div>
        ) : <div className="empty">No warranties expiring in the next 90 days.</div>}
      </Panel>
      <div className="two-col" style={{ marginTop: 20 }}>
        <Panel title="Still Missing" sub="Missing in an audit and not found since"
          onExport={data.stillMissing.length ? () => downloadRows('devicetrustline-missing.csv', [['assetTag', 'Asset tag'], ['name', 'Name'], ['audit', 'Missing in'], ['closedAt', 'Audit closed']], data.stillMissing) : null}>
          {data.stillMissing.length ? (
            <div className="table-scroll"><table>
              <thead><tr><th>ASSET</th><th>MISSING IN</th></tr></thead>
              <tbody>{data.stillMissing.map((a) => (
                <tr key={a.id} className="clickable" onClick={() => open(a.id)}>
                  <td><strong className="normal">{a.name}</strong><small className="mono">{a.assetTag}</small></td>
                  <td>{a.audit}<small>{date(a.closedAt)}</small></td>
                </tr>
              ))}</tbody>
            </table></div>
          ) : <div className="empty">Every asset has been accounted for.</div>}
        </Panel>
        <Panel title="Recent Audits" sub="Last 12 closed audits"
          onExport={data.recentAudits.length ? () => downloadRows('devicetrustline-audits.csv', [['name', 'Audit'], ['period', 'Period'], ['closedAt', 'Closed'], [(r) => r.summary?.verified ?? '', 'Verified'], [(r) => r.summary?.missing ?? '', 'Missing'], [(r) => r.summary?.wrongLocation ?? '', 'Wrong location'], [(r) => r.summary?.unexpected ?? '', 'Unexpected']], data.recentAudits) : null}>
          {data.recentAudits.length ? (
            <div className="table-scroll"><table>
              <thead><tr><th>AUDIT</th><th>CLOSED</th><th>VERIFIED</th><th>MISSING</th></tr></thead>
              <tbody>{data.recentAudits.map((r) => (
                <tr key={r.id}>
                  <td><strong className="normal">{r.name}</strong><small>{r.period === 'weekly' ? 'Weekly' : 'Monthly'}</small></td>
                  <td>{date(r.closedAt)}</td><td>{r.summary?.verified ?? '–'}</td><td>{r.summary?.missing ?? '–'}</td>
                </tr>
              ))}</tbody>
            </table></div>
          ) : <div className="empty">No audits closed yet.</div>}
        </Panel>
      </div>
    </>
  );
}
