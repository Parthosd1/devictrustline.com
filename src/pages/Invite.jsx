import React, { useEffect, useState } from 'react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner } from '../components.jsx';
import { ROLE_LABELS } from '../constants.js';
import { AuthCard } from './Login.jsx';

// Opened from an invitation link (/?invite=…), signed in or not.
export function Invite({ token, onDone }) {
  const { setUser, signOut } = useAuth();
  const [invite, setInvite] = useState(null);
  const [error, setError] = useState(null);
  const [form, setForm] = useState({ name: '', password: '' });
  const [busy, setBusy] = useState(false);

  useEffect(() => { api(`/invitations/lookup?token=${encodeURIComponent(token)}`).then(setInvite).catch(setError); }, [token]);

  async function accept(e) {
    e?.preventDefault();
    setBusy(true);
    try {
      const body = invite.accountExists ? { token } : { token, ...form };
      const r = await api('/invitations/accept', { method: 'POST', body });
      setUser(r.user);
      onDone();
    } catch (err) { setError(err); setBusy(false); }
  }

  const finish = () => { window.history.replaceState(null, '', '/'); onDone(); };
  if (!invite) {
    return (
      <AuthCard>
        <h1>Invitation</h1>
        <ErrorBanner error={error} />
        {!error && <p className="auth-sub">Checking your invitation…</p>}
        {error && <button className="text-link" onClick={finish}>Continue</button>}
      </AuthCard>
    );
  }
  const signedInAsInvitee = invite.signedInAs && invite.signedInAs.toLowerCase() === invite.email.toLowerCase();
  return (
    <AuthCard>
      <h1>Join {invite.workspace}</h1>
      <p className="auth-sub">You were invited as {ROLE_LABELS[invite.role]} ({invite.email}).</p>
      <ErrorBanner error={error} />
      {invite.accountExists ? (
        signedInAsInvitee ? (
          <button className="primary wide-btn" disabled={busy} onClick={accept}>Accept and open {invite.workspace}</button>
        ) : (
          <>
            <p className="hint">
              {invite.signedInAs
                ? `You are signed in as ${invite.signedInAs}. Sign out, then sign in as ${invite.email} to accept.`
                : `${invite.email} already has an account. Sign in to it, then open the invitation link again.`}
            </p>
            {invite.signedInAs
              ? <button className="primary wide-btn" onClick={signOut}>Sign out</button>
              : <button className="primary wide-btn" onClick={onDone}>Sign in</button>}
          </>
        )
      ) : (
        <form className="auth-form" onSubmit={accept}>
          <label>Your name<input required value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} autoComplete="name" /></label>
          <label>Choose a password<input required type="password" minLength={12} value={form.password}
            onChange={(e) => setForm({ ...form, password: e.target.value })} autoComplete="new-password" /><small>At least 12 characters.</small></label>
          <button className="primary" disabled={busy}>Create account and join</button>
        </form>
      )}
      <button className="text-link auth-switch" onClick={finish}>Not now</button>
    </AuthCard>
  );
}
