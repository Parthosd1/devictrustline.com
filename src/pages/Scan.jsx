import React, { useRef, useState } from 'react';
import { ScanLine, Search } from 'lucide-react';
import { api } from '../api.js';
import { CameraScanner, useDedupe } from '../camera.jsx';
import { AssetIcon, ErrorBanner, StatusBadge } from '../components.jsx';

// Looks up whatever a scanner produced: an asset tag, a label QR link, or a manufacturer serial barcode.
export function Scan({ onSelect }) {
  const [code, setCode] = useState('');
  const [results, setResults] = useState([]);
  const [error, setError] = useState(null);
  const inputRef = useRef(null);
  const isNew = useDedupe();

  async function lookup(raw) {
    const value = raw.trim();
    if (!value || !isNew(value)) return;
    const now = Date.now();
    try {
      const asset = await api(`/assets/lookup?code=${encodeURIComponent(value)}`);
      setError(null);
      setResults((r) => [{ code: value, asset, at: now }, ...r.filter((x) => x.asset?.id !== asset.id)].slice(0, 20));
    } catch (err) {
      setError(err);
      setResults((r) => [{ code: value, asset: null, at: now }, ...r].slice(0, 20));
    }
  }

  function submit(e) {
    e.preventDefault();
    lookup(code);
    setCode('');
    inputRef.current?.focus();
  }

  return (
    <div className="scan-layout">
      <section className="panel scan-panel">
        <div className="panel-head">
          <div><h2>Scan or enter a code</h2><p>Asset tag, label QR code, or manufacturer serial barcode. Handheld USB and Bluetooth scanners type straight into the box.</p></div>
        </div>
        <form className="scan-form" onSubmit={submit}>
          <div className="search">
            <ScanLine size={18} />
            <input ref={inputRef} autoFocus value={code} onChange={(e) => setCode(e.target.value)}
              placeholder="Scan now, or type DT-1001" aria-label="Scanned code" autoComplete="off" />
          </div>
          <button className="primary"><Search size={16} /> Look up</button>
        </form>
        <CameraScanner onCode={lookup} />
      </section>
      <section className="panel">
        <div className="panel-head"><div><h2>Scanned this session <span className="count">{results.length}</span></h2><p>Newest first. Select one to open it.</p></div></div>
        <ErrorBanner error={error} />
        <div className="scan-results">
          {results.length === 0 && <p className="empty">Nothing scanned yet.</p>}
          {results.map((r) => (r.asset ? (
            <button key={`${r.asset.id}-${r.at}`} className="component" onClick={() => onSelect(r.asset)}>
              <AssetIcon type={r.asset.type} size={16} />
              <span>
                <strong>{r.asset.name}</strong>
                <small>{r.asset.assetTag} · {r.asset.locationName || r.asset.buildingName || 'No location'}{r.asset.assignedPersonName ? ` · ${r.asset.assignedPersonName}` : ''}</small>
              </span>
              <StatusBadge status={r.asset.status} />
            </button>
          ) : (
            <div key={`${r.code}-${r.at}`} className="component unknown">
              <span className="asset-square">?</span>
              <span><strong>Not found</strong><small className="mono">{r.code}</small></span>
            </div>
          )))}
        </div>
      </section>
    </div>
  );
}
