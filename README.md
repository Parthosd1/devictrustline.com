# DeviceTrustline — Building & IT Asset Management MVP

A responsive React + Vite demonstration for tracking laptops, desktops, scanners, monitors and attached workstation equipment across buildings.

## Quick start

```bash
npm install
npm run dev
```

Build for production with `npm run build`.

## Demo features

- Dashboard with inventory and audit KPIs
- Searchable asset inventory and new asset registration
- Building overview and attached equipment counts
- Weekly and monthly audit screens with manual verification
- Maintenance issue tracking
- CSV asset export
- Browser-local persistence using `localStorage`

## Important limitations

This is a **frontend-only prototype** with fictional sample inventory. It does not have a backend, authentication, multi-user synchronization, real barcode scanning, server-side scheduling, or production-grade audit controls. Audit verification is a demo interaction and does not provide an immutable audit trail. Do not use real organizational or employer inventory until access control and secure storage are implemented.

## Next phase for Claude Code

Add PostgreSQL, server-side organization isolation, authentication, asset relationships, independent weekly/monthly audit runs, scan verification, immutable event history, CSV import, reports and scheduled notifications. Use pull requests for changes.

Domain: devicetrustline.com (GitHub repository is spelled `devictrustline.com`).
