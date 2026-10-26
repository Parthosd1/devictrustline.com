import React, { createContext, useCallback, useContext, useEffect, useState } from 'react';
import { api } from './api.js';

const AuthContext = createContext(null);

export function AuthProvider({ children }) {
  const [user, setUser] = useState(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(() => api('/auth/me')
    .then((data) => setUser(data.user))
    .catch(() => setUser(null))
    .finally(() => setLoading(false)), []);

  useEffect(() => { refresh(); }, [refresh]);

  const signOut = useCallback(async () => {
    await api('/auth/logout', { method: 'POST' }).catch(() => {});
    setUser(null);
  }, []);

  const switchWorkspace = useCallback(async (workspaceId) => {
    const r = await api('/auth/switch', { method: 'POST', body: { workspaceId } });
    setUser(r.user);
  }, []);

  const can = useCallback((permission) => !!user?.permissions.includes(permission), [user]);

  return (
    <AuthContext.Provider value={{ user, setUser, loading, signOut, can, refresh, switchWorkspace }}>{children}</AuthContext.Provider>
  );
}

export const useAuth = () => useContext(AuthContext);
