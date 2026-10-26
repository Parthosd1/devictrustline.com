#!/usr/bin/env bash
# Deployment checks against a real PostgreSQL, using the release binary:
#   1. fresh install: migrate, create the first admin, sign in, health check;
#   2. upgrade: a database created and filled by the previous release (schema 005) is migrated
#      to this release with its data intact (needs OLD_BIN, the previous release's binary);
#   3. backup and restore: pg_dump plus the attachments directory, restored into a new database
#      and directory, serve the same data and files.
#
#   cargo build --release && OLD_BIN=/path/to/old/devicetrustline ./scripts/test-deploy.sh
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="${BIN:-$PWD/target/release/devicetrustline}"
OLD_BIN="${OLD_BIN:-}"
PG="${PG_ADMIN_URL:-postgres://postgres:postgres@localhost:5432}"
WORK="$(mktemp -d)"
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done; rm -rf "$WORK"; }
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok - $*"; }
newdb() { psql -q "$PG/postgres" -c "DROP DATABASE IF EXISTS $1" -c "CREATE DATABASE $1" >/dev/null; echo "$PG/$1"; }
sql() { psql -qtAX "$1" -c "$2"; }

# serve DATABASE_URL PORT FILES_DIR [BIN]: starts the server and waits until it is healthy.
serve() {
  APP_ENV=production JWT_SECRET="deploy-test-secret-deploy-test-secret-0123" DATABASE_URL="$1" PORT="$2" \
    APP_ORIGIN="http://localhost:$2" ATTACHMENTS_DIR="$3" "${4:-$BIN}" serve >"$WORK/server-$2.log" 2>&1 &
  PIDS+=($!)
  for _ in $(seq 1 100); do
    curl -fs "http://localhost:$2/api/health" >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  cat "$WORK/server-$2.log" >&2
  fail "server on port $2 did not start"
}
stop() { kill "${PIDS[-1]}"; wait "${PIDS[-1]}" 2>/dev/null || true; unset 'PIDS[-1]'; }

# login PORT EMAIL PASSWORD: signs in and keeps the session in a cookie jar for that port.
login() {
  curl -fs -c "$WORK/jar-$1" -H 'content-type: application/json' \
    -d "{\"email\":\"$2\",\"password\":\"$3\"}" "http://localhost:$1/api/auth/login" >/dev/null \
    || fail "could not sign in as $2 on port $1"
}
get() { curl -fs -b "$WORK/jar-$1" "http://localhost:$1$2"; }
json() { node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const v=JSON.parse(s);console.log(String(($1)))})"; }

# 1. Fresh install -----------------------------------------------------------------------------
FRESH="$(newdb dt_deploy_fresh)"
DATABASE_URL="$FRESH" "$BIN" migrate >/dev/null
ORG_NAME="Fresh Co" ADMIN_EMAIL="owner@fresh.test" ADMIN_PASSWORD="fresh install password" \
  DATABASE_URL="$FRESH" "$BIN" create-admin >/dev/null
serve "$FRESH" 4501 "$WORK/fresh-files"
login 4501 owner@fresh.test "fresh install password"
[ "$(get 4501 /api/auth/me | json 'v.user.organization.name')" = "Fresh Co" ] || fail "fresh admin workspace"
[ "$(get 4501 /api/categories | json 'v.items.length')" -gt 0 ] || fail "fresh workspace has categories"
[ "$(get 4501 /api/plan | json 'v.plan.code')" = "business" ] || fail "fresh business plan"
# Production keeps sign-up closed unless ALLOW_SIGNUP=true.
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'content-type: application/json' \
  -d '{"email":"x@fresh.test","password":"some long password","name":"X"}' http://localhost:4501/api/auth/signup)
[ "$code" = "403" ] || fail "signup should be closed in production, got $code"
stop
pass "fresh install: migrate, create-admin, sign in, signup closed"

# 2. Upgrade from the previous release --------------------------------------------------------
if [ -z "$OLD_BIN" ]; then
  echo "skip - upgrade test (set OLD_BIN to the previous release's binary)"
