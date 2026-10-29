import React, { useCallback, useEffect, useState } from 'react';
import { Download, EyeOff, KeyRound, Link2, Pause, Play, Plus, RotateCcw, Unplug } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { useCatalog } from '../catalog.jsx';
import { ErrorBanner, Field, Modal } from '../components.jsx';

const DOWNLOAD = '/downloads/devicetrustline-agent.exe';
const when = (iso) => (iso ? new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : 'Never');

// Whether this server offers the Windows companion for download (the Docker image includes it).
// What the companion reported about a computer's own hardware, as short labelled lines.
export function hardwareLines(details = {}) {
  const lines = [];
  const { cpu, memory, storage, os, network } = details;
  if (cpu?.name || cpu?.cores) lines.push(['Processor', [cpu.name, cpu.cores && `${cpu.cores} cores`, cpu.threads && `${cpu.threads} threads`].filter(Boolean).join(' · ')]);
  if (memory?.totalGb) lines.push(['Memory', `${memory.totalGb} GB${memory.modules?.length ? ` in ${memory.modules.length} module${memory.modules.length === 1 ? '' : 's'}` : ''}`]);
  if (storage?.totalGb || storage?.volumes) {
    lines.push(['Storage', [storage.totalGb && `${storage.totalGb} GB`, storage.freeGb != null && `${storage.freeGb} GB free`,
      storage.volumes?.map((v) => v.drive).join(' ')].filter(Boolean).join(' · ')]);
  }
  if (os?.name) lines.push(['Operating system', [os.name, os.version, os.build && `build ${os.build}`, os.architecture].filter(Boolean).join(' · ')]);
  if (network?.length) lines.push(['Network', network.map((n) => `${n.name}${n.mac ? ` (${n.mac})` : ''}`).join(', ')]);
  return lines;
}

function useDownload() {
  const [available, setAvailable] = useState(false);
  useEffect(() => {
    fetch(DOWNLOAD, { method: 'HEAD' })
      .then((r) => setAvailable(r.ok && !(r.headers.get('content-type') || '').includes('text/html')))
      .catch(() => {});
  }, []);
  return available;
}

function PairDialog({ onClose }) {
  const [code, setCode] = useState(null);
  const [error, setError] = useState(null);
  const downloadable = useDownload();
  useEffect(() => { api('/agents/pairing-codes', { method: 'POST' }).then(setCode).catch(setError); }, []);
  return (
    <Modal title="Pair a computer" subtitle="Run the device companion on the computer you want to add." onClose={onClose}>
      <ErrorBanner error={error} />
      <ol className="steps">
        <li>
          {downloadable
            ? <>Download the Windows companion: <a className="outline inline" href={DOWNLOAD} download><Download size={14} /> devicetrustline-agent.exe</a></>
            : <>Get <code>devicetrustline-agent.exe</code> from your administrator (it is built with this release; see the agent README).</>}
        </li>
        <li>On that computer, open a command prompt as administrator in the download folder and run:
          {code && <pre className="code">devicetrustline-agent pair {code.server} {code.code}{'\n'}devicetrustline-agent report{'\n'}devicetrustline-agent install</pre>}
        </li>
        <li>Its hardware appears below for review. Nothing becomes an item until you add it.</li>
      </ol>
      {code && <p className="hint">This code works once and expires at {new Date(code.expiresAt).toLocaleTimeString()}.</p>}
      <div className="modal-actions"><button className="primary" onClick={onClose}>Done</button></div>
    </Modal>
  );
}

