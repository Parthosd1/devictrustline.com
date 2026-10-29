#!/usr/bin/env sh
# Builds the web app and the API, starts them on a fresh, seeded database and runs the browser
# walk-through. Needs PostgreSQL, Node and Rust; set E2E_DATABASE_URL if the database is not
# postgres://postgres:postgres@localhost:5432/devicetrustline_e2e. That database is wiped.
set -eu
cd "$(dirname "$0")"
root="$(cd .. && pwd)"
export E2E_DATABASE_URL="${E2E_DATABASE_URL:-postgres://postgres:postgres@localhost:5432/devicetrustline_e2e}"
port="${E2E_PORT:-4300}"
(cd "$root" && npm run build --silent)
(cd "$root/api" && cargo build --quiet)
bin="$root/api/target/debug/devicetrustline"

# A clean database: drop and recreate it through the server's own connection settings.
admin_url="${E2E_DATABASE_URL%/*}/postgres"
db="${E2E_DATABASE_URL##*/}"
psql "$admin_url" -qc "DROP DATABASE IF EXISTS $db" -c "CREATE DATABASE $db"
DATABASE_URL="$E2E_DATABASE_URL" "$bin" migrate >/dev/null
export SEED_ADMIN_PASSWORD="demo password 123"
DATABASE_URL="$E2E_DATABASE_URL" SEED_ADMIN_EMAIL=demo@example.com "$bin" seed >/dev/null

files_dir="$(mktemp -d)"
log="$(mktemp)"
DATABASE_URL="$E2E_DATABASE_URL" PORT="$port" STATIC_DIR="$root/dist" ATTACHMENTS_DIR="$files_dir" \
  AUTH_RATE_LIMIT=1000 DISABLE_SCHEDULER=true MAIL_MODE=capture APP_URL="http://127.0.0.1:$port" "$bin" >"$log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null; rm -rf "$files_dir" "$log"' EXIT
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:$port/api/health" >/dev/null && break
  kill -0 $server 2>/dev/null || { cat "$log"; exit 1; }
  sleep 0.2
done
[ -d node_modules ] || npm ci --silent
status=0
E2E_URL="http://127.0.0.1:$port" node walkthrough.mjs || status=$?
[ $status -eq 0 ] || { echo "--- server log ---"; tail -50 "$log"; }
exit $status
