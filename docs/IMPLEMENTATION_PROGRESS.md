# Implementation progress: production completion (phases 1–11)

Started 2026-10-27 from `main` at 5fd0503 (PRs #1–#11 merged). This file is the checkpoint for
resumed sessions: read it first, continue from "Next action".

## Gap checklist against main

Legend: ✅ already on main · 🔧 extended in this work · 🆕 new in this work · ⛔ blocked on an external dependency

| Area | On main | Gap to close | Phase |
|---|---|---|---|
| Personal workspaces | ✅ kind = personal, own categories, members | 🔧 categories for phones, TVs, consoles, routers, printers; archive / restore separate from Retired | 1 |
| Item fields | ✅ name, category, brand, model, serial, purchase date, price, seller, receipt and photo attachments, warranties (incl. extended), return deadline, maintenance due, condition, notes, tags, location, owner, custom fields (incl. date fields) | 🔧 archive / restore, filter archived | 1 |
| Export | ✅ client-side CSV | 🔧 server CSV export of own records | 1 |
| Automatic reminders | ✅ derived from warranty / return / maintenance / audit / loan dates; workspace day lists; fire once; in-app + email | 🆕 user-managed reminders (8 kinds, recurring, custom offsets), snooze, complete, reschedule, delete | 1 |
| Notification delivery | ✅ in-app bell, email outbox with retries | 🆕 per-user preferences (channels, timezone, quiet hours), persisted occurrence jobs, delivery log per channel with retries, Web Push, SMS provider interface (off until configured) | 1 |
| Hierarchical inventories | ✅ buildings → locations, parent/child assets | 🆕 inventories and sub-inventories, transfers (single, bulk, scan), approval, history, custody | 2 |
| Roles | ✅ admin, manager, technician, auditor, viewer per workspace; invitations with expiry and revoke | 🆕 owner role; inventory-scoped access grants enforced on the server; invitations carrying grants | 3 |
| Loaners | ✅ assign with due date, loan reminders | 🆕 loan records with Reserved → Deployed → Returned → Inspected → Available, exceptions, extensions, acknowledgment, overdue notices, history, reports | 4 |
| Audits | ✅ weekly / monthly runs, scan, mark, close, immutable once closed, scheduler | 🆕 audit types (soft, full hub, spot, transfer, return), frequencies (quarterly, annual, manual), draft → in progress → submitted → approved / rejected → locked, review history, self-approval rule, damaged and duplicate-scan tracking, report CSV | 5 |
| Manager approval center | — | 🆕 one queue for audits, transfers, overdue loans, missing / damaged, warranties, maintenance, access requests, discrepancies; follow-up tasks | 6 |
| Business notifications | ✅ reminders only | 🆕 event notifications (audit, transfer, loan, missing, assignment, return, invitation) with workspace defaults and user overrides, scoped to what each person may see | 7 |
| Web app | ✅ dashboards, inventory, detail, form, scan, labels, audits, buildings, people, maintenance, reports, users, plan, settings, devices | 🆕 pages for reminders, notification center and settings, inventories and transfers, loaners, audit approval, approval center, delegated access | 8 |
| Accounts | ✅ sign-up, login, reset, email verification, Google, invitations, workspace isolation | — | 9 |
| Plans | ✅ Personal Basic / Plus / Business limits enforced; billing reports unconfigured | 🔧 Free Personal / Personal Pro / Business / Enterprise, limits for inventories, notifications, reports | 9 |
| Windows companion | ✅ pairing, revocable credential, computer, monitors, disks, printers, peripherals, network adapters | 🔧 CPU, RAM, storage totals, OS version; ⛔ physical Windows test | 10 |
| Production | ✅ Docker, CI, migrations, health check, backups, auth rate limit, origin check | 🔧 tests for the 14 critical workflows, browser end-to-end test, launch checklist; ⛔ hosting, DNS, SMTP, Google OAuth client, payments, VAPID keys, SMS provider | 11 |

## Branches and pull requests

Each phase is stacked on the previous one.

| Phase | Branch | PR | Status |
|---|---|---|---|
| 1 | `phase-1-personal-notifications` | (opening) | backend complete, tested |

## Phase 1 — what was built

- Migration 010: `assets.archived_at`; personal categories for phones, TVs, gaming, routers, printers;
  `notification_settings`, `push_subscriptions`, `user_reminders`, `reminder_jobs`,
  `notification_deliveries`, SQL function `quiet_hours_end`.
- `src/notify.rs`: one `send` for every notification (in-app row, then email / push / SMS per the
  person's settings, deferred past quiet hours, idempotent by dedupe key) and a delivery worker
  with retries and backoff; expired browser subscriptions are dropped.
- `src/push.rs`: Web Push (RFC 8291 aes128gcm encryption, RFC 8292 VAPID), `devicetrustline vapid-keys`.
- `src/sms.rs`: SMS provider interface: disabled (default), `log` (development), `twilio`.
- `src/user_reminders.rs`: persisted reminder jobs, items' dates followed, recurrence, snooze, retries.
- Workspace (derived) reminders now deliver through the same engine and skip a person who set their
  own reminder for that item and deadline.
- Routes: reminders CRUD, snooze, complete; notification settings, test send, push subscriptions,
  notification delete and delivery log; item archive / restore and CSV export.

## Test results

Baseline on main (2026-10-27): `cargo build` ok, `scripts/test-http.sh` 87/87 pass.
Phase 1 (2026-10-27): `cargo fmt --check`, `cargo clippy -D warnings`, 16 unit tests, 94/94 HTTP tests
pass (7 new in `personal-reminders.test.js`, covering critical workflows 1 and 2, push decrypted by a
stand-in push service, SMS retry, quiet hours, archive and export).

## Next action

Phase 2: inventories and sub-inventories, transfers with approval, on branch
`phase-2-inventories` stacked on phase 1.
