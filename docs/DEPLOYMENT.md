# Deploying DeviceTrustline

The production build is one Docker image (the root `Dockerfile`). It builds the React web app, the Rust API and the Rust Windows device companion. The API binary serves the app, the `/api` routes and the companion download (`/downloads/devicetrustline-agent.exe`) from the same origin, and runs background jobs (reminders, email delivery, scheduled audits) in the same process. It needs PostgreSQL 14 or newer and a persistent directory for uploaded files (`/data` in the image).

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
     -e ADMIN_PASSWORD='at least twelve characters' app devicetrustline create-admin
   ```
4. Sign in at https://devicetrustline.com. Add your team under **Users**, your sites under **Buildings**, and load existing equipment with **Inventory → Import CSV**.

To update, run `git pull`, then run the same `docker compose … up -d --build` command. Database migrations run automatically on start.

**Backups.** The `backup` service writes a compressed `pg_dump` (`devicetrustline-*.sql.gz`) and a tar of uploaded photos and documents (`files-*.tar.gz`) to `deploy/backups/` every night, and keeps 14 days (`BACKUP_KEEP_DAYS`). Copy that folder off the server, for example with a nightly `rclone` or `aws s3 sync` cron job. A backup on the same disk does not protect against losing the server. To restore, into an empty database (stop the stack, remove the `pgdata` volume, start `db` alone):
```sh
C="docker compose -f deploy/docker-compose.yml --env-file deploy/.env"
gunzip -c deploy/backups/devicetrustline-YYYYMMDD-HHMMSS.sql.gz | $C exec -T db psql -U devicetrustline devicetrustline
$C run --rm -v "$PWD/deploy/backups:/backups:ro" --entrypoint sh backup -c 'cd /data && tar -xzf /backups/files-YYYYMMDD-HHMMSS.tar.gz'
$C up -d
```
The backup container mounts the files volume read-only, so for the second command change `attachments:/data:ro` to `attachments:/data` in `deploy/docker-compose.yml` first, and change it back afterwards. `api/scripts/test-deploy.sh` checks this round trip (database plus files) in CI, along with a fresh install and an upgrade from the previous release's schema with data.

## Option B: Render (managed hosting and managed PostgreSQL)

1. In Render, choose **New → Blueprint** and select this repository. `render.yaml` creates the web service and a PostgreSQL database, and generates `JWT_SECRET`.
2. Set `APP_ORIGIN` to the service URL, for example `https://devicetrustline.onrender.com`. After you add `devicetrustline.com` as a custom domain under **Settings → Custom Domains**, set it to `https://devicetrustline.com`. To accept both, separate them with commas.
3. Set `APP_URL` to the same public URL (links in emails use it), and fill in `SMTP_URL`, `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` if you use them (see below).
4. Open the service **Shell** and run step 3 above as `ORG_NAME=… ADMIN_EMAIL=… ADMIN_PASSWORD=… devicetrustline create-admin`.

`render.yaml` attaches a 5 GB persistent disk at `/data` for uploaded files. Render disks need a paid instance, keep the service to one instance, and are snapshotted daily by Render. Render's paid PostgreSQL plans include daily backups and point-in-time recovery. The same image also runs on Fly.io, Railway, Google Cloud Run, Azure Container Apps or AWS App Runner. On any of them, set the environment variables below and attach a managed PostgreSQL database.

## Environment variables