function AddDialog({ device, onClose, onDone }) {
  const { categories } = useCatalog();
  const guess = categories.find((c) => c.name.toLowerCase() === { monitor: 'monitor', printer: 'printer', dock: 'docking station', phone: 'phone' }[device.class])
    ?? categories.find((c) => c.name === 'Other') ?? categories[0];
  const [form, setForm] = useState({ name: device.name, type: guess?.name ?? '' });
  const [error, setError] = useState(null);
  async function submit(e) {
    e.preventDefault();
    try { onDone(await api(`/discovered-devices/${device.id}/add`, { method: 'POST', body: form })); } catch (err) { setError(err); }
  }
  return (
    <Modal title="Add as a new item" subtitle="Only what the computer reported is filled in. Add the purchase date, price and warranty yourself." onClose={onClose}>
      <ErrorBanner error={error} />
      <form onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name"><input required value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} /></Field>
          <Field label="Category">
            <select value={form.type} onChange={(e) => setForm({ ...form, type: e.target.value })}>{categories.map((c) => <option key={c.id}>{c.name}</option>)}</select>
          </Field>
        </div>
        <dl className="detail-grid compact">
          <div><dt>Manufacturer</dt><dd>{device.manufacturer || '—'}</dd></div>
          <div><dt>Model</dt><dd>{device.model || '—'}</dd></div>
          <div><dt>Serial</dt><dd>{device.serial || 'Not reported'}</dd></div>
          {hardwareLines(device.details).map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{v}</dd></div>)}
        </dl>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onClose}>Cancel</button>
          <button className="primary"><Plus size={16} /> Add item</button>
        </div>
      </form>
    </Modal>
  );
}

function LinkDialog({ device, onClose, onDone }) {
  const [q, setQ] = useState(device.serial || device.model || '');
  const [results, setResults] = useState([]);
  const [error, setError] = useState(null);
  useEffect(() => {
    const t = setTimeout(() => api(`/assets?limit=20&q=${encodeURIComponent(q)}`).then((r) => setResults(r.items)).catch(setError), 250);
    return () => clearTimeout(t);
  }, [q]);
  async function link(asset) {
    try { await api(`/discovered-devices/${device.id}/link`, { method: 'POST', body: { assetId: asset.id } }); onDone(); } catch (err) { setError(err); }
  }
  return (
    <Modal title={`Link ${device.name}`} subtitle="Pick the item this device already is." onClose={onClose}>
      <ErrorBanner error={error} />
      <input className="full-input" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search by name, tag or serial" aria-label="Search items" />
      <ul className="pick-list">
        {results.map((a) => (
          <li key={a.id}><button onClick={() => link(a)}><strong>{a.name}</strong><small>{a.assetTag} · {a.type}{a.serial ? ` · ${a.serial}` : ''}</small></button></li>
        ))}
        {results.length === 0 && <li className="muted">No matching items.</li>}
      </ul>
    </Modal>
  );
}

