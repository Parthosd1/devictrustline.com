import React, { useState } from 'react';
import { Download, FileUp, Upload } from 'lucide-react';
import { api } from '../api.js';
import { ErrorBanner, Modal } from '../components.jsx';
import { downloadRows, parseCSV } from '../csv.js';

// Accepted column headings (case and spacing ignored). Matches the inventory CSV export, so an export can be re-imported.
const HEADERS = {
  name: 'name', assetname: 'name', type: 'type', assettype: 'type', serial: 'serial', serialnumber: 'serial',
  manufacturer: 'manufacturer', make: 'manufacturer', model: 'model', status: 'status', building: 'building',
  location: 'location', station: 'location', locationstation: 'location', purchasedate: 'purchaseDate',
  warrantyexpires: 'warrantyExpires', warranty: 'warrantyExpires', notes: 'notes',
};
const TEMPLATE = [['name', 'Name'], ['type', 'Type'], ['serial', 'Serial'], ['manufacturer', 'Manufacturer'], ['model', 'Model'],
  ['status', 'Status'], ['building', 'Building'], ['location', 'Location'], ['purchaseDate', 'Purchase date'],
  ['warrantyExpires', 'Warranty expires'], ['notes', 'Notes']];
const EXAMPLE = [{ name: 'Dell Latitude 5440', type: 'Laptop', serial: 'DL5440-0001', manufacturer: 'Dell', model: 'Latitude 5440', status: 'Available', building: 'Building A', location: 'IT Storage', purchaseDate: '2025-01-15', warrantyExpires: '2028-01-15', notes: '' }];

function toRows(text) {
  const [header, ...lines] = parseCSV(text);
  if (!header) throw new Error('The file is empty');
  const keys = header.map((h) => HEADERS[h.toLowerCase().replace(/[^a-z]/g, '')] ?? null);
  if (!keys.includes('name') || !keys.includes('type')) throw new Error('The file needs at least "Name" and "Type" columns');
  return {
    ignored: header.filter((_, i) => !keys[i]),
    rows: lines.map((cells) => Object.fromEntries(keys.map((k, i) => [k, cells[i] ?? '']).filter(([k]) => k))),
  };
}

export function ImportAssets({ onClose, onImported }) {
  const [file, setFile] = useState(null);
  const [parsed, setParsed] = useState(null);
  const [createSites, setCreateSites] = useState(true);
  const [check, setCheck] = useState(null);
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);

  async function validate(rows, sites = createSites) {
    setBusy(true);
    try {
      setCheck(await api('/assets/import', { method: 'POST', body: { rows, createSites: sites, dryRun: true } }));
      setError(null);
    } catch (err) { setError(err); setCheck(null); }
    setBusy(false);
  }

  async function choose(e) {
    const f = e.target.files[0];
    if (!f) return;
    setFile(f);
    setCheck(null);
    try {
      const p = toRows(await f.text());
      setParsed(p);
      await validate(p.rows);
    } catch (err) { setParsed(null); setError(err); }
  }

  async function commit() {
    setBusy(true);
    try {
      const r = await api('/assets/import', { method: 'POST', body: { rows: parsed.rows, createSites, dryRun: false } });
      onImported(r.created);
    } catch (err) { setError(err); setBusy(false); }
  }

  return (
    <Modal title="Import Assets from CSV" subtitle="Up to 2,000 rows. Nothing is imported unless every row is valid." onClose={onClose} wide>
      <ErrorBanner error={error} />
      <div className="import-row">
        <label className="file-drop">
          <FileUp size={20} />
          <span>{file ? file.name : 'Choose a .csv file'}</span>
          <input type="file" accept=".csv,text/csv" onChange={choose} aria-label="CSV file" />
        </label>
        <button className="outline" onClick={() => downloadRows('devicetrustline-import-template.csv', TEMPLATE, EXAMPLE)}><Download size={15} /> Template</button>
      </div>
      <p className="hint">Columns: Name and Type are required; Serial, Manufacturer, Model, Status, Building, Location, Purchase date, Warranty expires and Notes are optional. Dates use YYYY-MM-DD. Asset tags are assigned automatically.</p>
      <label className="check">
        <input type="checkbox" checked={createSites} onChange={(e) => { setCreateSites(e.target.checked); if (parsed) validate(parsed.rows, e.target.checked); }} />
        Create buildings and locations that don't exist yet
      </label>
      {parsed?.ignored.length > 0 && <p className="hint">Ignored columns: {parsed.ignored.join(', ')}</p>}
      {check && (
        <div className="import-check">
          <p><strong>{check.valid} of {check.rows}</strong> rows ready{check.newBuildings.length + check.newLocations.length > 0 && (
            <> · will create {[...check.newBuildings.map((b) => `building ${b}`), ...check.newLocations.map((l) => `location ${l}`)].join(', ')}</>
          )}</p>
          {check.errors.length > 0 && (
            <ul className="import-errors">
              {check.errors.slice(0, 50).map((e) => <li key={e.row}><b>Row {e.row + 1}</b> {e.messages.join('; ')}</li>)}
              {check.errors.length > 50 && <li>…and {check.errors.length - 50} more rows</li>}
            </ul>
          )}
        </div>
      )}
      <div className="modal-actions">
        <button className="outline" onClick={onClose}>Cancel</button>
        <button className="primary" disabled={busy || !check || check.errors.length > 0} onClick={commit}>
          <Upload size={16} /> {check && !check.errors.length ? `Import ${check.rows} assets` : 'Import'}
        </button>
      </div>
    </Modal>
  );
}
