import React, { useCallback, useEffect, useState } from 'react';
import {
  Boxes, Building2, ChevronRight, ClipboardCheck, Contact, BarChart3, LayoutDashboard, LogOut, Menu, Plus, ScanLine, Settings as SettingsIcon, Users as UsersIcon, Wrench,
} from 'lucide-react';
import { api } from './api.js';
import { useAuth } from './auth.jsx';
import { ErrorBanner } from './components.jsx';
import { ROLE_LABELS } from './constants.js';
import { AssetDetail } from './pages/AssetDetail.jsx';
import { AssetForm } from './pages/AssetForm.jsx';
import { Audits } from './pages/Audits.jsx';
import { Buildings } from './pages/Buildings.jsx';
import { Dashboard } from './pages/Dashboard.jsx';
import { Inventory } from './pages/Inventory.jsx';
import { Maintenance } from './pages/Maintenance.jsx';
import { People } from './pages/People.jsx';
import { Reports } from './pages/Reports.jsx';
import { Scan } from './pages/Scan.jsx';
import { Settings } from './pages/Settings.jsx';
import { Users } from './pages/Users.jsx';

const PAGES = {
  Dashboard: { icon: LayoutDashboard, title: 'Asset Overview', blurb: 'Your equipment, locations and audit activity in one place.' },
  Inventory: { icon: Boxes, title: 'Inventory', blurb: 'Search and manage every asset across your facilities.' },
  Scan: { icon: ScanLine, title: 'Scan', blurb: 'Find any asset by scanning its label or barcode.' },
  Audits: { icon: ClipboardCheck, title: 'Audits', blurb: 'Run weekly and monthly physical audits by scanning, and keep a permanent record of each.' },
  Buildings: { icon: Building2, title: 'Buildings', blurb: 'A clear view of your equipment by facility and location.' },
  People: { icon: Contact, title: 'People', blurb: 'Employees and staff who can be assigned equipment.' },
  Maintenance: { icon: Wrench, title: 'Maintenance', blurb: 'Track repairs from report to resolution, with cost and history.' },
  Reports: { icon: BarChart3, title: 'Reports', blurb: 'Inventory, warranty, maintenance and audit reporting, with CSV downloads.' },
  Users: { icon: UsersIcon, title: 'Users', blurb: 'Who can sign in to this workspace, and what they can do.', permission: 'users:read' },
  Settings: { icon: SettingsIcon, title: 'Settings', blurb: 'Workspace automation and your account.' },
};

const initials = (name) => name.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join('') || '?';

export function App() {
  const { user, signOut, can } = useAuth();
  const [page, setPage] = useState('Dashboard');
  const [menu, setMenu] = useState(false);
  const [assets, setAssets] = useState([]);
  const [buildings, setBuildings] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [selected, setSelected] = useState(null);
  const [editing, setEditing] = useState(null);
  const [version, setVersion] = useState(0);

  const reload = useCallback(async () => {
    try {
      const [a, b] = await Promise.all([api('/assets?limit=500'), api('/buildings')]);
      setAssets(a.items);
      setBuildings(b.items);
      setError(null);
    } catch (err) {
      setError(err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { reload(); }, [reload, version]);

  // A label's QR code links to /?asset=DT-1001; open that asset once signed in.
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const tag = params.get('asset');
    if (!tag) return;
    params.delete('asset');
    window.history.replaceState(null, '', `${window.location.pathname}${params.size ? `?${params}` : ''}`);
    api(`/assets/lookup?code=${encodeURIComponent(tag)}`).then(setSelected).catch(setError);
  }, []);
  const refresh = () => setVersion((v) => v + 1);

  const go = (name) => { setPage(name); setMenu(false); };
  const visible = Object.entries(PAGES).filter(([, p]) => !p.permission || can(p.permission));
  const meta = PAGES[page];

  function onSaved(asset) {
    setEditing(null);
    setSelected(asset);
    refresh();
  }

  return (
    <div className="shell">
      <aside className={`sidebar ${menu ? 'open' : ''}`}>
        <div className="brand">
          <div className="brand-icon"><ScanLine size={22} /></div>
          <div><strong>DeviceTrustline</strong><small>ASSET INTELLIGENCE</small></div>
        </div>
        <div className="nav-label">WORKSPACE</div>
        <nav>
          {visible.map(([name, { icon: Icon }]) => (
            <button key={name} className={`nav-item ${page === name ? 'active' : ''}`} onClick={() => go(name)}>
              <Icon size={19} />{name}{page === name && <ChevronRight size={15} className="nav-chevron" />}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="workspace-avatar">{initials(user.organization.name)}</div>
          <div><strong>{user.organization.name}</strong><small>{ROLE_LABELS[user.role]}</small></div>
        </div>
      </aside>
      {menu && <div className="sidebar-scrim" onClick={() => setMenu(false)} />}
      <div className="main">
        <header className="topbar">
          <button className="mobile-menu" onClick={() => setMenu(!menu)} aria-label="Toggle menu"><Menu size={22} /></button>
          <div className="breadcrumb">{user.organization.name} <ChevronRight size={15} /> <strong>{page}</strong></div>
          <div className="top-right">
            <span className="user-label"><strong>{user.name}</strong><small>{user.email}</small></span>
            <span className="avatar" title={user.name}>{initials(user.name)}</span>
            <button className="icon-btn" onClick={signOut} aria-label="Sign out" title="Sign out"><LogOut size={18} /></button>
          </div>
        </header>
        <main className="content">
          <div className="heading">
            <div>
              <div className="eyebrow">DEVICE TRUSTLINE / {page.toUpperCase()}</div>
              <h1>{meta.title}</h1>
              <p>{meta.blurb}</p>
            </div>
            {can('assets:write') && (
              <button className="primary" onClick={() => setEditing({})}><Plus size={17} /> Add Asset</button>
            )}
          </div>
          <ErrorBanner error={error} />
          {page === 'Dashboard' && <Dashboard assets={assets} loading={loading} onSelect={setSelected} go={go} version={version} />}
          {page === 'Inventory' && <Inventory version={version} buildings={buildings} onSelect={setSelected} onChanged={refresh} />}
          {page === 'Scan' && <Scan onSelect={setSelected} />}
          {page === 'Audits' && <Audits buildings={buildings} onSelect={setSelected} onChanged={refresh} />}
          {page === 'Buildings' && <Buildings buildings={buildings} onChanged={refresh} />}
          {page === 'Maintenance' && <Maintenance assets={assets} onSelect={setSelected} onChanged={refresh} version={version} />}
          {page === 'People' && <People onSelect={setSelected} version={version} />}
          {page === 'Reports' && <Reports onSelect={setSelected} version={version} />}
          {page === 'Users' && <Users />}
          {page === 'Settings' && <Settings />}
        </main>
      </div>
      {selected && !editing && (
        <AssetDetail key={`${selected.id}-${version}`} asset={selected} onClose={() => setSelected(null)}
          onEdit={(a) => setEditing(a)} onSelect={setSelected} onChanged={onSaved} onRefresh={refresh} />
      )}
      {editing && (
        <AssetForm asset={editing.id ? editing : null} buildings={buildings} assets={assets} onClose={() => setEditing(null)} onSaved={onSaved} />
      )}
    </div>
  );
}
