import React, { useEffect, useRef, useState } from 'react';
import { Camera, CameraOff, ScanLine, Search } from 'lucide-react';
import { api } from '../api.js';
import { AssetIcon, ErrorBanner, StatusBadge } from '../components.jsx';

// Looks up whatever a scanner produced: an asset tag, a label QR link, or a manufacturer serial barcode.
export function Scan({ onSelect }) {
  const [code, setCode] = useState('');
  const [results, setResults] = useState([]);
  const [error, setError] = useState(null);
  const [camera, setCamera] = useState(false);
  const [cameraError, setCameraError] = useState(null);
  const inputRef = useRef(null);
  const videoRef = useRef(null);
  const lastScan = useRef({ code: '', at: 0 });

  async function lookup(raw) {
    const value = raw.trim();
    if (!value) return;
    // Cameras report the same code many times a second; ignore repeats for a moment.
    const now = Date.now();
    if (value === lastScan.current.code && now - lastScan.current.at < 2500) return;
    lastScan.current = { code: value, at: now };
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

  useEffect(() => {
    if (!camera) return undefined;
    let controls;
    let stopped = false;
    (async () => {
      try {
        const { BrowserMultiFormatReader } = await import('@zxing/browser');
        const reader = new BrowserMultiFormatReader();
        controls = await reader.decodeFromConstraints(
          { video: { facingMode: 'environment' } },
          videoRef.current,
          (result) => { if (result) lookup(result.getText()); },
        );
        if (stopped) controls.stop();
      } catch (err) {
        setCameraError(err?.name === 'NotAllowedError'
          ? 'Camera permission was denied. Allow camera access for this site, or use a handheld scanner.'
          : 'No camera is available on this device. Use a handheld scanner or type the code.');
        setCamera(false);
      }
    })();
    return () => { stopped = true; controls?.stop(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [camera]);

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
        <div className="camera-row">
          <button className="outline" type="button" onClick={() => { setCameraError(null); setCamera(!camera); }}>
            {camera ? <><CameraOff size={16} /> Stop camera</> : <><Camera size={16} /> Use camera</>}
          </button>
          {cameraError && <span className="camera-error">{cameraError}</span>}
        </div>
        {camera && <div className="video-wrap"><video ref={videoRef} muted playsInline /><div className="reticle" /></div>}
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
