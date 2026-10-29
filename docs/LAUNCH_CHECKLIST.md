# Launch checklist

Work through this list in order. Steps marked **(you)** need an account, a credential or a decision
from the owner, so nothing in the code can do them. How to deploy is in `docs/DEPLOYMENT.md`.
The variables are listed in `deploy/.env.example` (own server) and `render.yaml` (Render).

## 1. Merge the code

- [ ] Review and merge the stacked pull requests in order: #12 (phase 1) → … → #22 (phase 11). Each
      PR is based on the one before, so merge #12 into `main` first, then re-target the next PR to `main`.
- [ ] Confirm CI is green on `main` after the last merge. The jobs are frontend, api, deploy, agent,
      docker and e2e.

## 2. Hosting and domain (you)

- [ ] Choose hosting: your own Linux VM with Docker (Option A) or Render (Option B).
- [ ] Point DNS for `devicetrustline.com` and `www.devicetrustline.com` at it.
- [ ] Generate secrets on your own machine and put them only in the host's environment, never in git.
      Use `openssl rand -base64 48` for `JWT_SECRET` and `POSTGRES_PASSWORD`.
- [ ] Set `APP_URL` and `APP_ORIGIN` to `https://devicetrustline.com`.
- [ ] Set `ALLOW_SIGNUP` to choose whether anyone can create an account, and `TRUST_PROXY=true` only
      behind a proxy you control (Caddy and Render both qualify).

## 3. Email (you, needed before launch)

Without email, password reset and invitations by email do not work.

- [ ] Create an account with an SMTP provider (Postmark, SES, Mailgun, SendGrid or similar).
- [ ] Set `SMTP_URL` and `MAIL_FROM`, and add the SPF and DKIM records for the sending domain.
- [ ] Leave `REQUIRE_EMAIL_VERIFICATION` on (the default once email is set up).

## 4. Optional services (you)

| Service | What it turns on | Variables |
|---|---|---|
| Google OAuth client | "Sign in with Google". The redirect URI is `https://devicetrustline.com/api/auth/google/callback`. | `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` |
| VAPID key pair | Browser push notifications. Generate it with `devicetrustline vapid-keys`. | `VAPID_PUBLIC_KEY`, `VAPID_PRIVATE_KEY`, `VAPID_SUBJECT` |
| Twilio | SMS reminders for Personal Pro, Business and Enterprise | `SMS_PROVIDER=twilio`, `TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN`, `TWILIO_FROM` |
| Payment provider | Self-service upgrades. **Not built.** Until then, plans are changed by the operator with `devicetrustline set-workspace-plan`. | — |
| Code-signing certificate | Removes the SmartScreen warning on the Windows companion | — (signing step in CI) |

## 5. Deploy and first sign-in

- [ ] Deploy as described in `docs/DEPLOYMENT.md`. Migrations run automatically on start.
- [ ] Check that `https://devicetrustline.com/api/health` returns 200.
- [ ] Run `devicetrustline create-admin` for your workspace and sign in.
- [ ] Under **Settings**, send yourself a test notification and confirm it arrives by email, and by
      push and SMS if those are set up.
- [ ] Create a test personal account and a test business workspace, and check that neither can see
      the other.

## 6. Device companion (you, on a real Windows PC)

- [ ] Download `/downloads/devicetrustline-agent.exe`, pair it, and go through the checks in
      `agent/README.md` ("Checking it on a real Windows PC"): CPU, RAM, storage, OS, network
      adapters, monitors, printers and peripherals.
- [ ] Sign the executable before giving it to customers.

## 7. Operations

- [ ] Point an uptime monitor at `/api/health`.
- [ ] Copy nightly backups off the server (own server: `deploy/backups/`; Render: its managed
      Postgres backups plus the attachments disk).
- [ ] Do one restore into a scratch database (steps in `docs/DEPLOYMENT.md`) before you rely on it.
- [ ] If you run more than one API instance, also rate-limit `/api/auth/*` at the proxy. The built-in
      limit is per instance.

## 8. Go live

- [ ] Announce only after steps 1–5 are done and `/api/health` answers on the real domain.
