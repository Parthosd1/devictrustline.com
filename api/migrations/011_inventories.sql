-- Phase 2: hierarchical inventories (a hub and its sub-inventories) and transfers between them.
-- An item sits in at most one inventory (a single column), and every move is a transfer row
-- recording who moved it, from where, to where, when and why. Transfers can need approval.

CREATE TABLE inventories (
  id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id            uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  parent_id         uuid,
  name              text NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
  kind              text NOT NULL DEFAULT 'general'
                    CHECK (kind IN ('hub', 'general', 'available', 'loaner', 'repair', 'deployment', 'returned', 'technician', 'site')),
  description       text,
  building_id       uuid,
  location_id       uuid,
  -- The person who holds what is in it (a technician's own stock, for example).
  custodian_id      uuid,
  -- Moving items into or out of this inventory needs a manager's approval.
  requires_approval boolean NOT NULL DEFAULT false,
  archived_at       timestamptz,
  created_by        uuid REFERENCES users(id),
  created_at        timestamptz NOT NULL DEFAULT now(),
  updated_at        timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, parent_id) REFERENCES inventories (org_id, id),
  FOREIGN KEY (org_id, building_id) REFERENCES buildings (org_id, id),
  FOREIGN KEY (org_id, building_id, location_id) REFERENCES locations (org_id, building_id, id),
  FOREIGN KEY (org_id, custodian_id) REFERENCES memberships (org_id, user_id),
  CHECK (parent_id IS NULL OR parent_id <> id),
  CHECK (location_id IS NULL OR building_id IS NOT NULL)
);
CREATE UNIQUE INDEX inventories_name_uniq ON inventories
  (org_id, COALESCE(parent_id, '00000000-0000-0000-0000-000000000000'::uuid), lower(name)) WHERE archived_at IS NULL;
CREATE INDEX inventories_parent_idx ON inventories (org_id, parent_id);

ALTER TABLE assets
  ADD COLUMN inventory_id uuid,
  ADD CONSTRAINT assets_inventory_fk FOREIGN KEY (org_id, inventory_id) REFERENCES inventories (org_id, id);
CREATE INDEX assets_inventory_idx ON assets (org_id, inventory_id);

CREATE TABLE inventory_transfers (
  id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id            uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  -- Transfers made together (a bulk move, or a parent and its components) share a batch.
  batch_id          uuid NOT NULL,
  asset_id          uuid NOT NULL,
  from_inventory_id uuid,
  to_inventory_id   uuid NOT NULL,
  -- Set when the item moved because its parent did.
  via_asset_id      uuid,
  status            text NOT NULL CHECK (status IN ('pending', 'completed', 'rejected', 'cancelled')),
  method            text NOT NULL DEFAULT 'manual' CHECK (method IN ('manual', 'scan', 'bulk', 'system')),
  reason            text,
  requested_by      uuid NOT NULL,
  requested_at      timestamptz NOT NULL DEFAULT now(),
  decided_by        uuid,
  decided_at        timestamptz,
  decision_note     text,
  completed_at      timestamptz,
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id),
  FOREIGN KEY (org_id, from_inventory_id) REFERENCES inventories (org_id, id),
  FOREIGN KEY (org_id, to_inventory_id) REFERENCES inventories (org_id, id),
  FOREIGN KEY (org_id, via_asset_id) REFERENCES assets (org_id, id),
  FOREIGN KEY (org_id, requested_by) REFERENCES memberships (org_id, user_id),
  FOREIGN KEY (org_id, decided_by) REFERENCES memberships (org_id, user_id),
  CHECK (from_inventory_id IS DISTINCT FROM to_inventory_id),
  CHECK ((status = 'completed') = (completed_at IS NOT NULL)),
  CHECK (status = 'pending' OR decided_at IS NOT NULL)
);
-- At most one open request per item, so two people cannot move it to two places.
CREATE UNIQUE INDEX inventory_transfers_one_pending ON inventory_transfers (asset_id) WHERE status = 'pending';
CREATE INDEX inventory_transfers_org_idx ON inventory_transfers (org_id, status, requested_at DESC);
CREATE INDEX inventory_transfers_asset_idx ON inventory_transfers (asset_id, requested_at DESC);
CREATE INDEX inventory_transfers_inventory_idx ON inventory_transfers (org_id, to_inventory_id);

-- Transfer history is permanent: a decided transfer never changes and no transfer is deleted.
CREATE FUNCTION inventory_transfers_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'inventory_transfers is append-only';
  END IF;
  IF OLD.status <> 'pending' THEN
    RAISE EXCEPTION 'a decided transfer cannot change';
  END IF;
  IF NEW.asset_id <> OLD.asset_id OR NEW.to_inventory_id <> OLD.to_inventory_id
     OR NEW.from_inventory_id IS DISTINCT FROM OLD.from_inventory_id OR NEW.requested_by <> OLD.requested_by THEN
    RAISE EXCEPTION 'a transfer request cannot be rewritten';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER inventory_transfers_final BEFORE UPDATE OR DELETE ON inventory_transfers
  FOR EACH ROW EXECUTE FUNCTION inventory_transfers_final();

-- Business workspaces start with a hub and the usual sub-inventories. Existing items stay
-- unassigned until someone moves them.
CREATE FUNCTION seed_inventories(org uuid) RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  hub uuid;
BEGIN
  INSERT INTO inventories (org_id, name, kind) VALUES (org, 'Main IT Hub', 'hub') RETURNING id INTO hub;
  INSERT INTO inventories (org_id, parent_id, name, kind)
  VALUES (org, hub, 'Available Inventory', 'available'),
         (org, hub, 'Loaner Inventory', 'loaner'),
         (org, hub, 'RMA / Repair Inventory', 'repair'),
         (org, hub, 'Deployment Inventory', 'deployment'),
         (org, hub, 'Returned Equipment', 'returned');
END $$;
SELECT seed_inventories(id) FROM organizations WHERE kind = 'business';
