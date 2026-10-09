import path from 'node:path';
import cookieParser from 'cookie-parser';
import express from 'express';
import helmet from 'helmet';
import { config } from './config.js';
import { query } from './db/pool.js';
import { authenticate, checkOrigin } from './middleware/auth.js';
import { errorHandler } from './middleware/errors.js';
import { assetsRouter } from './routes/assets.js';
import { auditsRouter } from './routes/audits.js';
import { authRouter } from './routes/auth.js';
import { importRouter } from './routes/importAssets.js';
import { peopleRouter } from './routes/people.js';
import { reportsRouter } from './routes/reports.js';
import { settingsRouter } from './routes/settings.js';
import { sitesRouter } from './routes/sites.js';
import { usersRouter } from './routes/users.js';
import { workOrdersRouter } from './routes/workOrders.js';

export function createApp({ staticDir = config.staticDir } = {}) {
  const app = express();
  app.disable('x-powered-by');
  if (config.trustProxy) app.set('trust proxy', 1);

  app.use(helmet({
    contentSecurityPolicy: {
      directives: {
        defaultSrc: ["'self'"],
        scriptSrc: ["'self'"],
        styleSrc: ["'self'", "'unsafe-inline'", 'https://fonts.googleapis.com'],
        imgSrc: ["'self'", 'data:', 'blob:'],
        mediaSrc: ["'self'", 'blob:'],
        connectSrc: ["'self'"],
        fontSrc: ["'self'", 'data:', 'https://fonts.gstatic.com'],
        objectSrc: ["'none'"],
        frameAncestors: ["'none'"],
        baseUri: ["'self'"],
        formAction: ["'self'"],
        upgradeInsecureRequests: config.isProduction ? [] : null,
      },
    },
  }));
  // The scan pages use the camera; nothing else needs device access.
  app.use((_req, res, next) => {
    res.set('Permissions-Policy', 'camera=(self), microphone=(), geolocation=()');
    next();
  });
  if (config.logRequests) app.use(requestLog);
  app.use(express.json({ limit: '2mb' }));
  app.use(cookieParser());
  app.use('/api', checkOrigin(config.appOrigins));

  app.get('/api/health', async (_req, res) => {
    try {
      await query('SELECT 1');
      res.json({ status: 'ok' });
    } catch {
      res.status(503).json({ status: 'database unavailable' });
    }
  });

  app.use('/api/auth', authRouter);
  app.use('/api/users', authenticate, usersRouter);
  app.use('/api/assets/import', authenticate, importRouter);
  app.use('/api/assets', authenticate, assetsRouter);
  app.use('/api/people', authenticate, peopleRouter);
  app.use('/api/audits', authenticate, auditsRouter);
  app.use('/api/work-orders', authenticate, workOrdersRouter);
  app.use('/api/reports', authenticate, reportsRouter);
  app.use('/api/settings', authenticate, settingsRouter);
  app.use('/api', authenticate, sitesRouter);

  app.use('/api', (_req, res) => res.status(404).json({ error: 'Not found' }));
  if (staticDir) serveWebApp(app, staticDir);
  app.use(errorHandler);
  return app;
}

// Serves the built web app. Hashed files under /assets are cached for a year;
// every other path falls back to index.html, which is never cached.
function serveWebApp(app, dir) {
  const root = path.resolve(dir);
  app.use('/assets', express.static(path.join(root, 'assets'), { immutable: true, maxAge: '1y', fallthrough: false }));
  app.use(express.static(root, { index: false, maxAge: '1h' }));
  app.get('*', (_req, res) => {
    res.set('Cache-Control', 'no-cache');
    res.sendFile(path.join(root, 'index.html'));
  });
}

function requestLog(req, res, next) {
  const start = process.hrtime.bigint();
  res.on('finish', () => {
    if (req.originalUrl === '/api/health') return;
    console.log(JSON.stringify({
      t: new Date().toISOString(), method: req.method, path: req.originalUrl.split('?')[0], status: res.statusCode,
      ms: Number((process.hrtime.bigint() - start) / 1000000n), user: req.user?.id,
    }));
  });
  next();
}
