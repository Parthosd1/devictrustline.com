import React, { useState } from 'react';
import { ScanLine } from 'lucide-react';
import { api } from '../api.js';
import { useAuth } from '../auth.jsx';
import { ErrorBanner } from '../components.jsx';

export function Login() {
  const { setUser } = useAuth();
  const [mode, setMode] = useState('signin');
  const [form, setForm] = useState({ organizationName: '', name: '', email: '', password: '' });
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const set = (key) => (e) => setForm({ ...form, [key]: e.target.value });

  async function submit(e) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const data = mode === 'signin'
        ? await api('/auth/login', { method: 'POST', body: { email: form.email, password: form.password } })
        : await api('/auth/signup', { method: 'POST', body: form });
      setUser(data.user);
    } catch (err) {
      setError(err);
      setBusy(false);
    }
  }

  const signup = mode === 'signup';
  return (
    <div className="auth-page">
      <div className="auth-card">
        <div className="brand auth-brand">
          <div className="brand-icon"><ScanLine size={22} /></div>
          <div><strong>DeviceTrustline</strong><small>ASSET INTELLIGENCE</small></div>
        </div>
        <h1>{signup ? 'Create your workspace' : 'Sign in'}</h1>
        <p className="auth-sub">
          {signup ? 'Set up your organization. You will be its first admin.' : 'Track every device, location and audit in one place.'}
        </p>
        <ErrorBanner error={error} />
        <form onSubmit={submit} className="auth-form">
          {signup && (
            <>
              <label>Organization name<input required value={form.organizationName} onChange={set('organizationName')} autoComplete="organization" /></label>
              <label>Your name<input required value={form.name} onChange={set('name')} autoComplete="name" /></label>
            </>
          )}
          <label>Work email<input required type="email" value={form.email} onChange={set('email')} autoComplete="email" /></label>
          <label>
            Password
            <input required type="password" minLength={signup ? 12 : undefined} value={form.password} onChange={set('password')}
              autoComplete={signup ? 'new-password' : 'current-password'} />
            {signup && <small>At least 12 characters.</small>}
          </label>
          <button className="primary" disabled={busy}>{busy ? 'Please wait…' : signup ? 'Create workspace' : 'Sign in'}</button>
        </form>
        <button className="text-link auth-switch" onClick={() => { setMode(signup ? 'signin' : 'signup'); setError(null); }}>
          {signup ? 'Already have an account? Sign in' : 'New to DeviceTrustline? Create a workspace'}
        </button>
      </div>
    </div>
  );
}
