# Security and production review (Phase 11)

Each item from the production spec, how it is handled, and how it is verified. "Tested" means an
automated test exercises it in CI. The line is marked when a check is only by review or by hand.

| Item | How it is handled | Verified by |
|---|---|---|
| PostgreSQL migrations | 17 ordered SQL migrations embedded in the binary (`api/src/db.rs`). They run in a transaction under an advisory lock, each one once, at start-up or with `devicetrustline migrate`. | CI `deploy` job (`api/scripts/test-deploy.sh`): fresh install, then an upgrade from the release-before-workspaces schema (005) with data, then backup and restore. The HTTP suite migrates an empty database on every run. |
| API validation | Every body and query goes through `validate::Input`. Unknown fields are refused (`strict`), and types, lengths, enums, dates and UUIDs are checked. Errors come back as 400 with a field path. | HTTP tests throughout, such as bad time zones, quiet hours, offsets, plan limits, unknown notification groups and damaged items without a note. |
| Role and permission enforcement | Permissions are granted to roles in `api/src/auth.rs` (`GRANTS`) and checked on the server in every handler with `user.require(...)`. Owners alone manage owners, and a workspace always keeps one owner. | Unit test `roles_get_exactly_their_grants`, plus `rbac.test.js`, `delegated-access.test.js`, and workflows 3 and 8–12 in `critical-workflows.test.js`. |
| Delegated (inventory-scoped) access | The `src/scope.rs` middleware resolves every item, transfer, loan, audit, work-order, attachment and inventory id against the person's inventories. Anything outside answers 404. Lists filter in SQL, and notices about an item reach only people who can see it (`member_sees_asset`). | `delegated-access.test.js`, `business-notifications.test.js`, workflow 3. |
| Workspace isolation | Every table carries `org_id`. Composite foreign keys stop rows referencing another workspace's rows, and every query filters by the signed-in workspace. | Workflow 13 (another workspace, a personal account and a signed-out request all fail), `accounts.test.js`, and isolation checks in most suites. |
| Rate limiting | Sign-in, sign-up, password reset and invitation acceptance are limited per client address (`AUTH_RATE_LIMIT`, default 20 per 15 minutes) and answer 429 with `Retry-After`. `TRUST_PROXY` decides whether `X-Forwarded-For` is believed. | Unit tests `sign_in_attempts_are_limited_per_address` and `the_window_resets` (new in this phase). The HTTP suite raises the limit so that it can sign in hundreds of times. |
| Secure file uploads | The content type comes from the file's own bytes (JPEG, PNG, GIF, WebP, HEIC, PDF only), never from its name or the client. Files are stored outside the web root by id, the size is capped (`MAX_UPLOAD_MB`), there is a storage quota per plan, and downloads are sent with `Content-Disposition` and `nosniff`. | Unit test `file_types_come_from_content`; `registry.test.js` refuses an HTML file named `.jpg` (415). |
| Audit logs | `asset_events`, `loan_events`, `audit_reviews` and decided transfers are append-only, and database triggers refuse UPDATE and DELETE. Approved or closed audits are locked by a trigger. | Workflow 12 (the database refuses changes to a locked audit), `audit-workflow.test.js`, `loaners.test.js`, `inventories.test.js`. |
| CSRF protection | Session cookies are `HttpOnly` and `SameSite=Lax` (`Secure` in production), and bodies must be JSON. Cross-origin writes are refused by an Origin check (`web::check_origin`) unless the origin is listed in `APP_ORIGIN`. | `auth.test.js` "cross-origin writes are blocked". |
| Authentication security | Passwords are hashed with bcrypt and must be at least 12 characters. Sessions are signed with `JWT_SECRET`, which must be 32+ characters in production. Changing the password signs out other sessions. Reset and invitation tokens are single-use and stored hashed. Email verification is on when email is set up. Google sign-in uses OIDC with state and nonce. | `auth.test.js` and `accounts.test.js`; Google sign-in is tested end to end against a mock OIDC provider. |
| Security headers | A strict Content-Security-Policy, COOP and CORP same-origin, `nosniff`, `Referrer-Policy`, and camera access limited to this site. | By review (`web::security_headers`). |
| Background-job reliability | Notifications, email, push and SMS deliveries are rows in the database with attempts, next-attempt time and errors. Workers claim them with `FOR UPDATE SKIP LOCKED`, so several instances are safe, and back off on failure. Overdue-loan and reminder jobs are idempotent per due date. | `personal-reminders.test.js` (retries, quiet hours, expired push subscriptions dropped), and workflow 6 (the job runs three times and sends one notice). |
| Concurrency | One open loan per item and one pending transfer per item are enforced by unique indexes. Transfers with an expected source lock and re-check the item. Plan limits lock the workspace row. | Workflow 14: 6 simultaneous loans for one item give one 201 and five 409s; 6 simultaneous transfers move it once; only one pending transfer is ever recorded. |
| Database backup and restore | The compose stack runs a nightly `pg_dump` and copies uploaded files, keeping `BACKUP_KEEP_DAYS` days. Restore steps are in `docs/DEPLOYMENT.md`. | CI `deploy` job restores a backup into a fresh database and checks the data. |
| Health checks | `GET /api/health` checks the database, and the `devicetrustline healthcheck` command is used by the Docker `HEALTHCHECK`. | `test-deploy.sh` and the Docker image's healthcheck. |
| Environment configuration | `api/.env.example` (local) and `deploy/.env.example` (production) list every variable. The server refuses to start in production with a short `JWT_SECRET`, `MAIL_MODE=capture` or `SMS_PROVIDER=log`. | `config.rs` checks; by review. |
| Docker deployment | Multi-stage `Dockerfile` (web build, API build, slim runtime that runs as a non-root user with a healthcheck), `deploy/docker-compose.yml` (Caddy HTTPS, app, Postgres, backups), and `render.yaml`. | CI `docker` job builds both images. It was not built in this sandbox, where Docker Hub rate-limits pulls. |
| CI test execution | Jobs: `frontend` (build), `api` (fmt, clippy, unit tests, HTTP suite), `deploy`, `agent` (Linux and Windows builds), `docker`, and `e2e` (new). | GitHub Actions on every PR. |
| Frontend production build | `npm run build` (Vite). | CI `frontend` job; run locally for every phase. |
| Rust formatting, linting, tests | `cargo fmt --check` and `cargo clippy -D warnings` for the API and the agent, plus unit tests. | CI `api` and `agent` jobs; run locally for every phase. |
| API integration tests | 25 files of black-box HTTP tests against a real server and Postgres (`api/scripts/test-http.sh`). | CI `api` job. |
| End-to-end critical workflows | `api/tests/http/critical-workflows.test.js` has the 14 workflows from the spec. `e2e/walkthrough.mjs` drives the real web app in Chromium at 1366px and 390px. | CI `api` and `e2e` jobs. |

## Findings fixed during this phase

- **One-off spot audits.** The start form let you submit a spot audit with no inventory, and the API refused it with a 400. The form now requires an inventory for spot checks. This was found by the browser walk-through.
- **Rate limiting had no automated test.** Two unit tests were added.

## Known limits (not defects, but worth knowing)

- **Rate limiting is per instance, in memory.** With several API instances, each one keeps its own count. Put the limit at the proxy (Caddy or Render) as well if you scale out.
- **The Windows companion `.exe` is not code-signed,** so SmartScreen warns on first run.
- **Typed scans in an audit are de-duplicated for a short time.** This avoids double reads from a camera, so a deliberate second scan typed straight away is ignored. Duplicate scans from the API are counted.
