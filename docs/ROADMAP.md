# DeviceTrustline roadmap

DeviceTrustline grows from the frontend MVP into a multi-tenant SaaS for IT equipment and building assets.
Each phase ships as its own pull request (or a short series), keeps the existing app working, and has tests in CI.

| Phase | Scope | Status |
|---|---|---|
| 1. Foundation | Node/Express API, PostgreSQL schema and migrations, sign-up/sign-in, five roles, organization isolation, buildings and locations, asset registry, append-only asset history, Docker, CI | **Merged** |
| 2. App on the API | Sign-in screen, inventory/buildings/maintenance pages backed by the API, user management, asset detail with history, UI polish. CSV export and search carry over. Audit checklists stay browser-only until phase 5 | **Merged** |
| 3. Assignments and relationships | Assign assets to people and workstations, check-out/check-in, parent-child assets (a workstation with its monitor, scanner and dock), moving a parent moves its children | **Merged** |
| 4. QR and barcode | Printable QR/barcode labels per asset tag, camera scanning in the browser, scan to look up, USB/Bluetooth scanner input. Scan-to-verify lands with audits in phase 5 | **Merged** |
| 5. Audits | Weekly and monthly audits run independently, each with its own scope, schedule, assignee, scan-based verification, discrepancy list (missing, unexpected, wrong location) and a locked, immutable record when closed. Automatic creation of each week's/month's audit comes with scheduled notifications in phase 7 | **Merged** |
| 6. Maintenance | Work orders with priority, assignee, vendor, cost and notes; status flow; full maintenance history on each asset | **Merged** |
| 7. Reporting and import | Reports page (status, type, building, assignment, warranties, maintenance cost, still-missing assets, recent audits) with CSV downloads; CSV import with validation, preview and all-or-nothing commit; automatic weekly/monthly audits; settings page with password change. Email summaries need an email provider and move to phase 8 | **Merged** |
| 8. Production | One production image serving the app and API, HTTPS with automatic certificates, nightly backups, Render blueprint for managed hosting, first-admin setup command, security headers, request logs, health checks ([docs/DEPLOYMENT.md](DEPLOYMENT.md)). Email invitations and password reset wait on choosing an email provider | **Merged** |
| 9. Rust API | The Node API ported to Rust (Axum, SQLx) with the same endpoints, schema and tests | **Merged** |
| Release 1 | Personal and business workspaces, invitations, email, Google sign-in, categories, tags, custom fields, attachments, warranties, reminders, notifications, insights, plans, Windows device companion, deployment tests. See [PROGRESS.md](../PROGRESS.md) | **In review** |

## Decisions so far

- **Stack:** React + Vite frontend (existing), Rust API (Axum, SQLx) in `api/`, PostgreSQL 16, plain SQL migrations. The API was first written in Node/Express and ported to Rust with the same endpoints, schema and tests.
- **Tenancy:** every table carries `org_id`; composite foreign keys stop rows from one organization referencing another's buildings, locations or users.
- **Sessions:** signed token in an httpOnly, SameSite=Lax cookie; users are re-checked on every request, so deactivation, role changes and password resets take effect at once.
- **Roles:**

  | Permission | admin | manager | technician | auditor | viewer |
  |---|:-:|:-:|:-:|:-:|:-:|
  | View assets, buildings | ✓ | ✓ | ✓ | ✓ | ✓ |
  | Add and edit assets | ✓ | ✓ | ✓ | | |
  | Add people and assign or check in equipment | ✓ | ✓ | ✓ | | |
  | Manage buildings and locations | ✓ | ✓ | | | |
  | View users | ✓ | ✓ | | | |
  | Manage users and roles | ✓ | | | | |

  Admins, managers and auditors start, scan and close audits (phase 5); admins, managers and technicians open and work maintenance orders (phase 6).
- **History:** `asset_events` is append-only, enforced by a database trigger. Assets are retired, never deleted.
