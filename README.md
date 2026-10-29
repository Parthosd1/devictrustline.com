# DeviceTrustline

Track what you own and what your organization owns: items with their receipts, photos and warranties, reminders before warranties end or return windows close, and, for businesses, buildings, assignments, maintenance and audits.

Each account can belong to several workspaces. A **personal** workspace is for your own belongings; a **business** workspace adds buildings, employees, audits and reports.

## Languages and layout

| Part | Language | Path |
|---|---|---|
| Web app | JavaScript (React 18 + Vite), CSS | `src/`, `style.css` |
| API and background jobs | Rust (Axum, SQLx, Tokio) | `api/` |
| Database | PostgreSQL, plain SQL migrations | `api/migrations/` |
| Device companion for Windows | Rust (reads hardware through PowerShell/CIM) | `agent/` |
| HTTP tests | JavaScript (Node test runner + supertest) against the real Rust binary | `api/tests/http/` |
| Deployment | Docker, Docker Compose, Caddy, Render blueprint, Bash | `Dockerfile`, `deploy/`, `render.yaml` |

## Quick start

```bash
docker compose up -d                       # PostgreSQL, the API on :4000, Mailpit on :8025
npm install
npm run dev                                # web app on :5173, proxies /api to the API
```

Open http://localhost:5173 and create an account (emails land in Mailpit at http://localhost:8025), or load the sample business workspace:

```bash
docker compose exec -e SEED_ADMIN_EMAIL=you@example.com -e SEED_ADMIN_PASSWORD='a long password' api devicetrustline seed
```

Production deployment: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md). API: [api/README.md](api/README.md). Device companion: [agent/README.md](agent/README.md). Release status: [PROGRESS.md](PROGRESS.md).

## Features

**Accounts and workspaces**
- Email and password sign-up (controlled in production), email confirmation, self-service password reset
- Google sign-in; linking Google to an existing password account requires signing in to that account first
- Personal and business workspaces, a workspace switcher, invitations by email or copyable link, five roles

**Registry**
- Items with category, condition, owner, price and currency, seller, return deadline, maintenance date, tags and your own custom fields
- Photos, receipts, manuals and other documents per item, with storage limits per plan
- Several warranties per item (manufacturer, extended, store, insurance) with states: active, ending soon, expired, unknown
- Full, append-only change history per item

**Reminders and insights**
- Reminders before warranties end, return windows close, maintenance is due, loans are due back and audits are due, in the workspace time zone, in the app and by email when email is set up
- Dashboard deadlines and suggestions (idle items, warranties ending, frequent repairs, overdue loans), each with its reason; nothing is sold or listed automatically

**Business operations**
- Buildings and locations, people, assigning and checking in equipment, parent and child assets
- QR labels and scanning (camera, USB/Bluetooth scanners), weekly and monthly audits
- Maintenance work orders, reports with CSV downloads, CSV import with preview

**Device companion (Windows)**
- Pairs with a one-time code, reports the computer and attached hardware (monitors, USB devices, printers) for review
- Only real serial numbers are recorded; devices are never merged on model name alone; nothing is added to the inventory without review

**Plans**
- Free Personal, Personal Pro, Business and Enterprise. Limits (items, members, storage, paired computers, inventories, active reminders) and features (text messages, reports) are enforced by the API. No payment provider is connected yet; the operator changes plans