export function Devices({ onSelect, onChanged }) {
  const { can } = useAuth();
  const [agents, setAgents] = useState([]);
  const [devices, setDevices] = useState(null);
  const [filter, setFilter] = useState('new');
  const [error, setError] = useState(null);
  const [pairing, setPairing] = useState(false);
  const [adding, setAdding] = useState(null);
  const [linking, setLinking] = useState(null);
  const manage = can('devices:manage');
  const write = can('assets:write');

  const load = useCallback(() => {
    api('/agents').then((r) => setAgents(r.items)).catch(setError);
    api(`/discovered-devices?state=${filter}`).then((r) => setDevices(r.items)).catch(setError);
  }, [filter]);
  useEffect(() => { load(); }, [load]);

  const act = async (fn) => { try { await fn(); setError(null); load(); } catch (err) { setError(err); } };

  return (
    <>
      <ErrorBanner error={error} />
      <section className="panel">
        <div className="panel-head wrap">
          <div><h2>Computers <span className="count">{agents.filter((a) => !a.revokedAt).length}</span></h2><p>Computers running the device companion for this workspace.</p></div>
          {manage && <button className="outline" onClick={() => setPairing(true)}><KeyRound size={16} /> Pair a computer</button>}
        </div>
        <div className="table-scroll">
          <table>
            <thead><tr><th>COMPUTER</th><th>LAST REPORT</th><th>DEVICES</th><th>STATE</th>{manage && <th />}</tr></thead>
            <tbody>
              {agents.length === 0 && <tr><td colSpan="5" className="empty">No computers paired yet.</td></tr>}
              {agents.map((a) => (
                <tr key={a.id}>
                  <td><strong className="normal">{a.name}</strong><small>{a.platform}{a.version ? ` · v${a.version}` : ''}</small></td>
                  <td>{when(a.lastSeenAt)}</td>
                  <td>{a.devices}{a.newDevices > 0 && <small>{a.newDevices} to review</small>}</td>
                  <td>
                    <span className={`status ${a.revokedAt ? 'retired' : a.paused ? 'maintenance' : 'deployed'}`}><i />{a.revokedAt ? 'Disconnected' : a.paused ? 'Paused' : 'Active'}</span>
                  </td>
                  {manage && (
                    <td className="row-end">
                      {!a.revokedAt && (
                        <>
                          <button className="outline" onClick={() => act(() => api(`/agents/${a.id}`, { method: 'PATCH', body: { paused: !a.paused } }))}>
                            {a.paused ? <><Play size={14} /> Resume</> : <><Pause size={14} /> Pause</>}
                          </button>
                          <button className="outline" onClick={() => window.confirm(`Disconnect ${a.name}? Its credential stops working immediately.`) && act(() => api(`/agents/${a.id}`, { method: 'DELETE' }))}>
                            <Unplug size={14} /> Disconnect
                          </button>
                        </>
                      )}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <section className="panel">
        <div className="panel-head wrap">
          <div><h2>Found devices</h2><p>Review what the companion found: add it as an item, link it to one you have, or ignore it.</p></div>
          <div className="segmented" role="group" aria-label="Show">
            {[['new', 'To review'], ['linked', 'Linked'], ['ignored', 'Ignored'], ['all', 'All']].map(([k, label]) => (
              <button key={k} className={filter === k ? 'on' : ''} aria-pressed={filter === k} onClick={() => setFilter(k)}>{label}</button>
            ))}
          </div>
        </div>
        <div className="table-scroll">
          <table>
            <thead><tr><th>DEVICE</th><th>SERIAL</th><th>SEEN ON</th><th>LAST SEEN</th><th /></tr></thead>
            <tbody>
              {devices === null && <tr><td colSpan="5" className="empty">Loading…</td></tr>}
              {devices?.length === 0 && <tr><td colSpan="5" className="empty">Nothing here.</td></tr>}
              {devices?.map((d) => (
                <tr key={d.id}>
                  <td>
                    <strong className="normal">{d.name}</strong>
                    <small>{d.class}{d.manufacturer ? ` · ${d.manufacturer}` : ''}{d.model && d.model !== d.name ? ` · ${d.model}` : ''}</small>
                    {d.class === 'computer' && hardwareLines(d.details).map(([k, v]) => <small key={k}>{k}: {v}</small>)}
                  </td>
                  <td className="mono">{d.serial || <span className="muted">Not reported</span>}</td>
                  <td>{d.agentName}</td>
                  <td>{d.missingSince ? <span className="status missing"><i />Not seen since {new Date(d.missingSince).toLocaleDateString()}</span> : when(d.lastSeenAt)}</td>
                  <td className="row-end">
                    {d.state === 'linked' && d.assetId && (
                      <button className="link-text" onClick={() => onSelect({ id: d.assetId, name: d.assetName, assetTag: d.assetTag, type: '', status: '' })}>{d.assetTag}</button>
                    )}
                    {write && d.state === 'new' && (
                      <>
                        <button className="outline" onClick={() => setAdding(d)}><Plus size={14} /> Add</button>
                        <button className="outline" onClick={() => setLinking(d)}><Link2 size={14} /> Link</button>
                        <button className="outline" onClick={() => act(() => api(`/discovered-devices/${d.id}/ignore`, { method: 'POST' }))}><EyeOff size={14} /> Ignore</button>
                      </>
                    )}
                    {write && d.state !== 'new' && (
                      <button className="outline" onClick={() => act(() => api(`/discovered-devices/${d.id}/restore`, { method: 'POST' }))}><RotateCcw size={14} /> Review again</button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      {pairing && <PairDialog onClose={() => { setPairing(false); load(); }} />}
      {adding && <AddDialog device={adding} onClose={() => setAdding(null)} onDone={(item) => { setAdding(null); load(); onChanged(); onSelect(item); }} />}
      {linking && <LinkDialog device={linking} onClose={() => setLinking(null)} onDone={() => { setLinking(null); load(); onChanged(); }} />}
    </>
  );
}