| Variable | Required | Notes |
|---|---|---|
| `DATABASE_URL` | yes | `postgres://user:password@host:5432/db` |
| `JWT_SECRET` | yes | At least 32 random characters. Changing it signs everyone out. |
| `APP_ORIGIN` | yes | The site's public URL, such as `https://devicetrustline.com`. Comma-separate multiple URLs. Browser requests from other origins are refused. |
| `TRUST_PROXY` | behind a proxy | `true` behind Caddy, a load balancer or a PaaS, so rate limits and secure cookies see the real client. |
| `DATABASE_SSL` | managed DBs | `true` to require TLS with a verified certificate. Use `no-verify` if the provider's certificate is self-signed. |
| `APP_URL` | yes | The public URL used in email links and the Google callback. Defaults to the first `APP_ORIGIN`. |
| `ALLOW_SIGNUP` | no | Defaults to `false` in production, which keeps sign-up controlled: people join by invitation, or an operator runs `create-admin`. |
| `ATTACHMENTS_DIR` | yes | Where uploaded files are stored. `/data/attachments` in the image; mount a persistent volume at `/data`. |
| `MAX_UPLOAD_MB` | no | Largest upload, default 10. Plans also limit total storage per workspace. |
| `SMTP_URL` | for email | `smtps://user:pass@host:465` or `smtp://user:pass@host:587?tls=required`. Any provider works (Postmark, SendGrid, Amazon SES, Mailgun). Without it nothing is emailed: invitations show a link to copy, and the app never reports an email as sent. |
| `MAIL_FROM` | no | Sender, default `DeviceTrustline <no-reply@devicetrustline.com>`. The provider must allow this domain (SPF/DKIM). |
| `REQUIRE_EMAIL_VERIFICATION` | no | New accounts must confirm their email before using the app. Defaults to on when `SMTP_URL` is set. |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | for Google sign-in | From a Google Cloud OAuth client of type "Web application". Authorized redirect URI: `APP_URL` + `/api/auth/google/callback`, for example `https://devicetrustline.com/api/auth/google/callback`. The secret stays on the server. |
| `VAPID_PRIVATE_KEY`, `VAPID_SUBJECT` | for browser push | Keys that identify this server to browser push services. Generate once with `devicetrustline vapid-keys` and keep them: changing them signs every browser out of push. `VAPID_SUBJECT` is a `mailto:` or `https://` contact. `VAPID_PUBLIC_KEY` is optional and only checked against the private key. Without these, push is shown as not set up. |
| `SMS_PROVIDER` | for text messages | `disabled` (default) or `twilio` with `TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN` and `TWILIO_FROM` (a Twilio number in +E.164 form). `log` records messages without sending and is refused in production. |
| `SESSION_HOURS` | no | Defaults to 8. |
| `DISABLE_SCHEDULER` | no | `true` turns off background jobs on this instance (reminders, email, push and SMS delivery, automatic audits). They are safe to leave on with several instances; to run them elsewhere, call `devicetrustline run-jobs` from cron. |
| `LOG_REQUESTS` | no | One JSON line per request on stdout. On by default in production. |

## Operations

- **Health check:** `GET /api/health` returns 200 when the app can reach the database. Point your uptime monitor at it (UptimeRobot, Better Stack and similar tools all work). The Docker image and `render.yaml` already use it.
- **Logs:** the app writes request logs and errors to stdout. View them with `docker compose … logs -f app`, or in your host's log viewer.
- **Security:** sessions use httpOnly, SameSite cookies, which are `Secure` in production. Passwords are hashed with bcrypt. Sign-in is rate limited. Requests from other origins are refused. A strict Content-Security-Policy is set. Camera access is limited to this site. Every organization's data is isolated in the database.
- **Account recovery:** with email set up, people reset their own password from the sign-in page. Without email there is no self-service reset (an account can belong to several workspaces, so workspace admins cannot set its password), which is one reason to set up `SMTP_URL` before going live.
- **Plans:** limits per plan live in the database (`devicetrustline plans`, `set-plan-limit`, `set-workspace-plan`). No payment provider is connected, so the in-app upgrade button explains that and the operator changes plans with `set-workspace-plan`.
- **Device companion:** the Windows `.exe` is built into the image. It is not code-signed yet, so Windows SmartScreen warns on first run; sign it with your certificate before wide distribution.

## Before going live

1. Choose hosting (Option A or B) and point DNS at it.
2. Create an SMTP account and set `SMTP_URL` and `MAIL_FROM`, with SPF and DKIM for the sending domain.
3. Optionally create a Google OAuth client and set `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`.
4. Run `create-admin`, sign in, and check `/api/health`.
