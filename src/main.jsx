import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ScanLine } from 'lucide-react';
import { api } from './api.js';
import { App } from './App.jsx';
import { AuthProvider, useAuth } from './auth.jsx';
import { CatalogProvider } from './catalog.jsx';
import { Invite } from './pages/Invite.jsx';
import { ConfirmEmail, Login, googleError } from './pages/Login.jsx';
import '../style.css';

// Links from emails and from Google arrive as query parameters; read them once and tidy the URL.
const params = new URLSearchParams(window.location.search);
const link = {
  verify: params.get('verify'),
  reset: params.get('reset'),
  invite: params.get('invite'),
  googleError: googleError(params.get('google_error')),
  google: params.get('google'),
};
for (const key of ['verify', 'reset', 'google_error', 'google']) params.delete(key);
if (!link.invite) window.history.replaceState(null, '', `${window.location.pathname}${params.size ? `?${params}` : ''}`);

function Root() {
  const { user, loading, refresh } = useAuth();
  const [invite, setInvite] = useState(link.invite);
  const [verified, setVerified] = useState(null);
  const [config, setConfig] = useState(null);
  useEffect(() => { api('/auth/config').then(setConfig).catch(() => setConfig({})); }, []);

  useEffect(() => {
    if (!link.verify) return;
    api('/auth/verify-email', { method: 'POST', body: { token: link.verify } })
      .then(() => { setVerified('Your email address is confirmed.'); refresh(); })
      .catch((err) => setVerified(err.message));
  }, [refresh]);

  if (loading) return <div className="splash"><div className="brand-icon"><ScanLine size={22} /></div></div>;
  if (invite) return <Invite token={invite} onDone={() => setInvite(null)} />;
  if (!user) return <Login resetToken={link.reset} notice={verified} initialError={link.googleError} />;
  if (!user.emailVerified && config?.emailVerificationRequired) return <ConfirmEmail notice={verified} />;
  return (
    <CatalogProvider key={user.organization.id}>
      <App notice={verified || (link.google === 'linked' ? 'Google is now connected to your account.' : null)} initialError={link.googleError} />
    </CatalogProvider>
  );
}

createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <AuthProvider><Root /></AuthProvider>
  </React.StrictMode>,
);
