import React, { useEffect, useState } from 'react';
import { Download, QrCode, Search } from 'lucide-react';
import { api, query } from '../api.js';
import { AssetTable, ErrorBanner, downloadCSV } from '../components.jsx';
import { ASSET_STATUSES, ASSET_TYPES } from '../constants.js';
import { LabelSheet } from './Labels.jsx';

const PAGE_SIZE = 50;

export function Inventory({ version, buildings, onSelect }) {
  const [filters, setFilters] = useState({ q: '', status: '', type: '', buildingId: '' });
  const [debouncedQ, setDebouncedQ] = useState('');
  const [page, setPage] = useState(0);
  const [result, setResult] = useState({ items: [], total: 0 });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [exporting, setExporting] = useState(false);
  const [labels, setLabels] = useState(null);

  useEffect(() => {
    const t = setTimeout(() => setDebouncedQ(filters.q), 250);
    return () => clearTimeout(t);
  }, [filters.q]);

  const params = { ...filters, q: debouncedQ };
  const paramsKey = JSON.stringify(params);
  useEffect(() => { setPage(0); }, [paramsKey]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api(`/assets${query({ ...params, limit: PAGE_SIZE, offset: page * PAGE_SIZE })}`)
      .then((r) => { if (!cancelled) { setResult(r); setError(null); } })
      .catch((err) => !cancelled && setError(err))
      .finally(() => !cancelled && setLoading(false));
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paramsKey, page, version]);

  async function fetchAll() {
    const items = [];
    for (let offset = 0; ; offset += 500) {
      const r = await api(`/assets${query({ ...params, limit: 500, offset })}`);
      items.push(...r.items);
      if (items.length >= r.total || r.items.length === 0) return items;
    }
  }

  async function printLabels() {
    try { setLabels(await fetchAll()); } catch (err) { setError(err); }
  }

  async function exportAll() {
    setExporting(true);
    try {
      downloadCSV(await fetchAll());
    } catch (err) {
      setError(err);
    } finally {
      setExporting(false);
    }
  }

  const set = (key) => (e) => setFilters({ ...filters, [key]: e.target.value });
  const pages = Math.max(1, Math.ceil(result.total / PAGE_SIZE));

  return (
    <section className="panel">
      <div className="panel-head wrap">
        <div><h2>All Assets <span className="count">{result.total}</span></h2><p>Inventory registry</p></div>
        <div className="row-end">
          <button className="outline" onClick={printLabels} disabled={result.total === 0}><QrCode size={16} /> Print labels</button>
          <button className="outline" onClick={exportAll} disabled={exporting || result.total === 0}>
            <Download size={16} /> {exporting ? 'Exporting…' : 'Export CSV'}
          </button>
        </div>
      </div>
      <div className="filters">
        <div className="search">
          <Search size={18} />
          <input placeholder="Search tags, names, serials, locations..." value={filters.q} onChange={set('q')} aria-label="Search assets" />
        </div>
        <select value={filters.status} onChange={set('status')} aria-label="Status">
          <option value="">All statuses</option>
          {ASSET_STATUSES.map((s) => <option key={s}>{s}</option>)}
        </select>
        <select value={filters.type} onChange={set('type')} aria-label="Type">
          <option value="">All types</option>
          {ASSET_TYPES.map((t) => <option key={t}>{t}</option>)}
        </select>
        <select value={filters.buildingId} onChange={set('buildingId')} aria-label="Building">
          <option value="">All buildings</option>
          {buildings.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
        </select>
      </div>
      <ErrorBanner error={error} />
      <AssetTable assets={result.items} loading={loading} onSelect={onSelect} />
      {labels && <LabelSheet assets={labels} onClose={() => setLabels(null)} />}
      {pages > 1 && (
        <div className="pager">
          <button className="outline" disabled={page === 0} onClick={() => setPage(page - 1)}>Previous</button>
          <span>Page {page + 1} of {pages}</span>
          <button className="outline" disabled={page + 1 >= pages} onClick={() => setPage(page + 1)}>Next</button>
        </div>
      )}
    </section>
  );
}
