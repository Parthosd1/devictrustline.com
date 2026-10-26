import React, { useEffect, useState } from 'react';
import { Briefcase, Home, ScanLine } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner } from '../components.jsx';

const GOOGLE_ERRORS = {
  link_required: 'An account with this email already exists. Sign in with your password, then connect Google under Settings.',
  email_unverified: 'Google has not verified this email address, so it cannot be used to create an account.',
  no_account: 'There is no account for this Google login, and sign-up is closed. Ask your administrator for an invitation.',
  already_linked: 'That Google account is already connected to a different account.',
  cancelled: 'Google sign-in was cancelled.',
  invalid: 'That sign-in link was not valid or was already used. Please try again.',
  unavailable: 'Google sign-in is not available right now. Please try again, or use your password.',
  unconfigured: 'Google sign-in is not set up on this server.',
};

export const googleError = (code) => GOOGLE_ERRORS[code] || (code ? 'Google sign-in failed. Please try again.' : null);

function Brand() {
  return (
    <div className="brand auth-brand">
      <div className="brand-icon"><ScanLine size={22} /></div>
      <div><strong>DeviceTrustline</strong><small>ASSET INTELLIGENCE</small></div>
    </div>
  );
}

export function AuthCard({ children }) {
  return <div className="auth-page"><div className="auth-card"><Brand />{children}</div></div>;
}

function GoogleButton({ label = 'Continue with Google' }) {
  return (
    <a className="google-btn" href="/api/auth/google/start?mode=login">
      <svg width="18" height="18" viewBox="0 0 48 48" aria-hidden="true"><path fill="#FFC107" d="M43.6 20.5H42V20H24v8h11.3C33.7 32.7 29.3 36 24 36c-6.6 0-12-5.4-12-12s5.4-12 12-12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 12.9 4 4 12.9 4 24s8.9 20 20 20 20-8.9 20-20c0-1.3-.1-2.4-.4-3.5z" /><path fill="#FF3D00" d="m6.3 14.7 6.6 4.8C14.7 15.1 19 12 24 12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 16.3 4 9.7 8.3 6.3 14.7z" /><path fill="#4CAF50" d="M24 44c5.2 0 9.9-2 13.4-5.2l-6.2-5.2C29.2 35.1 26.7 36 24 36c-5.3 0-9.7-3.3-11.3-8l-6.5 5C9.5 39.6 16.2 44 24 44z" /><path fill="#1976D2" d="M43.6 20.5H42V20H24v8h11.3c-.8 2.2-2.2 4.2-4.1 5.6l6.2 5.2C37 39.2 44 34 44 24c0-1.3-.1-2.4-.4-3.5z" /></svg>
      {label}
    </a>
  );
}

