import React, { useEffect, useState } from 'react';
import { Download, QrCode, Search, Upload } from 'lucide-react';
import { api, query } from '../api.js';
import { useAuth } from '../auth.jsx';
import { AssetTable, ErrorBanner, downloadCSV } from '../components.jsx';
import { ASSET_STATUSES } from '../constants.js';
import { nouns, useCatalog } from '../catalog.jsx';
import { ImportAssets } from './ImportAssets.jsx';
import { LabelSheet } from './Labels.jsx';

const PAGE_SIZE = 50;

export function Inventory({ version, buildings, onSelect, onChanged }) {
  const { can, user } = useAuth();
  const { categories, tags } = useCatalog();
  const n = nouns(user.organization.kind);
  const [importing, setImporting] = useState(false);
  const [notice, setNotice] = useState(null);
  const [filters, setFilters] = useState({ q: '', status: '', categoryId: '', tag: '', buildingId: '' });
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
        <div><h2>All {n.Many} <span className="count">{result.total}</span></h2><p>{user.organization.kind === 'personal' ? 'Everything you own, in one place' : 'Inventory registry'}</p></div>
        <div className="row-end">
          {can('assets:write') && <button className="outline" onClick={() => setImporting(true)}><Upload size={16} /> Import CSV</button>}
          <button className="outline" onClick={printLabels} disabled={result.total === 0}><QrCode size={16} /> Print labels</button>
          <button className="outline" onClick={exportAll} disabled={exporting || result.total === 0}>
            <Download size={16} /> {exporting ? 'Exporting…' : 'Export CSV'}
          </button>
        </div>
      </div>
      <div className="filters">
        <div className="search">
          <Search size={18} />
          <input placeholder="Search names, serials, models, places…" value={filters.q} onChange={set('q')} aria-label={`Search ${n.many}`} />
        </div>
        <select value={filters.status} onChange={set('status')} aria-label="Status">
          <option value="">All statuses</option>
          {ASSET_STATUSES.map((s) => <option key={s}>{s}</option>)}
        </select>
        <select value={filters.categoryId} onChange={set('categoryId')} aria-label="Category">
          <option value="">All categories</option>
          {categories.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
        </select>
        {tags.length > 0 && (
          <select value={filters.tag} onChange={set('tag')} aria-label="Tag">
            <option value="">All tags</option>
            {tags.map((t) => <option key={t.id} value={t.name}>{t.name}</option>)}
          </select>
        )}
        <select value={filters.buildingId} onChange={set('buildingId')} aria-label={n.place}>
          <option value="">All {n.places.toLowerCase()}</option>
          {buildings.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
        </select>
      </div>
      <ErrorBanner error={error} />
      {notice && <div className="notice" role="status">{notice}</div>}
      <AssetTable assets={result.items} loading={loading} onSelect={onSelect} />
      {importing && (
        <ImportAssets onClose={() => setImporting(false)}
          onImported={(count) => { setImporting(false); setNotice(`Imported ${count} ${n.many}.`); onChanged(); }} />
      )}
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
