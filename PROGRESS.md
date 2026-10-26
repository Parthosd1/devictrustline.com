# Release 1 progress

Branch `release-1`, pull request #10. Updated 2026-10-26.

Languages actually used: React 18 + Vite (JavaScript) for the web app; Rust (Axum, SQLx, Tokio) for the API and background jobs; Rust for the Windows device companion; PostgreSQL with plain SQL migrations; JavaScript (Node test runner) for black-box HTTP tests against the Rust binary.

## Implemented and tested

| Area | What | Commit | Tests |
|---|---|---|---|
| Accounts | Memberships, personal and business workspaces, workspace switcher, controlled sign-up (`ALLOW_SIGNUP` off in production), email confirmation, password reset | 9b998b8 and earlier | `tests/http/accounts.test.js` |
| Invitations | Invite by email or copyable link (shown when email is not set up), accept, revoke, expiry; roles per workspace | 9b998b8 | `accounts.test.js` |
| Email | Outbox with retries; status shows queued, sent or failed, and only says sent after the SMTP server accepted it; capture mode for tests | 9b998b8 | `accounts.test.js`, `warranties-reminders.test.js` |
| Google sign-in | OpenID Connect with PKCE and state; linking to an existing password account only after signing in to it; matching email alone never joins accounts; secret stays on the server | 952f780 | `google.test.js` (local mock provider) |
| Registry | Categories, tags, custom fields, condition, owner, price, currency, seller, return deadline, maintenance date | 9b998b8 | `registry.test.js` |
| Attachments | Photos, receipts, manuals; type checked by content; per-workspace storage; plan storage limits | 9b998b8 | `registry.test.js` |
| Warranties | Several per item, proof of purchase, states active / ending soon / expired / unknown | 9b998b8 | `warranties-reminders.test.js` |
| Reminders | Warranty, return, maintenance, loan and audit reminders in the workspace time zone; fire once; cancelled when dates change; missed ones recorded, not announced late | 9b998b8 | `warranties-reminders.test.js` |
| Notifications and insights | Bell, read and read-all, deadlines list, suggestions with reasons (nothing sold or listed automatically) | 9b998b8 | `warranties-reminders.test.js` |
| Plans | Personal Basic, Personal Plus, Business limits, enforced under concurrency; upgrade returns `billing_unconfigured` until a provider exists; no prices invented | 682be92 | `plans-agents.test.js` |
| Device companion | Rust agent: pair with one-time code, scan, report, background run with backoff, pause, disconnect, Windows scheduled task; server-side review (add, link, ignore); only real serials kept; never merged by model name | 682be92 | `plans-agents.test.js` (drives the real agent binary on Linux), 6 agent unit tests |
| Workspace isolation | Every route, export, attachment, job and agent enrollment scoped to the workspace | all | cross-workspace checks throughout the HTTP tests |
| Web app | Sign-in, sign-up type, reset, Google button, invite acceptance, switcher, catalog, item form and detail with photos and warranties, loans, bell, personal and business dashboards, devices, plan, settings | 2808829 | `npm run build`; headless Chromium walk-through at 1366×860 and 390×844 (no console errors besides the expected 401 before sign-in, no horizontal overflow) |
| Deployment | Image includes the Windows agent download; persistent files volume; backups include files; Mailpit for local email; `.env.example` complete | 9122023 | `scripts/test-deploy.sh`: fresh install, upgrade from the previous release (schema 005) with data, backup and restore of database and files |
| CI | frontend build; API fmt, clippy, unit and 87 HTTP tests; agent fmt, clippy (Linux and Windows), tests, Windows build artifact; deploy tests; Docker images | ab91c63, 9122023 | GitHub Actions |

## Awaiting configuration

These work in code and tests, and need accounts or settings only the owner can provide:

1. **Hosting and DNS.** Choose Render (recommended) or your own VM (see `docs/DEPLOYMENT.md`), then point `devicetrustline.com` at it. Nothing is deployed yet, so no URL is live.
2. **Email.** Create an SMTP account (Postmark, SendGrid, Amazon SES or Mailgun), set `SMTP_URL` and `MAIL_FROM`, and add SPF and DKIM records for the sending domain. Until then invitations show a link to copy and no email is sent.
3. **Google sign-in.** Create an OAuth client (Web application) in Google Cloud with redirect URI `https://devicetrustline.com/api/auth/google/callback`, then set `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`.
4. **Payments.** No payment provider is connected and no prices are set. Plan changes are made with `devicetrustline set-workspace-plan` until prices and a provider are chosen.
5. **Code signing.** The Windows companion is unsigned, so SmartScreen warns on first run.

## Unverified

- The device companion has not run on a physical Windows PC. It cross-compiles for Windows and its parsers are unit-tested with sample PowerShell output, but real hardware readings (monitor serials over EDID, USB devices, printers) are untested.
- Real SMTP delivery and real Google sign-in: tested only against a capture mailbox and a local mock OpenID provider.
- The production Docker image is built in CI only; it has not been run on a real server.
- Accessibility: keyboard focus, labels and roles were added and checked by hand; no screen reader or automated audit was run.

## Next task

Merge PR #10 once CI is green, then follow "Before going live" in `docs/DEPLOYMENT.md` after the hosting choice.
