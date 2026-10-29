# DeviceTrustline production completion: final report

Date: 2026-10-29. Covers phases 1–11 of the production spec. The work is in eleven stacked pull
requests, #12–#22. **Nothing has been merged or deployed.** The site is not live, and nothing in
this report says that it is.

## 1. Features implemented

| Phase | PR | What was built |
|---|---|---|
| 1. Personal notifications | #12 | Personal device records (new categories, archive and restore, server CSV export). User-managed reminders: 8 kinds, recurring, custom offsets, snooze, pause, complete, reschedule and delete. Per-user channels, time zone and quiet hours. A delivery log per channel with retries. Web Push, and an SMS provider interface that stays off until configured. |
| 2. Inventories and transfers | #13 | Inventories and sub-inventories. Single, bulk and scanned transfers, with optional approval, history and custody. |
| 3. Delegated access | #14 | Owner role. Inventory-scoped access enforced on the server. Invitations that carry access, with expiry and revoke. Access requests. |
| 4. Loaners | #15 | Loans go Reserved → Deployed → Returned → Inspected → Available. Also: exceptions (damaged, lost, repair), extensions, borrower acknowledgment, one overdue notice per due date, history, CSV reports. |
| 5. Audits | #16 | Audit types: soft, full hub, spot, transfer and return. Frequencies from weekly to annual, plus manual. Audits go draft → in progress → submitted → approved or rejected → locked. Also: review history, the auditor cannot approve their own audit, damaged and duplicate-scan tracking, and a CSV report. |
| 6. Approval center | #17 | One queue for audits, transfers, overdue loans, missing and damaged items, warranties, maintenance, access requests and discrepancies, with follow-up tasks. |
| 7. Business notifications | #18 | Event notices for audits, transfers, loans, missing items, assignments, returns and invitations. Workspace defaults and personal overrides. A person is told only about items they can see. |
| 8. Responsive UI | #19 | Web pages for approvals, inventories and transfers, loaners, reminders, notification center and settings, audit review, and delegated access. Checked with no sideways scrolling at 390px. |
| 9. Plans | #20 | Free Personal, Personal Pro, Business and Enterprise. Limits and features (members, items, inventories, reminders, storage, SMS, reports) are enforced by the API. Billing shows as not configured. |
| 10. Windows companion | #21 | The Windows and Linux companion now reports CPU, memory, storage, OS and network adapters, and the Devices page shows them. |
| 11. Security and testing | #22 | Automated tests for the 14 critical workflows, a browser end-to-end test that runs in CI, rate-limit unit tests, a spot-audit form fix, a full `api/.env.example`, a security review, a launch checklist and this report. |

## 2. Commits and pull requests

All on `Parthosd1/devictrustline.com`, each PR based on the one before:

| PR | Branch | Commit(s) |
|---|---|---|
| #12 | `phase-1-personal-notifications` → `main` | ef8e72b |
| #13 | `phase-2-inventories` → phase 1 | ce7ec0e, 73048a4 |
| #14 | `phase-3-delegated-access` → phase 2 | 331b33f, d6e5d13 (merge of phase 2) |
| #15 | `phase-4-loaners` → phase 3 | 615b503 |
| #16 | `phase-5-audits` → phase 4 | be03454 |
| #17 | `phase-6-approvals` → phase 5 | b9e3231 |
| #18 | `phase-7-notifications` → phase 6 | 88e6f7a |
| #19 | `phase-8-ui` → phase 7 | eee371b |
| #20 | `phase-9-plans` → phase 8 | 200d658 |
| #21 | `phase-10-companion` → phase 9 | 5f3ce0d |
| #22 | `phase-11-launch` → phase 10 | see the PR |

## 3. Tests: what was run and the results

These were all executed. None of the numbers are estimates.

| Check | Where it ran | Result |
|---|---|---|
| `cargo fmt --check`, `cargo clippy -D warnings` (API) | locally, and CI on #12–#21 | pass |
| API unit tests | locally | 18 / 18 pass |
| API HTTP integration suite (real server, real Postgres) | locally | 142 / 142 pass |
| The 14 critical workflows (`critical-workflows.test.js`) | locally | 14 / 14 pass |
| Browser end-to-end walk-through (Chromium: owner at 1366px, manager at 390px) | locally | pass |
| Web production build (`npm run build`) | locally, and CI | pass |
| Companion fmt, clippy and 6 unit tests; Windows `.exe` cross-build | locally (tests), CI (Windows build) | pass |
| Deploy test: fresh install, upgrade with data, backup and restore | CI `deploy` job on #12–#21 | pass |
| Docker image builds | CI `docker` job on #12–#21 | pass. Not run locally, because Docker Hub rate-limits this sandbox. |
| CI on #22, including the new `e2e` job | GitHub Actions | see PR #22 (running at the time of writing) |

