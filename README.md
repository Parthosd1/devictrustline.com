# DeviceTrustline — Building & IT Asset Management

A multi-tenant web app for tracking laptops, desktops, scanners, monitors and attached workstation equipment across buildings.

## Quick start

```bash
docker compose up -d                       # PostgreSQL + API on :4000
npm install
npm run dev                                # frontend on :5173, proxies /api to the API
```

Open http://localhost:5173 and choose **Create a workspace**, or load the sample inventory first:

```bash
cd server && npm install
DATABASE_URL=postgres://devicetrustline:devicetrustline@localhost:5432/devicetrustline \
  SEED_ADMIN_EMAIL=you@example.com SEED_ADMIN_PASSWORD='a long password' npm run seed
```

To deploy to production (one Docker image, HTTPS, backups), follow [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md). The API is documented in [server/README.md](server/README.md).

## Features

- Sign-in, workspace sign-up, and five roles (admin, manager, technician, auditor, viewer)
- Dashboard with inventory KPIs and type breakdown
- Searchable, filterable inventory with CSV export
- Asset registration and editing with an automatic asset tag, plus full change history per asset
- People, plus assigning and checking in equipment (a workstation's components go with it)
- Parent-child assets: attach monitors, scanners and docks to a workstation; they move with it
- QR labels for any asset or a whole filtered list, printable on plain or adhesive paper
- Scan page: camera scanning (QR and common barcodes), handheld USB/Bluetooth scanners, or typed codes; matches asset tags and manufacturer serials
- Buildings and locations/stations management
- Maintenance work orders: priority, assignee, vendor, due date, cost, notes; assets go into and out of Maintenance automatically
- User management for admins
- Independent weekly and monthly audits: pick a scope, scan to verify, catch wrong-location and unexpected assets, close to a permanent record; optionally started automatically each week and month
- Reports: status, type, building, assignment, expiring warranties, maintenance cost, assets still missing after audits, with CSV downloads
- CSV import with a validated preview, all-or-nothing
- Settings: scheduled audits and password change

## Current limitations

Email (invitations, self-service password reset, emailed reports) needs an email provider and is not built yet. See [docs/ROADMAP.md](docs/ROADMAP.md).

Domain: devicetrustline.com (GitHub repository is spelled `devictrustline.com`).
