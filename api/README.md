# DeviceTrustline API

Rust (Axum, SQLx) + PostgreSQL. One binary serves the JSON API under `/api` and, when `STATIC_DIR` is set, the built web app. Migrations in `migrations/` are compiled into the binary and run automatically when the server starts.

## Run locally

```bash
# Option A: everything in Docker (Postgres + API on :4000)
docker compose up -d

# Option B: your own Postgres (install Rust from https://rustup.rs)
cd api
cp .env.example .env   # then edit DATABASE_URL and JWT_SECRET
set -a; . ./.env; set +a
cargo run -- migrate
SEED_ADMIN_EMAIL=you@example.com SEED_ADMIN_PASSWORD='a long password' cargo run -- seed   # optional demo data
cargo run
```

The Vite dev server proxies `/api` to `http://localhost:4000`.

## Commands

| Command | What it does |
|---|---|
| `devicetrustline` (or `serve`) | Runs migrations, background jobs (reminders, email delivery, scheduled audits) and the server |
| `devicetrustline migrate` | Applies pending migrations and exits |
| `devicetrustline create-admin` | Creates an organization and its first admin from `ORG_NAME`, `ADMIN_EMAIL`, `ADMIN_NAME`, `ADMIN_PASSWORD` |
| `devicetrustline seed` | Creates a demo workspace from `SEED_ADMIN_EMAIL` and `SEED_ADMIN_PASSWORD` |
| `devicetrustline run-scheduled-audits` | Opens any automatic audits that are due, once, and prints them as JSON |
| `devicetrustline run-jobs` | Runs one round of background jobs and prints what it did (for cron, or with `DISABLE_SCHEDULER=true`) |
| `devicetrustline plans` | Lists plans and their limits |
| `devicetrustline set-plan-limit PLAN LIMIT VALUE` | Changes a plan limit (`items`, `members`, `storageMb`, `devices`; `none` removes it) |
| `devicetrustline set-workspace-plan ADMIN_EMAIL PLAN` | Moves the workspaces this person administers (of the plan's kind) to that plan |
| `devicetrustline healthcheck` | Exits 0 when the local server answers `/api/health` (used by the Docker health check) |

With `cargo`, run them as `cargo run -- <command>`.

## Code layout

| Path | Contents |
|---|---|
| `src/main.rs` | Commands, server start-up and shutdown |
| `src/config.rs` | Environment variables (see `.env.example`) |
| `src/db.rs` | Connection pool, migrations, and `Q`, which returns rows as JSON so each SQL query is the shape of its response |
| `src/auth.rs` | Roles and permissions, password hashing (bcrypt), session tokens (HS256 JWT in an httpOnly cookie) |
| `src/validate.rs` | Request validation with field-level error details |
| `src/web.rs` | Security headers, cross-origin check, request log, sign-in rate limit, web app serving |
| `src/routes/` | One file per area: assets, audits, work orders, import, reports, people, sites, users, settings |
| `src/scheduler.rs` | Automatic weekly and monthly audits |

## Tests

```bash
cargo test                    # unit tests
./scripts/test-http.sh        # starts the server and runs the HTTP tests in tests/http (needs Node 22)
OLD_BIN=… ./scripts/test-deploy.sh   # fresh install, upgrade from the previous release, backup/restore
```

The HTTP tests run against a real, disposable PostgreSQL database that is wiped on every run:

```bash
createdb devicetrustline_test
TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/devicetrustline_test ./scripts/test-http.sh
```

They started as the black-box tests the earlier Node API passed and now cover release 1 as well, including Google sign-in against a local mock OpenID provider and the real device companion binary.

## Endpoints

| Method | Path | Permission |
|---|---|---|
| GET | `/api/health` | public |
| POST | `/api/auth/signup` | public, when `ALLOW_SIGNUP=true` (creates a workspace and its owner) |
| POST | `/api/auth/login`, `/api/auth/logout` | public |
| GET | `/api/auth/me` | signed in |
| POST | `/api/auth/change-password` | signed in |
| GET | `/api/users` | `users:read` |
| POST, PATCH | `/api/users`, `/api/users/:id` `{name?, role?, isActive?, password?, accessScope?: 'all'\|'inventories', inventoryIds?}` (only owners make or change owners) | `users:manage` |
| GET, POST, DELETE | `/api/invitations` `{email, role, accessScope?, inventoryIds?}`, `/api/invitations/:id` | `users:manage` (list: `users:read`) |
| GET, POST | `/api/access-requests?status=` (own requests, inventories one may ask for; deciders see all), `/api/access-requests` `{inventoryId, note?}` | signed in |
| POST | `/api/access-requests/:id/approve` `{note?}`, `/api/access-requests/:id/reject` `{note?}` | `access:approve` with access to everything, not one's own; `/cancel` by the requester |
| GET | `/api/buildings` (with locations and counts) | `sites:read` |
| POST, PATCH, DELETE | `/api/buildings[/:id]`, `/api/buildings/:id/locations`, `/api/locations/:id` | `sites:write` |
| GET | `/api/assets?q=&status=&type=&buildingId=&locationId=&personId=&parentId=&limit=&offset=` | `assets:read` |
| GET | `/api/assets/lookup?code=` (asset tag, label URL or serial) | `assets:read` |
| GET | `/api/assets/:id`, `/api/assets/:id/events` | `assets:read` |
| POST, PATCH | `/api/assets`, `/api/assets/:id` (`parentId` attaches it to another asset) | `assets:write` |
| POST | `/api/assets/:id/assign` `{personId, includeComponents?, note?}`, `/api/assets/:id/return` `{status?, includeComponents?, note?}` | `assets:write` |
| GET | `/api/people` | `assets:read` |
| POST, PATCH | `/api/people`, `/api/people/:id` | `people:write` |
| GET | `/api/audits?period=&status=`, `/api/audits/:id` (with items) | `audits:read` |
| POST | `/api/audits` `{period?, auditType?: soft\|full\|spot\|transfer\|return, buildingId?, locationId?, inventoryId?, assetIds?, requiresReview?, draft?, name?, dueAt?}`, `/api/audits/:id/begin`, `/scan` `{code, locationId?}`, `/submit` `{note?}`, `/reopen` `{note?}`, `/close` (audits without review) | `audits:perform` |
| POST | `/api/audits/:id/approve` `{note?}`, `/api/audits/:id/reject` `{note}` (not by whoever started or submitted it) | `audits:approve` |
| GET | `/api/audits/:id/export` (CSV) | `audits:read` |
| PATCH | `/api/audits/:id/items/:assetId` `{result, note?}` | `audits:perform` |
| GET | `/api/work-orders?status=&assetId=&assigneeId=`, `/api/work-orders/:id` (with notes), `/api/work-orders/assignees` | `maintenance:read` |
| POST, PATCH | `/api/work-orders`, `/api/work-orders/:id`, `/api/work-orders/:id/notes`, `/api/work-orders/:id/resolve`, `/api/work-orders/:id/cancel` | `maintenance:write` |
| POST | `/api/assets/import` `{rows, createSites?, dryRun?}` (up to 2,000 rows; nothing is saved unless every row is valid) | `assets:write` |
| GET | `/api/reports/summary` | `assets:read` |
| GET, PATCH | `/api/settings` `{autoAudits: {weekly, monthly}}` (PATCH is admin only) | `settings:manage` |
| GET | `/api/assets/export?…` (same filters as the list, CSV) | `assets:read` |
| POST | `/api/assets/:id/archive`, `/api/assets/:id/restore` (archived items leave lists, reminders and audits; `?archived=true\|all` lists them) | `assets:write` |
| GET, POST | `/api/reminders?status=&assetId=&kind=`, `/api/reminders` `{kind, title?, notes?, assetId?, dateSource?, dueOn?, offsets?, repeat?, repeatEvery?, repeatUntil?}` | signed in (own reminders only) |
| GET, PATCH, DELETE | `/api/reminders/:id` (GET includes each scheduled notification and its deliveries) | own reminder |
| POST | `/api/reminders/:id/snooze` `{hours? \| until?}`, `/api/reminders/:id/complete` | own reminder |
| GET, PATCH | `/api/notification-settings` `{enabled, email, push, sms, phone, timezone, quietStart, quietEnd, sendHour, defaultOffsets}` | signed in |
| POST | `/api/notification-settings/test`; `/api/push/subscriptions` (a browser's PushSubscription); DELETE `/api/push/subscriptions` `{endpoint?}` | signed in |
| DELETE | `/api/notifications/:id` | own notification |
| GET | `/api/inventories?archived=`, `/api/inventories/:id` (with sub-inventories and counts by status) | `inventory:read` |
| POST, PATCH | `/api/inventories` `{name, kind?, parentId?, description?, buildingId?, locationId?, custodianId?, requiresApproval?}`, `/api/inventories/:id`, `/api/inventories/:id/archive`, `/api/inventories/:id/restore` | `inventory:manage` |
| GET | `/api/transfers?status=&inventoryId=&assetId=&batchId=`, `/api/transfers/:id`; `/api/assets?inventoryId=<id\|none>&includeSub=true` | `inventory:read` |
| POST | `/api/transfers` `{assetIds \| codes, toInventoryId, fromInventoryId?, reason?, includeComponents?}`, `/api/transfers/:id/cancel` (requester or approver) | `inventory:transfer` |
| POST | `/api/transfers/:id/approve` `{note?}`, `/api/transfers/:id/reject` `{reason}` | `transfers:approve` |

| GET | `/api/loans?status=&overdue=&personId=&assetId=` (with `summary`), `/api/loans/export` (CSV), `/api/loans/:id` (with events) | `loans:read` |
| POST | `/api/loans` `{assetId \| code, personId, dueOn, startsOn?, purpose?, deployNow?, acknowledgedName?}`, `/api/loans/:id/deploy` `{acknowledgedName?, condition?, note?}`, `/extend` `{dueOn, reason}`, `/return` `{condition?, damaged?, note?}`, `/inspect` `{result: ready\|repair, note?}`, `/cancel` `{reason?}` | `loans:manage` |
| POST | `/api/loans/:id/lost` `{note}`; extending a loan a second time | `loans:resolve` |

People whose access is limited to some inventories (`accessScope: 'inventories'`) reach only items, transfers, work orders and inventories inside the granted inventories and their sub-inventories; anything else answers 404. Reports, insights, devices, audits and import are closed to them; catalog, sites, people and settings are read-only.

With `autoAudits` on, the API starts an organization-wide audit at the start of each week (Monday) or month, checked every 15 minutes. Set `DISABLE_SCHEDULER=true` to turn the scheduler off on extra instances; it is also safe to run on several instances at once.

Sign-in uses an httpOnly session cookie; API clients may send the same token as `Authorization: Bearer <token>`.
Login and sign-up are rate limited. State-changing requests from browser origins not listed in `APP_ORIGIN` are refused.
