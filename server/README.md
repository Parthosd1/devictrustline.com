# DeviceTrustline API

Node 22 + Express + PostgreSQL. Migrations in `migrations/` run automatically when the server starts.

## Run locally

```bash
# Option A: everything in Docker (Postgres + API on :4000)
docker compose up -d

# Option B: your own Postgres
cd server
cp .env.example .env   # then edit DATABASE_URL and JWT_SECRET
npm install
npm run migrate
SEED_ADMIN_EMAIL=you@example.com SEED_ADMIN_PASSWORD='a long password' npm run seed   # optional demo data
node --env-file=.env src/index.js
```

The Vite dev server proxies `/api` to `http://localhost:4000`.

## Tests

Tests run against a real, disposable PostgreSQL database (it is wiped on every run):

```bash
createdb devicetrustline_test
TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/devicetrustline_test npm test
```

## Endpoints

| Method | Path | Permission |
|---|---|---|
| GET | `/api/health` | public |
| POST | `/api/auth/signup` | public, when `ALLOW_SIGNUP=true` (creates a workspace and its admin) |
| POST | `/api/auth/login`, `/api/auth/logout` | public |
| GET | `/api/auth/me` | signed in |
| POST | `/api/auth/change-password` | signed in |
| GET | `/api/users` | `users:read` |
| POST, PATCH | `/api/users`, `/api/users/:id` | `users:manage` |
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
| POST | `/api/audits` `{period, buildingId?, locationId?}`, `/api/audits/:id/scan` `{code, locationId?}`, `/api/audits/:id/close` | `audits:perform` |
| PATCH | `/api/audits/:id/items/:assetId` `{result, note?}` | `audits:perform` |

Sign-in uses an httpOnly session cookie; API clients may send the same token as `Authorization: Bearer <token>`.
Login and sign-up are rate limited. State-changing requests from browser origins not listed in `APP_ORIGIN` are refused.
