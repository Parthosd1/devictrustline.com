-- Phase 5: recorded audit runs. Weekly and monthly audits are independent runs, each with its own
-- snapshot of the assets expected in scope. A closed run and its items can never change again.

CREATE TABLE audit_runs (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id       uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  period       text NOT NULL CHECK (period IN ('weekly', 'monthly')),
  name         text NOT NULL,
  building_id  uuid,
  location_id  uuid,
  status       text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
  due_at       timestamptz NOT NULL,
  started_by   uuid NOT NULL,
  started_at   timestamptz NOT NULL DEFAULT now(),
  closed_by    uuid,
  closed_at    timestamptz,
  summary      jsonb,
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, building_id) REFERENCES buildings (org_id, id),
  FOREIGN KEY (org_id, building_id, location_id) REFERENCES locations (org_id, building_id, id),
  FOREIGN KEY (org_id, started_by) REFERENCES users (org_id, id),
  FOREIGN KEY (org_id, closed_by) REFERENCES users (org_id, id),
  CHECK (location_id IS NULL OR building_id IS NOT NULL),
  CHECK ((status = 'closed') = (closed_at IS NOT NULL))
);
-- One open audit per period and scope at a time.
CREATE UNIQUE INDEX audit_runs_one_open ON audit_runs
  (org_id, period, COALESCE(building_id, '00000000-0000-0000-0000-000000000000'::uuid),
   COALESCE(location_id, '00000000-0000-0000-0000-000000000000'::uuid))
  WHERE status = 'open';
CREATE INDEX audit_runs_org_idx ON audit_runs (org_id, period, started_at DESC);

CREATE TABLE audit_items (
  run_id               uuid NOT NULL,
  org_id               uuid NOT NULL,
  asset_id             uuid NOT NULL,
  -- Where the asset was expected when the audit started. NULL for assets found that were not expected.
  expected             boolean NOT NULL DEFAULT true,
  expected_building_id uuid,
  expected_location_id uuid,
  result               text NOT NULL DEFAULT 'pending'
                       CHECK (result IN ('pending', 'verified', 'wrong_location', 'missing')),
  found_location_id    uuid,
  method               text CHECK (method IN ('scan', 'manual')),
  note                 text,
  checked_by           uuid,
  checked_at           timestamptz,
  PRIMARY KEY (run_id, asset_id),
  FOREIGN KEY (org_id, run_id) REFERENCES audit_runs (org_id, id) ON DELETE CASCADE,
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id),
  FOREIGN KEY (org_id, checked_by) REFERENCES users (org_id, id)
);

CREATE FUNCTION audit_closed_is_final() RETURNS trigger AS $$
DECLARE
  run_status text;
BEGIN
  IF TG_TABLE_NAME = 'audit_runs' THEN
    IF OLD.status = 'closed' THEN
      RAISE EXCEPTION 'closed audits cannot be changed';
    END IF;
  ELSE
    SELECT status INTO run_status FROM audit_runs WHERE id = COALESCE(NEW.run_id, OLD.run_id);
    IF run_status = 'closed' THEN
      RAISE EXCEPTION 'closed audits cannot be changed';
    END IF;
  END IF;
  RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_runs_final BEFORE UPDATE OR DELETE ON audit_runs
  FOR EACH ROW EXECUTE FUNCTION audit_closed_is_final();
CREATE TRIGGER audit_items_final BEFORE INSERT OR UPDATE OR DELETE ON audit_items
  FOR EACH ROW EXECUTE FUNCTION audit_closed_is_final();
