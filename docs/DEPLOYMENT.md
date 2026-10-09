# Deploying DeviceTrustline

The production build is one Docker image (the root `Dockerfile`). It builds the web app and runs the API, which serves the app and the `/api` routes from the same origin. The only other thing it needs is PostgreSQL 14 or newer.

Pick one of the two options below.

## Option A: your own server, with automatic HTTPS

Use any Linux VM with Docker (for example 2 vCPU and 2 GB RAM on DigitalOcean, Hetzner, AWS Lightsail or Azure).

1. Point DNS for `devicetrustline.com` and `www.devicetrustline.com` (A records) at the server's IP address.
2. On the server:
   ```sh
   git clone https://github.com/Parthosd1/devictrustline.com.git && cd devictrustline.com
   cp deploy/.env.example deploy/.env
   # Fill in deploy/.env. Generate JWT_SECRET and POSTGRES_PASSWORD with: openssl rand -base64 48
   docker compose -f deploy/docker-compose.yml --env-file deploy/.env up -d --build
   ```
   Caddy gets the TLS certificate on first start. Ports 80 and 443 must be open.
3. Create your organization and first administrator:
   ```sh
   docker compose -f deploy/docker-compose.yml --env-file deploy/.env exec \
     -e ORG_NAME='Your Company' -e ADMIN_EMAIL=you@yourcompany.com -e ADMIN_NAME='Your Name' \
     -e ADMIN_PASSWORD='at least twelve characters' app npm run create-admin
   ```
4. Sign in at https://devicetrustline.com. Add your team under **Users**, your sites under **Buildings**, and load existing equipment with **Inventory → Import CSV**.

To update, run `git pull`, then run the same `docker compose … up -d --build` command. Database migrations run automatically on start.

**Backups.** The `backup` service writes a compressed `pg_dump` to `deploy/backups/` every night and keeps 14 days (`BACKUP_KEEP_DAYS`). Copy that folder off the server, for example with a nightly `rclone` or `aws s3 sync` cron job. A backup on the same disk does not protect against losing the server. To restore:
```sh
gunzip -c deploy/backups/devicetrustline-YYYYMMDD-HHMMSS.sql.gz | \
  docker compose -f deploy/docker-compose.yml --env-file deploy/.env exec -T db psql -U devicetrustline devicetrustline
```
Restore into an empty database. To get one, stop the stack, remove the `pgdata` volume, then start `db` alone before restoring.

## Option B: Render (managed hosting and managed PostgreSQL)

1. In Render, choose **New → Blueprint** and select this repository. `render.yaml` creates the web service and a PostgreSQL database, and generates `JWT_SECRET`.
2. Set `APP_ORIGIN` to the service URL, for example `https://devicetrustline.onrender.com`. After you add `devicetrustline.com` as a custom domain under **Settings → Custom Domains**, set it to `https://devicetrustline.com`. To accept both, separate them with commas.
3. Open the service **Shell** and run step 3 above as `ORG_NAME=… ADMIN_EMAIL=… ADMIN_PASSWORD=… npm run create-admin`.

Render's paid PostgreSQL plans include daily backups and point-in-time recovery. The same image also runs on Fly.io, Railway, Google Cloud Run, Azure Container Apps or AWS App Runner. On any of them, set the environment variables below and attach a managed PostgreSQL database.

## Environment variables

| Variable | Required | Notes |
|---|---|---|
| `DATABASE_URL` | yes | `postgres://user:password@host:5432/db` |
| `JWT_SECRET` | yes | At least 32 random characters. Changing it signs everyone out. |
| `APP_ORIGIN` | yes | The site's public URL, such as `https://devicetrustline.com`. Comma-separate multiple URLs. Browser requests from other origins are refused. |
| `TRUST_PROXY` | behind a proxy | `true` behind Caddy, a load balancer or a PaaS, so rate limits and secure cookies see the real client. |
| `DATABASE_SSL` | managed DBs | `true` to require TLS with a verified certificate. Use `no-verify` if the provider's certificate is self-signed. |
| `ALLOW_SIGNUP` | no | Defaults to `false` in production. Leave it off and use `create-admin`. |
| `SESSION_HOURS` | no | Defaults to 8. |
| `DISABLE_SCHEDULER` | no | `true` turns off automatic weekly and monthly audits on this instance. They are safe to leave on with several instances. |
| `LOG_REQUESTS` | no | One JSON line per request on stdout. On by default in production. |

## Operations

- **Health check:** `GET /api/health` returns 200 when the app can reach the database. Point your uptime monitor at it (UptimeRobot, Better Stack and similar tools all work). The Docker image and `render.yaml` already use it.
- **Logs:** the app writes request logs and errors to stdout. View them with `docker compose … logs -f app`, or in your host's log viewer.
- **Security:** sessions use httpOnly, SameSite cookies, which are `Secure` in production. Passwords are hashed with bcrypt. Sign-in is rate limited. Requests from other origins are refused. A strict Content-Security-Policy is set. Camera access is limited to this site. Every organization's data is isolated in the database.
- **Account recovery:** admins reset a user's password from **Users**, which signs that user out everywhere.

## Not included yet

Email (invitations, self-service password reset, scheduled report emails) needs an email provider account, such as Postmark, SendGrid or Amazon SES. Once one is chosen, add it as a follow-up. Until then, admins create users with a temporary password.
