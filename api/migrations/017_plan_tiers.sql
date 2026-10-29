-- Phase 9: four plans. Free Personal and Personal Pro replace Personal Basic and Personal Plus
-- (limits the operator set are carried over); Business gains limits; Enterprise has none.
-- Features are switches a plan includes or not; limits stay numbers (null = no limit).
-- Prices are not stored here: they belong to the billing provider once one is connected.
ALTER TABLE plan_definitions ADD COLUMN features jsonb NOT NULL DEFAULT '{}'::jsonb;

INSERT INTO plan_definitions (code, name, kind, limits, features, position)
SELECT 'free_personal', 'Free Personal', 'personal', limits || '{"reminders": 50}', '{"sms": false, "reports": false}', 1
  FROM plan_definitions WHERE code = 'personal_basic';
INSERT INTO plan_definitions (code, name, kind, limits, features, position)
SELECT 'personal_pro', 'Personal Pro', 'personal', limits || '{"reminders": null}', '{"sms": true, "reports": false}', 2
  FROM plan_definitions WHERE code = 'personal_plus';

UPDATE organizations SET plan = 'free_personal' WHERE plan = 'personal_basic';
UPDATE organizations SET plan = 'personal_pro' WHERE plan = 'personal_plus';
DELETE FROM plan_definitions WHERE code IN ('personal_basic', 'personal_plus');

UPDATE plan_definitions
   SET limits = limits || '{"members": 100, "devices": 500, "inventories": 100, "reminders": null}',
       features = '{"sms": true, "reports": true}', position = 3, updated_at = now()
 WHERE code = 'business';
INSERT INTO plan_definitions (code, name, kind, limits, features, position) VALUES
  ('enterprise', 'Enterprise', 'business',
   '{"items": null, "members": null, "devices": null, "storageMb": null, "inventories": null, "reminders": null}',
   '{"sms": true, "reports": true}', 4);
