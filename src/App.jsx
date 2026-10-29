import React, { useCallback, useEffect, useState } from 'react';
import {
  AlarmClock, ArrowRightLeft, BarChart3, Bell, Boxes, Building2, Handshake, Inbox, ChevronRight, ClipboardCheck, Contact, CreditCard, LayoutDashboard, LogOut, Menu, MonitorSmartphone, Plus,
  ScanLine, Settings as SettingsIcon, Tags, Users as UsersIcon, Wrench,
} from 'lucide-react';
import { api } from './api.js';
import { useAuth } from './auth.jsx';
import { nouns, useCatalog } from './catalog.jsx';
import { ErrorBanner } from './components.jsx';
import { ROLE_LABELS } from './constants.js';
import { NotificationBell } from './notifications.jsx';
import { Approvals } from './pages/Approvals.jsx';
import { AssetDetail } from './pages/AssetDetail.jsx';
import { AssetForm } from './pages/AssetForm.jsx';
import { Audits } from './pages/Audits.jsx';
import { Buildings } from './pages/Buildings.jsx';
import { Catalog } from './pages/Catalog.jsx';
import { Dashboard } from './pages/Dashboard.jsx';
import { Devices } from './pages/Devices.jsx';
import { Inventories } from './pages/Inventories.jsx';
import { Inventory } from './pages/Inventory.jsx';
import { Loaners } from './pages/Loaners.jsx';
import { Maintenance } from './pages/Maintenance.jsx';
import { NotificationCenter } from './pages/Notifications.jsx';
import { People } from './pages/People.jsx';
import { Plan } from './pages/Plan.jsx';
import { Reminders } from './pages/Reminders.jsx';
import { Reports } from './pages/Reports.jsx';
import { Scan } from './pages/Scan.jsx';
import { Settings } from './pages/Settings.jsx';
import { Users } from './pages/Users.jsx';
import { WorkspaceSwitcher } from './workspaces.jsx';

// Which pages each kind of workspace shows, and how they are named there.
function pagesFor(kind) {
  const n = nouns(kind);
  const personal = kind === 'personal';
  return {
    Dashboard: { icon: LayoutDashboard, title: personal ? 'My Things' : 'Asset Overview', blurb: personal ? 'Your belongings, warranties and upcoming deadlines.' : 'Your equipment, locations and audit activity in one place.' },
    Approvals: { icon: Inbox, title: 'Approval Center', blurb: 'Transfers, audits and access waiting on a decision, exceptions to follow up, and who is on it.', hidden: personal },
    Inventory: { icon: Boxes, label: n.Many, title: n.Many, blurb: `Search and manage every ${n.one}.` },
    Inventories: { icon: ArrowRightLeft, label: 'Inventories', title: 'Inventories & Transfers', blurb: 'Hubs and stock rooms, what is in each, and every move between them.', hidden: personal, permission: 'inventory:read' },
    Loaners: { icon: Handshake, title: 'Loaners', blurb: 'Reserve, hand over, check in and inspect loaner equipment.', hidden: personal, permission: 'loans:read' },
    Scan: { icon: ScanLine, title: 'Scan', blurb: `Find any ${n.one} by scanning its label or barcode.` },
    Audits: { icon: ClipboardCheck, title: 'Audits', blurb: 'Run scheduled and one-off audits by scanning, send them for review, and keep a permanent record of each.', hidden: personal },
    Buildings: { icon: Building2, label: n.places, title: n.places, blurb: personal ? 'Rooms and places where your things are kept.' : 'A clear view of your equipment by facility and location.' },
    People: { icon: Contact, title: 'People', blurb: personal ? 'People you lend things to.' : 'Employees and staff who can be assigned equipment.' },
    Maintenance: { icon: Wrench, title: 'Maintenance', blurb: 'Track repairs from report to resolution, with cost and history.' },
    Reminders: { icon: AlarmClock, title: 'Reminders', blurb: 'Your own reminders for warranties, returns, maintenance and renewals.' },
    Notifications: { icon: Bell, title: 'Notifications', blurb: 'Everything sent to you, and how each was delivered.' },
    Devices: { icon: MonitorSmartphone, title: 'Devices', blurb: 'Pair computers with the device companion and review the hardware it finds.' },
    Reports: { icon: BarChart3, title: 'Reports', blurb: 'Inventory, warranty, maintenance and audit reporting, with CSV downloads.', hidden: personal },
    Catalog: { icon: Tags, label: 'Categories & Fields', title: 'Categories & Fields', blurb: `How you describe your ${n.many}: categories, tags and your own fields.` },
    Users: { icon: UsersIcon, label: personal ? 'Members' : 'Users', title: personal ? 'Members' : 'Users', blurb: 'Who can sign in to this workspace, and what they can do.', permission: 'users:read' },
    Plan: { icon: CreditCard, label: 'Plan', title: 'Plan & Billing', blurb: 'What your plan includes and how much of it you use.' },
    Settings: { icon: SettingsIcon, title: 'Settings', blurb: 'Your account, reminders and workspace preferences.' },
  };
}

const initials = (name) => name.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join('') || '?';

