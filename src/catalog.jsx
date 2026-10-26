import React, { createContext, useCallback, useContext, useEffect, useState } from 'react';
import { api } from './api.js';

// The workspace's categories, tags and custom fields, shared by forms and filters.
const CatalogContext = createContext(null);

export function CatalogProvider({ children }) {
  const [catalog, setCatalog] = useState({ categories: [], tags: [], fields: [], loaded: false });
  const reload = useCallback(async () => {
    try {
      const [c, t, f] = await Promise.all([api('/categories'), api('/tags'), api('/custom-fields')]);
      setCatalog({ categories: c.items, tags: t.items, fields: f.items, loaded: true });
    } catch {
      setCatalog((x) => ({ ...x, loaded: true }));
    }
  }, []);
  useEffect(() => { reload(); }, [reload]);
  return <CatalogContext.Provider value={{ ...catalog, reload }}>{children}</CatalogContext.Provider>;
}

export const useCatalog = () => useContext(CatalogContext);

// "Items" in a personal workspace, "Assets" in a business one.
export const nouns = (kind) => (kind === 'personal'
  ? { one: 'item', many: 'items', One: 'Item', Many: 'Items', places: 'Places', place: 'Place', spot: 'Spot' }
  : { one: 'asset', many: 'assets', One: 'Asset', Many: 'Assets', places: 'Buildings', place: 'Building', spot: 'Location / Station' });
