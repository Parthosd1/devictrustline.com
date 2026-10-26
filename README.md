# DeviceTrustline — Building & IT Asset Management MVP

A responsive React + Vite demonstration for tracking laptops, desktops, scanners, monitors and attached workstation equipment across buildings.

## Quick start

```bash
npm install
npm run dev
```

Build for production with `npm run build`.

The backend lives in [`server/`](server/README.md): `docker compose up -d` starts PostgreSQL and the API.
See [docs/ROADMAP.md](docs/ROADMAP.md) for the phased plan.

## Demo features

- Dashboard with inventory and audit KPIs
- Searchable asset inventory and new asset registration
- Building overview and attached equipment counts
- Weekly and monthly audit screens with manual verification
- Maintenance issue tracking
- CSV asset export
- Browser-local persistence using `localStorage`

## Important limitations

The React app is still a **frontend-only prototype** with fictional sample inventory until phase 2 connects it to the new API. Until then it stores data in this browser only, has no sign-in or real barcode scanning, and its audit verification is a demo interaction, not an immutable audit trail. Do not enter real organizational or employer inventory into the demo.

## Roadmap

Phase 1 adds the API foundation: PostgreSQL, authentication, roles, organization isolation and append-only asset history. Asset relationships, assignments, scanning, independent weekly/monthly audit runs, maintenance, reports and deployment follow in later phases ([docs/ROADMAP.md](docs/ROADMAP.md)). Changes go through pull requests.

Domain: devicetrustline.com (GitHub repository is spelled `devictrustline.com`).
