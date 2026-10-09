import React, { createContext, useCallback, useContext, useEffect, useState } from 'react';
import { api } from './api.js';

const AuthContext = createContext(null);

export function AuthProvider({ children }) {
  const [user, setUser] = useState(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api('/auth/me')
      .then((data) => setUser(data.user))
      .catch(() => setUser(null))
      .finally(() => setLoading(false));
  }, []);

  const signOut = useCallback(async () => {
    await api('/auth/logout', { method: 'POST' }).catch(() => {});
    setUser(null);
  }, []);

  const can = useCallback((permission) => !!user?.permissions.includes(permission), [user]);

  return <AuthContext.Provider value={{ user, setUser, loading, signOut, can }}>{children}</AuthContext.Provider>;
}

export const useAuth = () => useContext(AuthContext);
