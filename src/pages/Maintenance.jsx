import React from 'react';
import { AssetTable } from '../components.jsx';

export function Maintenance({ assets, loading, onSelect }) {
  return (
    <section className="panel">
      <div className="panel-head">
        <div><h2>Maintenance Queue</h2><p>Assets currently marked for repair or investigation. Full work orders arrive in a later phase.</p></div>
      </div>
      <AssetTable assets={assets.filter((a) => a.status === 'Maintenance')} loading={loading} onSelect={onSelect} />
    </section>
  );
}