**Failed:** none of the checks that were run.

**Not run:** the companion on a physical Windows PC; real email, push and SMS delivery (they need
credentials); load testing; a penetration test by a third party.

### The 14 critical workflows

Each one is its own test in `api/tests/http/critical-workflows.test.js`, and all of them pass:

1. A personal user creates a device and schedules a warranty reminder.
2. They edit, snooze and disable the reminder.
3. A manager creates a sub-inventory and delegates access.
4. A technician transfers a device into the sub-inventory.
5. A technician deploys a loaner.
6. An overdue loaner triggers exactly one notification, even when the job runs three times.
7. The loaner is returned and inspected.
8. An auditor starts a full hub audit.
9. The auditor scans assets and submits the audit.
10. A manager rejects it with a reason.
11. The auditor corrects and resubmits it.
12. The manager approves it, and the database then refuses changes to the locked audit.
13. An unauthorized user cannot reach another workspace.
14. Concurrent requests cannot double-loan or double-transfer an asset. Six requests at once give one loan; six transfers at once move the item once.

## 4. Still incomplete

- **Payments.** No payment provider is connected. Plans are changed by the operator with
  `devicetrustline set-workspace-plan`, and the upgrade button says billing is not configured.
- **Windows companion on real hardware.** It is built and unit-tested but has not been run on a
  physical Windows PC. It is also not code-signed.
- **SMS** needs a Twilio account, and **push** needs VAPID keys. The code paths are tested against
  stand-in services.
- **Rate limiting is per instance.** Add a proxy limit if you run several instances.
- **No mobile app.** The web app is responsive and works on phones.

## 5. External credentials and services needed

| Needed for | What | Required? |
|---|---|---|
| Hosting | A Docker VM (with the included Caddy for HTTPS) or a Render account | yes |
| Domain | DNS for devicetrustline.com | yes |
| Secrets | `JWT_SECRET` and `POSTGRES_PASSWORD`, generated by you | yes |
| Email | SMTP account (`SMTP_URL`, `MAIL_FROM`) plus SPF and DKIM | yes, for password reset and invitations |
| Google sign-in | Google OAuth client ID and secret | optional |
| Push | VAPID key pair (`devicetrustline vapid-keys`) | optional |
| SMS | Twilio SID, token and number | optional |
| Billing | A payment provider (not yet integrated) | for self-service upgrades |
| Companion | A Windows code-signing certificate | before customer distribution |

No secrets are committed. All of the files with "example" in their name contain placeholders only.

## 6. Deployment readiness

The code is ready for a review and staging deployment. It is not live, and has not been deployed
anywhere. What is in place:

- the Docker image and compose stack (HTTPS, backups)
- the Render blueprint
- migrations that run on start
- the health check
- production config checks that refuse weak secrets and test mail or SMS modes
- CI with six jobs

Before launch, the PRs need review and merging, and steps 2–5 of `docs/LAUNCH_CHECKLIST.md` must
be done with your accounts.

## 7. Exact steps to go live

1. Review and merge PRs #12 → #22 in order, and confirm CI is green on `main`.
2. Choose hosting (`docs/DEPLOYMENT.md` Option A or B) and point DNS for devicetrustline.com at it.
3. Generate `JWT_SECRET` and `POSTGRES_PASSWORD`, and set `APP_URL` and `APP_ORIGIN` to
   `https://devicetrustline.com`.
4. Create the SMTP account and set `SMTP_URL` and `MAIL_FROM` with SPF and DKIM. Optionally set up
   the Google OAuth client, VAPID keys and Twilio.
5. Deploy. Check that `https://devicetrustline.com/api/health` returns 200.
6. Run `devicetrustline create-admin`, sign in, and send a test notification from Settings.
7. Pair the Windows companion on a real PC and go through the `agent/README.md` checks. Sign the
   executable.
8. Set up an uptime monitor and off-server backup copies, and test one restore.
9. Announce.
