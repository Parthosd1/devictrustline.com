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
| 1 | `phase-1-personal-notifications` | #12 → main | backend complete, tested |
| 2 | `phase-2-inventories` | #13 → phase 1 | backend complete, tested |
| 3 | `phase-3-delegated-access` | #14 → phase 2 | backend complete, tested |
| 4 | `phase-4-loaners` | #15 → phase 3 | backend complete, tested |
| 5 | `phase-5-audits` | (opening) → phase 4 | backend complete, tested |

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

## Phase 2 — what was built

- Migration 011: `inventories` (nested, kinds, place, custodian, approval flag, archive),
  `assets.inventory_id` (one inventory per item), `inventory_transfers` (pending / completed /
  rejected / cancelled; one pending per item by unique index; decided rows immutable by trigger),
  `seed_inventories()` giving every business workspace a Main IT Hub with Available, Loaner,
  RMA / Repair, Deployment and Returned sub-inventories.
- Routes: inventories CRUD, archive (only when empty) and restore; transfers by id or scanned code,
  single or bulk, components move with their parent, optional expected source; approve, reject
  (reason required), cancel; item list filter by inventory and sub-inventories.
- `src/events.rs`: approvers are notified of pending transfers; requesters of decisions.
- Permissions: `inventory:read` (all), `inventory:manage` and `transfers:approve` (admin, manager),
  `inventory:transfer` (admin, manager, technician).

## Phase 3 — what was built

- Migration 012: `owner` role (each workspace's longest-standing active admin becomes its owner;
  owners hold every admin permission plus `workspace:own`), `memberships.access_scope`
  (`all` | `inventories`, never limited for owners and admins), `inventory_grants`, invitation
  tokens carrying `access_scope` and `inventory_ids`, `access_requests` (one pending per person
  and inventory).
- Sign-up and new workspaces make their creator the owner. Only owners make, change or remove
  owners; a workspace always keeps one active owner (owners are locked during the check).
- `src/scope.rs`: middleware enforcing limited access on every route that takes an item,
  attachment, warranty, work order, transfer or inventory id (outside access answers 404, exactly
  like another workspace); reports, insights, devices, audits and bulk import are closed to limited
  users, and catalog, sites, people and settings are read-only for them. Lists (items, CSV export,
  inventories, transfers, work orders, deadlines) filter in SQL by the same scope. A user's scope
  is the granted inventories plus all their sub-inventories, recomputed on every request, so
  changes apply without signing in again.
- Users: `accessScope` / `inventoryIds` on create and update (validated against active
  inventories of the workspace); promotion to admin or owner widens access to everything.
- Invitations carry access; on acceptance it is applied (inventories archived since are dropped;
  if none remain the invitation is refused rather than widened). Expiry and revoke unchanged.
- Access requests: limited users ask for an inventory; managers and admins with full access
  approve (adds the grant) or reject; no self-decision; the requester may cancel; both sides are
  notified.
- Audits are closed to limited users for now; Phase 5 adds inventory-based audits that respect scope.

## Phase 4 — what was built

- Migration 013: `loans` (numbered LN-1001…, reserved → deployed → returned → inspected, plus
  cancelled and lost; one open loan per item by unique index; closed loans immutable by trigger),
  `loan_events` (append-only history), `organizations.next_loan_number`.
- `src/routes/loans.rs`: reserve (by id or scanned code; item must be in a loaner inventory or one
  inside it, available, unassigned; optional `deployNow`), deploy with borrower acknowledgment and
  checkout condition, extend (reason required; technicians once, managers again), return (condition,
  damage note; damaged items go to Maintenance), inspect (`ready` → Available, `repair` →
  Maintenance, note required), cancel a reservation, report lost (managers and admins).
- The item's assignment and `loan_due` follow the loan, so the existing due-back reminders still
  fire; plain assign / return is refused while a loan is out or awaiting inspection.
- Overdue job (every background run): one `overdue` record and one notification per due date to
  owners, admins and managers with full access and to the loaner inventory's custodian.
- Reports: list summary (reserved, deployed, overdue, awaiting inspection, lost, completed in 30
  days, average loan days, damaged returns), filters, and CSV export. Scope rules from Phase 3 apply.
- Permissions: `loans:read` (all roles), `loans:manage` (admin, manager, technician),
  `loans:resolve` (admin, manager).
- CI fix carried from Phase 2: the inventory tree sorts by lowercased names with byte collation, so
  the order no longer depends on the database locale (CI uses en_US, local used C).

## Phase 5 — what was built

- Migration 014: audit types (`soft`, `full`, `spot`, `transfer`, `return`), frequencies (`weekly`,
  `monthly`, `quarterly`, `annual`, `manual`), audits scoped to an inventory and everything inside
  it or to chosen items, statuses `draft` → `open` (in progress) → `submitted` → `approved` (locked) |
  `rejected` → `open`, plus `closed` for audits that need no review; `audit_reviews` (append-only
  history of started / submitted / approved / rejected / reopened / closed); damaged flag and scan
  count on items; a database check that the reviewer is neither the starter nor the submitter.
- Routes: start (type, period, inventory, items, draft, requiresReview), begin, scan (duplicates
  counted and flagged), mark (damaged needs a note), submit, approve, reject (note required), reopen,
  close (only without review), CSV export. Defaults: soft and untyped (original) audits close
  directly; full, spot, transfer and return audits need review. Return audits default to the
  Returned Equipment inventory.
- `audits:approve` (admin, manager). Reviewers who can see the audit are notified on submit; the
  submitter is notified of the decision.
- Automatic audits: quarterly and annual as well as weekly and monthly (`autoAudits` settings).
- Limited-access users audit only inside their inventories (Phase 3 had closed audits to them).

## Test results

Baseline on main (2026-10-27): `cargo build` ok, `scripts/test-http.sh` 87/87 pass.
Phase 1 (2026-10-27): `cargo fmt --check`, `cargo clippy -D warnings`, 16 unit tests, 94/94 HTTP tests
pass (7 new in `personal-reminders.test.js`, covering critical workflows 1 and 2, push decrypted by a
stand-in push service, SMS retry, quiet hours, archive and export).

Phase 2 (2026-10-29): fmt, clippy, 16 unit tests, 101/101 HTTP tests (7 new in
`inventories.test.js`: workflows 3 (inventory part), 4 and 14 (transfers)).

Phase 3 (2026-10-29): fmt, clippy, 16 unit tests, 108/108 HTTP tests (7 new in
`delegated-access.test.js`: workflow 3 (delegation), workflow 13 (cross-workspace), scoped
manager, owner / admin rules, read-only roles, invitations with access, expiry and revoke, access
requests; `rbac.test.js` rewritten for owner rules).

Phase 4 (2026-10-29): fmt, clippy, 16 unit tests, 113/113 HTTP tests under an en_US database (5 new in
`loaners.test.js`: full lifecycle, eligibility, exceptions, overdue notices and reports, scope and
cross-workspace).

Phase 5 (2026-10-29): fmt, clippy, 16 unit tests, 118/118 HTTP tests (5 new in
`audit-workflow.test.js`; `audits.test.js` and `reports-import.test.js` updated for the new summary
counters and settings).

## Next action

Phase 6: manager approval center (one queue for audits, transfers, overdue loans, missing / damaged
items, warranties, maintenance, access requests, discrepancies; follow-up tasks); branch
`phase-6-approvals` on phase 5.
