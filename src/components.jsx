import React, { cloneElement, useEffect, useId } from 'react';
import { AlertTriangle, Laptop, Monitor, Package, Printer, ScanLine, Smartphone, X } from 'lucide-react';
import { downloadRows } from './csv.js';

const TYPE_ICONS = {
  Laptop, Desktop: Monitor, Workstation: Monitor, Monitor, Scanner: ScanLine, Printer, Phone: Smartphone, Tablet: Smartphone,
};

export function AssetIcon({ type, size = 18 }) {
  const Icon = TYPE_ICONS[type] || Package;
  return <span className="asset-square"><Icon size={size} /></span>;
}

export function StatusBadge({ status }) {
  if (!status) return null;
  return <span className={`status ${status.toLowerCase()}`}><i />{status}</span>;
}

export function ErrorBanner({ error }) {
  if (!error) return null;
  return <div className="error-banner" role="alert"><AlertTriangle size={16} />{error.message || String(error)}</div>;
}

export function Modal({ title, subtitle, onClose, children, wide }) {
  useEffect(() => {
    const onKey = (e) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className={`modal ${wide ? 'wide' : ''}`} role="dialog" aria-modal="true" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">
          <div><h2>{title}</h2>{subtitle && <p>{subtitle}</p>}</div>
          <button className="icon-btn" onClick={onClose} aria-label="Close"><X /></button>
        </div>
        {children}
      </div>
    </div>
  );
}

// A labelled form control. The label is tied to the control by id, so select options never leak into its name.
export function Field({ label, children, wide }) {
  const id = useId();
  return (
    <div className={`field ${wide ? 'span-2' : ''}`}>
      <label htmlFor={id}>{label}</label>
      {cloneElement(children, { id })}
    </div>
  );
}

export function AssetTable({ assets, onSelect, loading }) {
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr><th>ASSET</th><th>TYPE</th><th>SERIAL NUMBER</th><th>LOCATION</th><th>ASSIGNED TO</th><th>STATUS</th></tr>
        </thead>
        <tbody>
          {assets.length === 0 ? (
            <tr><td colSpan="6" className="empty">{loading ? 'Loading…' : 'No assets found'}</td></tr>
          ) : assets.map((a) => (
            <tr key={a.id} className={onSelect ? 'clickable' : undefined} onClick={onSelect && (() => onSelect(a))}
              tabIndex={onSelect ? 0 : undefined} onKeyDown={onSelect && ((e) => e.key === 'Enter' && onSelect(a))}>
              <td>
                <div className="asset-name">
                  <AssetIcon type={a.type} />
                  <span><strong>{a.name}</strong><small>{a.assetTag}{a.parentTag && ` · part of ${a.parentTag}`}{a.childCount > 0 && ` · ${a.childCount} component${a.childCount === 1 ? '' : 's'}`}</small></span>
                </div>
              </td>
              <td>{a.type}</td>
              <td className="mono">{a.serial || '—'}</td>
              <td><strong className="normal">{a.buildingName || 'No building'}</strong><small>{a.locationName || 'Unassigned'}</small></td>
              <td>{a.assignedPersonName || <span className="muted">—</span>}</td>
              <td><StatusBadge status={a.status} /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function downloadCSV(assets, filename = 'devicetrustline-assets.csv') {
  downloadRows(filename, [
    ['assetTag', 'Asset tag'], ['name', 'Name'], ['type', 'Type'], ['serial', 'Serial'],
    ['manufacturer', 'Manufacturer'], ['model', 'Model'], ['buildingName', 'Building'],
    ['locationName', 'Location'], ['parentTag', 'Part of'], ['assignedPersonName', 'Assigned to'], ['status', 'Status'],
    ['purchaseDate', 'Purchase date'], ['warrantyExpires', 'Warranty expires'], ['notes', 'Notes'],
  ], assets);
}
