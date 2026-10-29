-- Phase 5: audit types, more frequencies, inventory-scoped audits and a review workflow.
--   draft → open (in progress) → submitted → approved (locked) | rejected → open again
-- Audits that need no review still close directly ('closed', also locked). Nobody approves an
-- audit they started or submitted. Every submit / approve / reject / reopen is kept in
-- audit_reviews, which never changes.

ALTER TABLE audit_runs DROP CONSTRAINT audit_runs_period_check;
ALTER TABLE audit_runs ADD CONSTRAINT audit_runs_period_check
  CHECK (period IN ('weekly', 'monthly', 'quarterly', 'annual', 'manual'));
ALTER TABLE audit_runs DROP CONSTRAINT audit_runs_status_check;
ALTER TABLE audit_runs ADD CONSTRAINT audit_runs_status_check
  CHECK (status IN ('draft', 'open', 'submitted', 'approved', 'rejected', 'closed'));
ALTER TABLE audit_runs DROP CONSTRAINT audit_runs_check1;
ALTER TABLE audit_runs ADD CONSTRAINT audit_runs_final_closed_at
  CHECK ((status IN ('closed', 'approved')) = (closed_at IS NOT NULL));

ALTER TABLE audit_runs
  ADD COLUMN audit_type      text NOT NULL DEFAULT 'full' CHECK (audit_type IN ('soft', 'full', 'spot', 'transfer', 'return')),
  ADD COLUMN requires_review boolean NOT NULL DEFAULT false,
  ADD COLUMN inventory_id    uuid,
  ADD COLUMN submitted_by    uuid,
  ADD COLUMN submitted_at    timestamptz,
  ADD COLUMN reviewed_by     uuid,
  ADD COLUMN reviewed_at     timestamptz,
  ADD COLUMN review_note     text,
  ADD CONSTRAINT audit_runs_inventory_fk FOREIGN KEY (org_id, inventory_id) REFERENCES inventories (org_id, id),
  ADD CONSTRAINT audit_runs_reviewer_not_author CHECK (reviewed_by IS NULL OR (reviewed_by <> started_by AND reviewed_by IS DISTINCT FROM submitted_by));

-- One unfinished audit per period, type and scope.
DROP INDEX audit_runs_one_open;
CREATE UNIQUE INDEX audit_runs_one_open ON audit_runs
  (org_id, period, audit_type, COALESCE(building_id, '00000000-0000-0000-0000-000000000000'::uuid),
   COALESCE(location_id, '00000000-0000-0000-0000-000000000000'::uuid),
   COALESCE(inventory_id, '00000000-0000-0000-0000-000000000000'::uuid))
  WHERE status IN ('draft', 'open', 'submitted', 'rejected') AND audit_type <> 'spot';

ALTER TABLE audit_items
  ADD COLUMN damaged    boolean NOT NULL DEFAULT false,
  ADD COLUMN scan_count integer NOT NULL DEFAULT 0;

CREATE TABLE audit_reviews (
  id         bigserial PRIMARY KEY,
  org_id     uuid NOT NULL,
  run_id     uuid NOT NULL,
  action     text NOT NULL CHECK (action IN ('started', 'submitted', 'approved', 'rejected', 'reopened', 'closed')),
  actor_id   uuid REFERENCES users(id),
  note       text,
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (org_id, run_id) REFERENCES audit_runs (org_id, id)
);
CREATE INDEX audit_reviews_run_idx ON audit_reviews (run_id, id);
CREATE FUNCTION audit_reviews_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'audit_reviews is append-only';
END $$;
CREATE TRIGGER audit_reviews_append_only BEFORE UPDATE OR DELETE ON audit_reviews
  FOR EACH ROW EXECUTE FUNCTION audit_reviews_append_only();

-- Finished audits never change; items change only while the audit is a draft or in progress.
CREATE OR REPLACE FUNCTION audit_closed_is_final() RETURNS trigger AS $$
DECLARE
  run_status text;
BEGIN
  IF TG_TABLE_NAME = 'audit_runs' THEN
    IF OLD.status IN ('closed', 'approved') THEN
      RAISE EXCEPTION 'closed audits cannot be changed';
    END IF;
  ELSE
    SELECT status INTO run_status FROM audit_runs WHERE id = COALESCE(NEW.run_id, OLD.run_id);
    IF run_status IN ('closed', 'approved', 'submitted', 'rejected') THEN
      RAISE EXCEPTION 'closed audits cannot be changed';
    END IF;
  END IF;
  RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;
