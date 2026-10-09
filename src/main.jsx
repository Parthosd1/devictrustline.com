import React from 'react';
import { createRoot } from 'react-dom/client';
import { ScanLine } from 'lucide-react';
import { App } from './App.jsx';
import { AuthProvider, useAuth } from './auth.jsx';
import { Login } from './pages/Login.jsx';
import '../style.css';

function Root() {
  const { user, loading } = useAuth();
  if (loading) {
    return <div className="splash"><div className="brand-icon"><ScanLine size={22} /></div></div>;
  }
  return user ? <App /> : <Login />;
}

createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <AuthProvider><Root /></AuthProvider>
  </React.StrictMode>,
);
