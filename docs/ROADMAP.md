# DeviceTrustline roadmap

DeviceTrustline grows from the frontend MVP into a multi-tenant SaaS for IT equipment and building assets.
Each phase ships as its own pull request (or a short series), keeps the existing app working, and has tests in CI.

| Phase | Scope | Status |
|---|---|---|
| 1. Foundation | Node/Express API, PostgreSQL schema and migrations, sign-up/sign-in, five roles, organization isolation, buildings and locations, asset registry, append-only asset history, Docker, CI | **This PR** |
| 2. App on the API | Sign-in screen, inventory/buildings/maintenance pages backed by the API, user management, asset detail with history, UI polish. CSV export and search carry over. Demo mode stays available | Next |
| 3. Assignments and relationships | Assign assets to people and workstations, check-out/check-in, parent-child assets (a workstation with its monitor, scanner and dock), moving a parent moves its children | Planned |
| 4. QR and barcode | Printable QR/barcode labels per asset tag, camera scanning in the browser, scan to look up, scan to verify, USB/Bluetooth scanner input | Planned |
| 5. Audits | Weekly and monthly audits run independently, each with its own scope, schedule, assignee, scan-based verification, discrepancy list (missing, unexpected, wrong location) and a locked, immutable record when closed | Planned |
| 6. Maintenance | Work orders with priority, assignee, vendor, cost and notes; status flow; full maintenance history on each asset | Planned |
| 7. Reporting and import | Dashboard KPIs from live data, audit and maintenance reports, CSV import with validation and preview, CSV/PDF export, scheduled email summaries | Planned |
| 8. Production | Hosting and managed PostgreSQL, devicetrustline.com domain and TLS, backups, error monitoring, email invitations and password reset, security review | Planned |

## Decisions so far

- **Stack:** React + Vite frontend (existing), Node 22 + Express API in `server/`, PostgreSQL 16, plain SQL migrations.
- **Tenancy:** every table carries `org_id`; composite foreign keys stop rows from one organization referencing another's buildings, locations or users.
- **Sessions:** signed token in an httpOnly, SameSite=Lax cookie; users are re-checked on every request, so deactivation, role changes and password resets take effect at once.
- **Roles:**

  | Permission | admin | manager | technician | auditor | viewer |
  |---|:-:|:-:|:-:|:-:|:-:|
  | View assets, buildings | ✓ | ✓ | ✓ | ✓ | ✓ |
  | Add and edit assets | ✓ | ✓ | ✓ | | |
  | Manage buildings and locations | ✓ | ✓ | | | |
  | View users | ✓ | ✓ | | | |
  | Manage users and roles | ✓ | | | | |

  Auditors gain audit permissions in phase 5; technicians gain maintenance permissions in phase 6.
- **History:** `asset_events` is append-only, enforced by a database trigger. Assets are retired, never deleted.
