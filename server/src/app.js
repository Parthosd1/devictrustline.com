import cookieParser from 'cookie-parser';
import express from 'express';
import helmet from 'helmet';
import { config } from './config.js';
import { query } from './db/pool.js';
import { authenticate, checkOrigin } from './middleware/auth.js';
import { errorHandler } from './middleware/errors.js';
import { assetsRouter } from './routes/assets.js';
import { authRouter } from './routes/auth.js';
import { peopleRouter } from './routes/people.js';
import { sitesRouter } from './routes/sites.js';
import { usersRouter } from './routes/users.js';

export function createApp() {
  const app = express();
  app.disable('x-powered-by');
  if (config.trustProxy) app.set('trust proxy', 1);

  app.use(helmet());
  app.use(express.json({ limit: '100kb' }));
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
  app.use('/api/assets', authenticate, assetsRouter);
  app.use('/api/people', authenticate, peopleRouter);
  app.use('/api', authenticate, sitesRouter);

  app.use('/api', (_req, res) => res.status(404).json({ error: 'Not found' }));
  app.use(errorHandler);
  return app;
}