else
  UP="$(newdb dt_deploy_upgrade)"
  DATABASE_URL="$UP" "$OLD_BIN" migrate >/dev/null
  SEED_ADMIN_EMAIL="demo@upgrade.test" SEED_ADMIN_PASSWORD="upgrade test password" DATABASE_URL="$UP" "$OLD_BIN" seed >/dev/null
  [ "$(sql "$UP" "SELECT max(name) FROM schema_migrations")" = "005_settings_scheduling.sql" ] || fail "old schema is not 005"
  # Use the old release the way a customer would: a person, an assignment and a repair.
  serve "$UP" 4502 "$WORK/old-files" "$OLD_BIN"
  login 4502 demo@upgrade.test "upgrade test password"
  post() { curl -fs -b "$WORK/jar-4502" -H 'content-type: application/json' -d "$2" "http://localhost:4502$1"; }
  person=$(post /api/people '{"name":"Robin Field","email":"robin@upgrade.test"}' | json 'v.id')
  lent=$(get 4502 '/api/assets?status=Available&limit=1' | json 'v.items[0].id')
  post "/api/assets/$lent/assign" "{\"personId\":\"$person\"}" >/dev/null
  post /api/work-orders "{\"assetId\":\"$lent\",\"title\":\"Cracked hinge\",\"priority\":\"high\"}" >/dev/null
  stop
  before=$(sql "$UP" "SELECT (SELECT count(*) FROM assets)||'/'||(SELECT count(*) FROM asset_events)||'/'||(SELECT count(*) FROM people)||'/'||(SELECT count(*) FROM buildings)||'/'||(SELECT count(*) FROM users)")
  tags_before=$(sql "$UP" "SELECT string_agg(asset_tag||':'||type||':'||coalesce(serial,''), ',' ORDER BY asset_tag) FROM assets")

  serve "$UP" 4502 "$WORK/upgrade-files"   # serve applies the new migrations on start
  after=$(sql "$UP" "SELECT (SELECT count(*) FROM assets)||'/'||(SELECT count(*) FROM asset_events)||'/'||(SELECT count(*) FROM people)||'/'||(SELECT count(*) FROM buildings)||'/'||(SELECT count(*) FROM users)")
  tags_after=$(sql "$UP" "SELECT string_agg(asset_tag||':'||type||':'||coalesce(serial,''), ',' ORDER BY asset_tag) FROM assets")
  [ "$before" = "$after" ] || fail "row counts changed in upgrade: $before -> $after"
  [ "$tags_before" = "$tags_after" ] || fail "asset data changed in upgrade"
  [ "$(sql "$UP" "SELECT count(*) FROM assets WHERE category_id IS NULL")" = "0" ] || fail "assets without category"
  [ "$(sql "$UP" "SELECT count(*) FROM users u WHERE NOT EXISTS (SELECT 1 FROM memberships m WHERE m.user_id = u.id)")" = "0" ] \
    || fail "users without membership"
  [ "$(sql "$UP" "SELECT kind||'/'||plan FROM organizations LIMIT 1")" = "business/business" ] || fail "upgraded workspace kind/plan"
  login 4502 demo@upgrade.test "upgrade test password"
  [ "$(get 4502 '/api/assets?limit=500' | json 'v.total')" = "${after%%/*}" ] || fail "upgraded assets visible through the API"
  first=$(get 4502 '/api/assets?limit=1' | json 'v.items[0].id')
  [ "$(get 4502 "/api/assets/$first/events" | json 'v.items.length')" -gt 0 ] || fail "history kept"
  [ "$(get 4502 "/api/assets/$lent" | json 'v.assignedPersonName')" = "Robin Field" ] || fail "assignment kept"
  [ "$(get 4502 "/api/work-orders?assetId=$lent" | json 'v.items[0].title')" = "Cracked hinge" ] || fail "work order kept"
  stop
  pass "upgrade from schema 005: $after assets/events/people/buildings/users kept, categories and memberships backfilled"
fi

# 3. Backup and restore -----------------------------------------------------------------------
SRC="$(newdb dt_deploy_backup)"
DATABASE_URL="$SRC" "$BIN" migrate >/dev/null
ORG_NAME="Backup Co" ADMIN_EMAIL="owner@backup.test" ADMIN_PASSWORD="backup test password" \
  DATABASE_URL="$SRC" "$BIN" create-admin >/dev/null
serve "$SRC" 4503 "$WORK/src-files"
login 4503 owner@backup.test "backup test password"
item=$(curl -fs -b "$WORK/jar-4503" -H 'content-type: application/json' \
  -d '{"name":"Server rack","type":"Network","serial":"RK-77","warrantyExpires":"2030-01-31"}' \
  http://localhost:4503/api/assets | json 'v.id')
printf '%%PDF-1.4 backup test receipt' >"$WORK/receipt.pdf"
file=$(curl -fs -b "$WORK/jar-4503" -F kind=receipt -F "file=@$WORK/receipt.pdf;type=application/pdf" \
  "http://localhost:4503/api/assets/$item/attachments" | json 'v.id')
stop

# What the backup service in deploy/docker-compose.yml does: pg_dump and a tar of the files.
pg_dump --no-owner "$SRC" | gzip >"$WORK/db.sql.gz"
tar -czf "$WORK/files.tar.gz" -C "$WORK/src-files" .

DST="$(newdb dt_deploy_restore)"
gunzip -c "$WORK/db.sql.gz" | psql -q -v ON_ERROR_STOP=1 "$DST" >/dev/null
mkdir -p "$WORK/dst-files" && tar -xzf "$WORK/files.tar.gz" -C "$WORK/dst-files"
serve "$DST" 4504 "$WORK/dst-files"
login 4504 owner@backup.test "backup test password"
[ "$(get 4504 "/api/assets/$item" | json 'v.serial+"/"+v.warrantyExpires')" = "RK-77/2030-01-31" ] || fail "restored item"
[ "$(get 4504 "/api/assets/$item/warranties" | json 'v.items.length')" = "1" ] || fail "restored warranty"
get 4504 "/api/attachments/$file" | cmp -s - "$WORK/receipt.pdf" || fail "restored attachment content"
stop
pass "backup and restore: database and attachments round-trip"
echo "deploy tests passed"
