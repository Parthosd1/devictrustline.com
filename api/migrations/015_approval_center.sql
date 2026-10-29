-- Phase 6: follow-up tasks raised from the approval center (a missing item to trace, a damaged
-- return to repair, a discrepancy to correct). A task can point at the item and at what raised it.

CREATE TABLE follow_up_tasks (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id       uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  title        text NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 200),
  notes        text,
  kind         text NOT NULL DEFAULT 'general'
               CHECK (kind IN ('general', 'missing', 'damaged', 'discrepancy', 'overdue_loan', 'warranty', 'maintenance', 'audit', 'transfer')),
  -- What raised it: an audit, transfer, loan or work order id, kept as a reference.
  source_type  text CHECK (source_type IN ('audit', 'transfer', 'loan', 'work_order', 'warranty', 'access_request')),
  source_id    uuid,
  asset_id     uuid,
  assignee_id  uuid,
  due_on       date,
  status       text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'done', 'cancelled')),
  resolution   text,
  created_by   uuid NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  closed_by    uuid,
  closed_at    timestamptz,
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id),
  FOREIGN KEY (org_id, assignee_id) REFERENCES memberships (org_id, user_id),
  FOREIGN KEY (org_id, created_by) REFERENCES memberships (org_id, user_id),
  CHECK ((status = 'open') = (closed_at IS NULL)),
  CHECK ((source_type IS NULL) = (source_id IS NULL))
);
CREATE INDEX follow_up_tasks_org_idx ON follow_up_tasks (org_id, status, due_on);
-- One open task per thing that raised it, so the same discrepancy is not raised twice.
CREATE UNIQUE INDEX follow_up_tasks_one_open ON follow_up_tasks (org_id, source_type, source_id, COALESCE(asset_id, '00000000-0000-0000-0000-000000000000'::uuid))
  WHERE status = 'open' AND source_id IS NOT NULL;
