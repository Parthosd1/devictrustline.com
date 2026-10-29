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
| 5 | `phase-5-audits` | #16 → phase 4 | backend complete, tested |
| 6 | `phase-6-approvals` | #17 → phase 5 | backend complete, tested |
| 7 | `phase-7-notifications` | #18 → phase 6 | backend complete, tested |
| 8 | `phase-8-ui` | #19 → phase 7 | web pages complete, checked in a browser |
| 9 | `phase-9-plans` | #20 → phase 8 | complete, tested |
| 10 | `phase-10-companion` | #21 → phase 9 | code complete; ⛔ physical Windows test |
| 11 | `phase-11-launch` | #22 → phase 10 | critical workflow tests, browser e2e in CI, launch docs |

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

## Phase 6 — what was built

- `GET /api/approvals`: one queue with sections for pending transfers, submitted audits (with
  `canReview`, false for one's own), access requests, overdue loans, loans awaiting inspection,
  items still missing after their latest finished audit, discrepancies (misplaced / unexpected,
  90 days), damaged items (audits and loan returns, 90 days), warranties ending within 30 days (or
  ended in the last 7), and maintenance due within 7 days; plus counts and open follow-up tasks.
  Each section appears only with the permission to act on it, filtered to the person's access.
  Decisions still go through each area's routes, so there is one rule per action.
- Migration 015 `follow_up_tasks`: title, kind, source (audit / transfer / loan / work order /
  warranty / access request), item, assignee, due date, open / done / cancelled with resolution;
  one open task per source and item. Managers create and edit; the assignee may close their own
  (done needs a resolution); the assignee is notified. Queue rows show `hasTask`.
- Permissions: `tasks:read` (all roles), `tasks:manage` (admin, manager).

## Phase 7 — what was built

- Event notifications grouped as transfers, access requests, audits, missing items, loaners,
  assignments and returns, follow-up tasks, invitations (`notify::GROUPS`). Channels for a group:
  the person's own choice (migration 016 `notification_preferences`), else the workspace default
  (`settings.notifications`, set by admins), else every channel; applied inside `notify::send`, so
  every event (including the overdue-loan job) follows it, together with the person's channel
  settings and quiet hours from Phase 1.
- New events: item assigned / returned (to the item's owner and its inventory's custodian), loan
  returned (to whoever reserved it and the keepers; mentions damage), items missing after an audit
  closes or is approved (custodian and full-access reviewers), invitation accepted (to the inviter).
- SQL `member_sees_asset()`: notices about an item go only to people who may see it.
- Routes: `GET / PUT /api/notification-settings/events`; `PATCH /api/settings {notifications}`.

## Phase 8 — what was built

- New pages: Approval Center (queue sections with approve / reject / send back inline, follow-up
  tasks), Inventories & Transfers (tree, detail with items, create / edit / archive / restore,
  transfer by scanned tags, transfer list with approve / reject / cancel, access requests),
  Loaners (summary, filters, lend or reserve with acknowledgment, hand over, extend, check in with
  damage, inspect, cancel, report lost, history, CSV), Reminders (create, edit, snooze, pause,
  resume, done, delete, per-notification delivery), Notifications (full list with each channel's
  delivery, mark read, delete).
- Audits: weekly, monthly, quarterly, annual and one-off; audit types; inventory scope; drafts;
  submit, approve, send back, reopen; damaged marking; duplicate scans; review history; CSV.
- Users: owner role (only owners grant it), access scope with inventory picker on add, invite and
  change; access requests panel.
- Settings: personal channels (email, browser push via `public/sw.js`, SMS with phone), time zone,
  quiet hours, send hour, default days, test send; per-group event preferences; admins' workspace
  defaults; quarterly and annual automatic audits. The old "email me reminders" checkbox was the
  same setting as the new Email channel and was removed from the account panel.
- Responsive: the phone sidebar scrolls, tabs scroll sideways, forms never overflow at 390px.

## Phase 9 — what was built

- Migration 017: plans Free Personal (`free_personal`), Personal Pro (`personal_pro`), Business,
  Enterprise. Free Personal and Personal Pro replace Personal Basic and Plus, carrying over any
  limits the operator set; workspaces move with them. New limits: `inventories`, `reminders`
  (active, per workspace). New `features`: `sms`, `reports`.
- Defaults: Free Personal 100 items, 2 members, 2 computers, 500 MB, 50 reminders, no text
  messages. Personal Pro 2,000 items, 6 members, 10 computers, 10 GB, text messages. Business
  unlimited items, 100 members, 500 computers, 100 GB, 100 inventories, text messages, reports.
  Enterprise no limits. These are starting values the operator can change with
  `set-plan-limit` / `set-plan-feature`; prices are not stored and belong to the billing provider.
- Enforcement on the server: creating inventories and reminders (402 `plan_limit`), turning on
  text messages and reading reports (402 `plan_feature`); text messages also stop being sent when a
  workspace moves to a plan without them, and the delivery log says why.
- Billing stays a boundary: `/api/billing/checkout` answers 503 `billing_unconfigured`.
- Plan page lists limits and features per plan; Settings explains when text messages are not in the plan.

## Phase 10 — what was built

- Windows companion: the same single PowerShell call now also reads `Win32_OperatingSystem`,
  `Win32_Processor`, `Win32_PhysicalMemory` and `Win32_LogicalDisk` (fixed drives), and adapter
  speed. The computer's report carries `cpu` (name, manufacturer, cores, threads, MHz, sockets),
  `memory` (total GB and each module), `storage` (disk total, free space, drives), `os` (edition,
  version, build, architecture) and `network` (built-in adapters with MAC, type and speed).
  Missing values are left out, never guessed.
- Linux companion: CPU, memory, disk size and OS from procfs, sysfs and `/etc/os-release`.
- Devices page and "Add as a new item" show these details for computers (checked in a browser with a
  report shaped like the Windows parser output).
- ⛔ Not run on a physical Windows PC; `agent/README.md` lists the checks to do on one.

## Phase 11 — what was built

- `api/tests/http/critical-workflows.test.js`: one test per critical workflow (1–14) from the spec,
  in one shared workspace with an owner, manager, technician and auditor.
- `e2e/`: a Playwright walk-through of the real web app (owner at 1366px, manager at 390px) on a
  freshly seeded database, run by the new CI `e2e` job; screenshots are uploaded on failure.
- Rate limiter unit tests; spot audits now require an inventory in the start form (found by the
  walk-through).
- `api/.env.example` lists every variable; `docs/SECURITY_REVIEW.md`, `docs/LAUNCH_CHECKLIST.md`
  and `docs/FINAL_REPORT.md`.

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

Phase 6 (2026-10-29): fmt, clippy, 16 unit tests, 121/121 HTTP tests (3 new in
`approval-center.test.js`).

Phase 7 (2026-10-29): fmt, clippy, 16 unit tests, 125/125 HTTP tests (4 new in
`business-notifications.test.js`).

Phase 8 (2026-10-29): `npm run build` passes. Browser walk-through with Playwright against the real
API on a freshly seeded database: owner at 1366px creates an inventory, transfers by tag, lends
with acknowledgment, checks in damaged, inspects to repair, adds a reminder, starts a full audit,
scans (including a duplicate), submits it, changes an event preference and sends a test; a
manager at 390px approves the audit from the Approval Center and opens every page. No console
errors, no failed API calls, no horizontal overflow on any page at 390px. (The walk-through becomes
a committed end-to-end test in Phase 11.)

Phase 9 (2026-10-29): fmt, clippy, 16 unit tests, 128/128 HTTP tests (3 new in
`plan-tiers.test.js`; plans, notification-settings and SMS tests updated for the new plan codes).

Phase 10 (2026-10-29): agent fmt, clippy and 6 unit tests (Windows parser with a recorded CIM
sample including the new classes; Linux parser with a fake procfs / sysfs tree). The Windows
executable is built by CI (`x86_64-pc-windows-gnu`); it has not been run on Windows.

Phase 11 (2026-10-29): fmt, clippy, 18 unit tests, 142/142 HTTP tests (14 new in
`critical-workflows.test.js`, all pass); browser walk-through (`e2e/run.sh`) passes.

## Next action

All 11 phases are code complete. Remaining work needs the owner: review and merge #12 → #22, then
`docs/LAUNCH_CHECKLIST.md` (hosting, DNS, SMTP, optional Google / VAPID / Twilio, physical Windows
test, payments when chosen).
