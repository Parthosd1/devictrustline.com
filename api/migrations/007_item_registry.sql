-- Release 1: a flexible item registry. Categories are per workspace (no fixed list), items get
-- tags, workspace-defined custom fields, purchase and ownership details, and private attachments.

CREATE TABLE categories (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id     uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name       text NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 80),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id)
);
CREATE UNIQUE INDEX categories_org_name_uniq ON categories (org_id, lower(name));

-- Existing workspaces keep every category their items already use, plus the old built-in list.
INSERT INTO categories (org_id, name)
SELECT o.id, t.name
  FROM organizations o
 CROSS JOIN unnest(ARRAY['Laptop', 'Desktop', 'Workstation', 'Monitor', 'Scanner', 'Printer', 'Docking Station',
                         'Phone', 'Tablet', 'Network', 'Accessory', 'Other']) AS t(name)
UNION
SELECT DISTINCT org_id, type FROM assets
ON CONFLICT DO NOTHING;

ALTER TABLE assets
  ADD COLUMN category_id     uuid,
  ADD COLUMN condition       text CHECK (condition IN ('New', 'Good', 'Fair', 'Poor', 'Broken')),
  ADD COLUMN owner_id        uuid,
  ADD COLUMN purchase_price  numeric(12, 2) CHECK (purchase_price >= 0),
  ADD COLUMN currency        text CHECK (currency ~ '^[A-Z]{3}$'),
  ADD COLUMN seller          text,
  ADD COLUMN return_deadline date,
  ADD COLUMN maintenance_due date,
  ADD COLUMN custom          jsonb NOT NULL DEFAULT '{}'::jsonb,
  ADD CONSTRAINT assets_category_fk FOREIGN KEY (org_id, category_id) REFERENCES categories (org_id, id),
  ADD CONSTRAINT assets_owner_fk FOREIGN KEY (org_id, owner_id) REFERENCES memberships (org_id, user_id);

-- `type` stays the category's name (what lists, reports and exports show); `category_id` is kept
-- in step by this trigger, which also refuses a category the workspace does not have.
CREATE FUNCTION assets_set_category() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  SELECT id, name INTO NEW.category_id, NEW.type
    FROM categories WHERE org_id = NEW.org_id AND lower(name) = lower(NEW.type);
  IF NEW.category_id IS NULL THEN
    RAISE EXCEPTION 'unknown category "%"', NEW.type USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER assets_set_category BEFORE INSERT OR UPDATE OF type ON assets
  FOR EACH ROW EXECUTE FUNCTION assets_set_category();
UPDATE assets SET type = type;
ALTER TABLE assets ALTER COLUMN category_id SET NOT NULL;
CREATE INDEX assets_category_idx ON assets (org_id, category_id);
CREATE INDEX assets_owner_idx ON assets (owner_id) WHERE owner_id IS NOT NULL;

CREATE TABLE tags (
  id     uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name   text NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 40),
  UNIQUE (org_id, id)
);
CREATE UNIQUE INDEX tags_org_name_uniq ON tags (org_id, lower(name));

CREATE TABLE asset_tags (
  org_id   uuid NOT NULL,
  asset_id uuid NOT NULL,
  tag_id   uuid NOT NULL,
  PRIMARY KEY (asset_id, tag_id),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id) ON DELETE CASCADE,
  FOREIGN KEY (org_id, tag_id) REFERENCES tags (org_id, id) ON DELETE CASCADE
);
CREATE INDEX asset_tags_tag_idx ON asset_tags (tag_id);

-- Fields a workspace adds to its items. Values live in assets.custom, keyed by field id.
CREATE TABLE custom_fields (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id      uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  label       text NOT NULL CHECK (length(trim(label)) BETWEEN 1 AND 80),
  field_type  text NOT NULL CHECK (field_type IN ('text', 'number', 'date', 'boolean', 'select')),
  options     jsonb NOT NULL DEFAULT '[]'::jsonb,
  -- Only shown for items in this category, when set.
  category_id uuid,
  position    integer NOT NULL DEFAULT 0,
  created_at  timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (org_id, category_id) REFERENCES categories (org_id, id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX custom_fields_org_label_uniq ON custom_fields (org_id, lower(label));

-- Photos, receipts and documents. Files are stored privately on disk (ATTACHMENTS_DIR), never
-- served directly: every download goes through the API's access check.
CREATE TABLE attachments (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id       uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  asset_id     uuid NOT NULL,
  kind         text NOT NULL CHECK (kind IN ('photo', 'receipt', 'warranty', 'manual', 'other')),
  filename     text NOT NULL,
  content_type text NOT NULL,
  size_bytes   bigint NOT NULL CHECK (size_bytes > 0),
  sha256       bytea NOT NULL,
  uploaded_by  uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, id),
  FOREIGN KEY (org_id, asset_id) REFERENCES assets (org_id, id) ON DELETE CASCADE
);
CREATE INDEX attachments_asset_idx ON attachments (org_id, asset_id);