export function Login({ initialMode = 'signin', resetToken = null, notice: initialNotice = null, initialError = null }) {
  const { setUser } = useAuth();
  const [mode, setMode] = useState(resetToken ? 'reset' : initialMode);
  const [config, setConfig] = useState({ signup: false, google: false, email: false });
  const [form, setForm] = useState({ accountType: 'personal', organizationName: '', name: '', email: '', password: '' });
  const [error, setError] = useState(initialError ? new Error(initialError) : null);
  const [notice, setNotice] = useState(initialNotice);
  const [busy, setBusy] = useState(false);
  const set = (key) => (e) => setForm({ ...form, [key]: e.target.value });

  useEffect(() => { api('/auth/config').then(setConfig).catch(() => {}); }, []);

  const switchTo = (m) => { setMode(m); setError(null); setNotice(null); };

  async function submit(e) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (mode === 'signin') {
        setUser((await api('/auth/login', { method: 'POST', body: { email: form.email, password: form.password } })).user);
      } else if (mode === 'signup') {
        const body = { ...form };
        if (body.accountType === 'personal' && !body.organizationName) delete body.organizationName;
        setUser((await api('/auth/signup', { method: 'POST', body })).user);
      } else if (mode === 'forgot') {
        const r = await api('/auth/password-reset/request', { method: 'POST', body: { email: form.email } });
        setNotice(r.emailAvailable
          ? 'If that address has an account, a reset link is on its way. It works once and expires in 1 hour.'
          : 'Email is not set up on this server, so reset links cannot be sent. Ask your administrator to set a new password for you.');
        setBusy(false);
      } else if (mode === 'reset') {
        await api('/auth/password-reset/confirm', { method: 'POST', body: { token: resetToken, password: form.password } });
        window.history.replaceState(null, '', '/');
        setMode('signin');
        setNotice('Password changed. Sign in with your new password.');
        setBusy(false);
      }
    } catch (err) {
      setError(err);
      setBusy(false);
    }
  }

  const titles = {
    signin: ['Sign in', 'Track every device, warranty and receipt in one place.'],
    signup: ['Create your account', 'Choose how you will use DeviceTrustline. You can add more workspaces later.'],
    forgot: ['Reset your password', 'Enter your email and we will send you a link to choose a new password.'],
    reset: ['Choose a new password', 'At least 12 characters. This signs you out on every other device.'],
  };
  const business = form.accountType === 'business';
  return (
    <AuthCard>
      <h1>{titles[mode][0]}</h1>
      <p className="auth-sub">{titles[mode][1]}</p>
      <ErrorBanner error={error} />
      {notice && <div className="notice" role="status">{notice}</div>}
      <form onSubmit={submit} className="auth-form">
        {mode === 'signup' && (
          <>
            <fieldset className="choice">
              <legend>Account type</legend>
              <label className={!business ? 'on' : ''}>
                <input type="radio" name="accountType" value="personal" checked={!business} onChange={set('accountType')} />
                <Home size={18} /><span><strong>Personal</strong><small>Your belongings, receipts, warranties and reminders</small></span>
              </label>
              <label className={business ? 'on' : ''}>
                <input type="radio" name="accountType" value="business" checked={business} onChange={set('accountType')} />
                <Briefcase size={18} /><span><strong>Business</strong><small>Locations, employees, assignments, maintenance and audits</small></span>
              </label>
            </fieldset>
            {business && <label>Organization name<input required value={form.organizationName} onChange={set('organizationName')} autoComplete="organization" /></label>}
            <label>Your name<input required value={form.name} onChange={set('name')} autoComplete="name" /></label>
          </>
        )}
        {mode !== 'reset' && (
          <label>Email<input required type="email" value={form.email} onChange={set('email')} autoComplete="email" /></label>
        )}
        {mode !== 'forgot' && (
          <label>
            {mode === 'reset' ? 'New password' : 'Password'}
            <input required type="password" minLength={mode === 'signin' ? undefined : 12} value={form.password} onChange={set('password')}
              autoComplete={mode === 'signin' ? 'current-password' : 'new-password'} />
            {mode !== 'signin' && <small>At least 12 characters.</small>}
          </label>
        )}
        <button className="primary" disabled={busy}>
          {busy ? 'Please wait…' : { signin: 'Sign in', signup: 'Create account', forgot: 'Send reset link', reset: 'Set new password' }[mode]}
        </button>
      </form>
      {config.google && (mode === 'signin' || mode === 'signup') && (
        <>
          <div className="or"><span>or</span></div>
          <GoogleButton />
          {mode === 'signup' && <p className="hint">Signing up with Google creates a personal account.</p>}
        </>
      )}
      <div className="auth-links">
        {mode === 'signin' && <button className="text-link" onClick={() => switchTo('forgot')}>Forgot your password?</button>}
        {mode === 'signin' && config.signup && <button className="text-link" onClick={() => switchTo('signup')}>New to DeviceTrustline? Create an account</button>}
        {mode === 'signin' && !config.signup && <p className="hint">New accounts are by invitation. Ask your administrator.</p>}
        {mode !== 'signin' && <button className="text-link" onClick={() => switchTo('signin')}>Back to sign in</button>}
      </div>
    </AuthCard>
  );
}

// Shown when this server requires a confirmed email before using the app.
export function ConfirmEmail({ notice }) {
  const { user, signOut, refresh } = useAuth();
  const [message, setMessage] = useState(notice);
  const [error, setError] = useState(null);
  async function resend() {
    try {
      await api('/auth/resend-verification', { method: 'POST' });
      setMessage(`A new link is on its way to ${user.email}.`);
      setError(null);
    } catch (err) { setError(err); }
  }
  return (
    <AuthCard>
      <h1>Confirm your email</h1>
      <p className="auth-sub">We sent a link to {user.email}. Open it to finish setting up your account.</p>
      <ErrorBanner error={error} />
      {message && <div className="notice" role="status">{message}</div>}
      <div className="auth-form">
        <button className="primary" onClick={refresh}>I have confirmed it</button>
        <button className="outline" onClick={resend}>Send a new link</button>
      </div>
      <button className="text-link auth-switch" onClick={signOut}>Sign out</button>
    </AuthCard>
  );
}
