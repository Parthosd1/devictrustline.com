import { createApp } from './app.js';
import { config } from './config.js';
import { migrate } from './db/migrate.js';
import { pool } from './db/pool.js';
import { startScheduler } from './scheduler.js';

await migrate();
if (process.env.DISABLE_SCHEDULER !== 'true') startScheduler();
const server = createApp().listen(config.port, () => {
  console.log(`DeviceTrustline API listening on :${config.port}`);
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => server.close(() => pool.end().then(() => process.exit(0))));
}
