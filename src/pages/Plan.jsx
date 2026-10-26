import React, { useEffect, useState } from 'react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner } from '../components.jsx';

const LIMITS = [['items', 'Items'], ['members', 'People who can sign in'], ['devices', 'Paired computers'], ['storageMb', 'File storage (MB)']];

export function Plan() {
  const { can } = useAuth();
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [message, setMessage] = useState(null);
  useEffect(() => { api('/plan').then(setData).catch(setError); }, []);

  async function choose(code) {
    setMessage(null);
    try {
      await api('/billing/checkout', { method: 'POST', body: { plan: code } });
    } catch (err) {
      if (err.code === 'billing_unconfigured') setMessage(err.message);
      else setError(err);
    }
  }

  if (!data) return <><ErrorBanner error={error} />{!error && <p className="muted">Loading…</p>}</>;
  return (
    <>
      <ErrorBanner error={error} />
      {message && <div className="notice" role="status">{message}</div>}
      <section className="panel">
        <div className="panel-head"><div><h2>{data.plan.name}</h2><p>Your current plan and how much of it you use.</p></div></div>
        <div className="usage">
          {LIMITS.map(([key, label]) => {
            const limit = data.plan.limits[key];
            const used = data.usage[key] ?? 0;
            const pct = limit ? Math.min(100, (used / limit) * 100) : 0;
            return (
              <div key={key} className="usage-row">
                <div><span>{label}</span><b>{used}{limit != null ? ` of ${limit}` : ' (no limit)'}</b></div>
                {limit != null && (
                  <div className="track" role="progressbar" aria-label={label} aria-valuenow={used} aria-valuemin={0} aria-valuemax={limit}>
                    <div className={pct >= 90 ? 'warn' : ''} style={{ width: `${pct}%` }} />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </section>
      <section className="panel">
        <div className="panel-head"><div><h2>Plans</h2><p>{data.billing.configured ? 'Change plans here.' : 'Online payment is not set up on this server yet. Plan changes are made by the service operator for now.'}</p></div></div>
        <div className="plan-grid">
          {data.plans.map((p) => (
            <div key={p.code} className={`plan-card ${p.code === data.plan.code ? 'current' : ''}`}>
              <h3>{p.name}</h3>
              <ul>
                {LIMITS.map(([key, label]) => <li key={key}>{label}: <b>{p.limits[key] ?? 'No limit'}</b></li>)}
              </ul>
              {p.code === data.plan.code
                ? <span className="status deployed"><i />Current plan</span>
                : can('settings:manage') && <button className="outline" onClick={() => choose(p.code)}>Switch to {p.name}</button>}
            </div>
          ))}
        </div>
      </section>
    </>
  );
}
