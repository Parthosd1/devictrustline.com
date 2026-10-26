#!/usr/bin/env sh
# Starts the API on a disposable database and runs the black-box HTTP tests in tests/http
# against it. Needs PostgreSQL and Node; set TEST_DATABASE_URL if the test database is not
# postgres://postgres:postgres@localhost:5432/devicetrustline_test. The database is wiped.
set -eu
cd "$(dirname "$0")/.."
cargo build --quiet
export TEST_DATABASE_URL="${TEST_DATABASE_URL:-postgres://postgres:postgres@localhost:5432/devicetrustline_test}"
port="${TEST_PORT:-4100}"
static_dir="$(mktemp -d)"
log="$(mktemp)"
files_dir="$(mktemp -d)"
# A stand-in Google, so sign-in with Google runs end to end.
oidc_port="${MOCK_OIDC_PORT:-4101}"
MOCK_OIDC_PORT="$oidc_port" node tests/http/mock-oidc.js >/dev/null 2>&1 &
oidc=$!
DATABASE_URL="$TEST_DATABASE_URL" PORT="$port" STATIC_DIR="$static_dir" ATTACHMENTS_DIR="$files_dir" ALLOW_SIGNUP=true \
  AUTH_RATE_LIMIT=1000 DISABLE_SCHEDULER=true MAIL_MODE=capture APP_URL=http://app.test \
  GOOGLE_CLIENT_ID=test-client GOOGLE_CLIENT_SECRET=test-secret GOOGLE_ISSUER="http://127.0.0.1:$oidc_port" ./target/debug/devicetrustline >"$log" 2>&1 &
server=$!
trap 'kill $server $oidc 2>/dev/null; rm -rf "$static_dir" "$files_dir" "$log"' EXIT
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:$port/api/health" >/dev/null && break
  kill -0 $server 2>/dev/null || { cat "$log"; exit 1; }
  sleep 0.2
done
bin="$(pwd)/target/debug/devicetrustline"
cd tests/http
[ -d node_modules ] || npm ci --silent
status=0
TEST_API_URL="http://127.0.0.1:$port" TEST_API_BIN="$bin" TEST_DATABASE_URL="$TEST_DATABASE_URL" \
  TEST_STATIC_DIR="$static_dir" npm test || status=$?
[ $status -eq 0 ] || { echo "--- server log ---"; tail -50 "$log"; }
exit $status
