-- Phase 7: per-organization settings (automatic audits) and marking audits the scheduler started.

ALTER TABLE organizations ADD COLUMN settings jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE audit_runs ADD COLUMN scheduled boolean NOT NULL DEFAULT false;
