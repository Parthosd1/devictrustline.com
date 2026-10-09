import { Router } from 'express';
import { z } from 'zod';
import { query } from '../db/pool.js';
import { route } from '../lib/http.js';
import { requirePermission } from '../middleware/auth.js';

const DEFAULTS = { autoAudits: { weekly: false, monthly: false } };

const updateSchema = z.object({
  autoAudits: z.object({ weekly: z.boolean(), monthly: z.boolean() }).partial().strict(),
}).partial().strict();

export const withDefaults = (settings = {}) => ({
  ...DEFAULTS, ...settings, autoAudits: { ...DEFAULTS.autoAudits, ...settings.autoAudits },
});

export const settingsRouter = Router();

settingsRouter.get('/', route(async (req, res) => {
  const { rows: [org] } = await query('SELECT settings FROM organizations WHERE id = $1', [req.user.org_id]);
  res.json(withDefaults(org.settings));
}));

settingsRouter.patch('/', requirePermission('settings:manage'), route(async (req, res) => {
  const body = updateSchema.parse(req.body);
  const { rows: [org] } = await query('SELECT settings FROM organizations WHERE id = $1 FOR UPDATE', [req.user.org_id]);
  const current = withDefaults(org.settings);
  const next = { ...current, ...body, autoAudits: { ...current.autoAudits, ...body.autoAudits } };
  await query('UPDATE organizations SET settings = $2 WHERE id = $1', [req.user.org_id, JSON.stringify(next)]);
  res.json(next);
}));
