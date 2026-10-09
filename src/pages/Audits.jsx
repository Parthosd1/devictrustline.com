import React, { useState } from 'react';
import { CheckCircle2, Info, ScanLine } from 'lucide-react';
import { AssetIcon } from '../components.jsx';

// Until server-side audit runs land (roadmap phase 5), checklists are kept in this browser, per workspace.
const key = (orgId) => `dt-audit-checklist:${orgId}`;

export function loadChecklist(orgId) {
  try {
    const saved = JSON.parse(localStorage.getItem(key(orgId)));
    if (saved?.Weekly && saved?.Monthly) return saved;
  } catch { /* fall through */ }
  return { Weekly: [], Monthly: [] };
}

export function Audits({ assets, orgId }) {
  const [period, setPeriod] = useState('Weekly');
  const [checked, setChecked] = useState(() => loadChecklist(orgId));
  const active = assets.filter((a) => a.status !== 'Retired');
  const done = active.filter((a) => checked[period].includes(a.id)).length;

  function toggle(id) {
    const list = checked[period];
    const next = { ...checked, [period]: list.includes(id) ? list.filter((x) => x !== id) : [...list, id] };
    setChecked(next);
    try { localStorage.setItem(key(orgId), JSON.stringify(next)); } catch { /* storage unavailable */ }
  }

  return (
    <>
      <div className="tabs">
        {['Weekly', 'Monthly'].map((p) => (
          <button key={p} className={period === p ? 'selected' : ''} onClick={() => setPeriod(p)}>{p} Audit</button>
        ))}
      </div>
      <div className="metrics audit-metrics">
        <div className="metric"><span>Expected Assets</span><strong>{active.length}</strong><small>Active inventory</small></div>
        <div className="metric"><span>Verified</span><strong>{done}</strong><small>{period} cycle</small></div>
        <div className="metric"><span>Remaining</span><strong>{active.length - done}</strong><small>Pending verification</small></div>
      </div>
      <section className="panel">
        <div className="panel-head">
          <div>
            <h2>{period} Verification Checklist</h2>
            <p className="note"><Info size={14} /> Saved in this browser only. Recorded, scan-based audit runs are coming in a later phase.</p>
          </div>
        </div>
        <div className="audit-items">
          {active.length === 0 && <p className="empty">No assets to audit yet.</p>}
          {active.map((a) => {
            const ok = checked[period].includes(a.id);
            return (
              <div className="audit-item" key={a.id}>
                <AssetIcon type={a.type} size={19} />
                <div className="audit-info">
                  <strong>{a.name}</strong>
                  <small>{a.assetTag} · {a.buildingName || 'No building'} · {a.locationName || 'Unassigned'}</small>
                </div>
                <button className={ok ? 'verified-btn' : 'outline'} onClick={() => toggle(a.id)}>
                  {ok ? <><CheckCircle2 size={16} /> Verified</> : <><ScanLine size={16} /> Mark Verified</>}
                </button>
              </div>
            );
          })}
        </div>
      </section>
    </>
  );
}