export function App({ notice: initialNotice, initialError }) {
  const { user, signOut, can } = useAuth();
  const catalog = useCatalog();
  const kind = user.organization.kind;
  const n = nouns(kind);
  const PAGES = pagesFor(kind);
  const [page, setPage] = useState('Dashboard');
  const [menu, setMenu] = useState(false);
  const [assets, setAssets] = useState([]);
  const [buildings, setBuildings] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(initialError ? new Error(initialError) : null);
  const [notice, setNotice] = useState(initialNotice);
  const [selected, setSelected] = useState(null);
  const [editing, setEditing] = useState(null);
  const [version, setVersion] = useState(0);

  const reload = useCallback(async () => {
    try {
      const [a, b] = await Promise.all([api('/assets?limit=500'), api('/buildings')]);
      setAssets(a.items);
      setBuildings(b.items);
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
  const refresh = () => { setVersion((v) => v + 1); catalog.reload(); };

  const go = (name) => { setPage(name); setMenu(false); };
  const visible = Object.entries(PAGES).filter(([, p]) => !p.hidden && (!p.permission || can(p.permission)));
  const shown = (name) => PAGES[name] && !PAGES[name].hidden && (!PAGES[name].permission || can(PAGES[name].permission));
  const meta = shown(page) ? PAGES[page] : PAGES.Dashboard;
  const current = shown(page) ? page : 'Dashboard';

  function onSaved(asset) {
    setEditing(null);
    setSelected(asset);
    refresh();
  }

  return (
    <div className="shell">
      <aside className={`sidebar ${menu ? 'open' : ''}`} aria-label="Main navigation">
        <div className="brand">
          <div className="brand-icon"><ScanLine size={22} /></div>
          <div><strong>DeviceTrustline</strong><small>{kind === 'personal' ? 'PERSONAL' : 'ASSET INTELLIGENCE'}</small></div>
        </div>
        <div className="nav-label">WORKSPACE</div>
        <nav>
          {visible.map(([name, { icon: Icon, label }]) => (
            <button key={name} className={`nav-item ${current === name ? 'active' : ''}`} onClick={() => go(name)} aria-current={current === name ? 'page' : undefined}>
              <Icon size={19} />{label || name}{current === name && <ChevronRight size={15} className="nav-chevron" />}
            </button>
          ))}
        </nav>
        <WorkspaceSwitcher onSwitched={() => { setPage('Dashboard'); setSelected(null); setMenu(false); }} />
      </aside>
      {menu && <div className="sidebar-scrim" onClick={() => setMenu(false)} />}
      <div className="main">
        <header className="topbar">
          <button className="mobile-menu" onClick={() => setMenu(!menu)} aria-label="Toggle menu" aria-expanded={menu}><Menu size={22} /></button>
          <div className="breadcrumb">{user.organization.name} <ChevronRight size={15} /> <strong>{meta.label || current}</strong></div>
          <div className="top-right">
            <NotificationBell onOpenAsset={setSelected} onViewAll={() => go('Notifications')} />
            <span className="user-label"><strong>{user.name}</strong><small>{user.email} · {ROLE_LABELS[user.role]}</small></span>
            <span className="avatar" title={user.name}>{initials(user.name)}</span>
            <button className="icon-btn" onClick={signOut} aria-label="Sign out" title="Sign out"><LogOut size={18} /></button>
          </div>
        </header>
        <main className="content">
          <div className="heading">
            <div>
              <div className="eyebrow">DEVICE TRUSTLINE / {(meta.label || current).toUpperCase()}</div>
              <h1>{meta.title}</h1>
              <p>{meta.blurb}</p>
            </div>
            {can('assets:write') && (
              <button className="primary" onClick={() => setEditing({})}><Plus size={17} /> Add {n.One}</button>
            )}
          </div>
          <ErrorBanner error={error} />
          {notice && <div className="notice dismissable" role="status">{notice}<button className="text-link" onClick={() => setNotice(null)}>Dismiss</button></div>}
          {current === 'Dashboard' && <Dashboard assets={assets} loading={loading} onSelect={setSelected} go={go} version={version} />}
          {current === 'Approvals' && <Approvals onSelect={setSelected} onChanged={refresh} go={go} />}
          {current === 'Inventories' && <Inventories buildings={buildings} onSelect={setSelected} onChanged={refresh} />}
          {current === 'Loaners' && <Loaners onSelect={setSelected} onChanged={refresh} />}
          {current === 'Reminders' && <Reminders assets={assets} />}
          {current === 'Notifications' && <NotificationCenter onOpenAsset={setSelected} />}
          {current === 'Inventory' && <Inventory version={version} buildings={buildings} onSelect={setSelected} onChanged={refresh} />}
          {current === 'Scan' && <Scan onSelect={setSelected} />}
          {current === 'Audits' && <Audits buildings={buildings} onSelect={setSelected} onChanged={refresh} />}
          {current === 'Buildings' && <Buildings buildings={buildings} onChanged={refresh} />}
          {current === 'Maintenance' && <Maintenance assets={assets} onSelect={setSelected} onChanged={refresh} version={version} />}
          {current === 'People' && <People onSelect={setSelected} version={version} />}
          {current === 'Devices' && <Devices onSelect={setSelected} onChanged={refresh} />}
          {current === 'Reports' && <Reports onSelect={setSelected} version={version} />}
          {current === 'Catalog' && <Catalog />}
          {current === 'Users' && <Users />}
          {current === 'Plan' && <Plan />}
          {current === 'Settings' && <Settings />}
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
