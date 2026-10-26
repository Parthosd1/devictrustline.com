// Mirrors ASSET_STATUSES and CONDITIONS in api/src/routes/assets.rs. Categories are per workspace
// (see catalog.jsx), not fixed.
export const CONDITIONS = ['New', 'Good', 'Fair', 'Poor', 'Broken'];
export const ATTACHMENT_KINDS = { photo: 'Photo', receipt: 'Receipt', warranty: 'Warranty document', manual: 'Manual', other: 'Other' };
export const WARRANTY_KINDS = { manufacturer: 'Manufacturer', extended: 'Extended', other: 'Other' };
export const WARRANTY_STATES = { active: 'Active', expiring_soon: 'Expiring soon', expired: 'Expired', unknown: 'Unknown' };
export const ASSET_STATUSES = ['Available', 'Deployed', 'Maintenance', 'Retired'];
export const ROLES = ['admin', 'manager', 'technician', 'auditor', 'viewer'];
export const ROLE_LABELS = {
  admin: 'Admin', manager: 'Manager', technician: 'Technician', auditor: 'Auditor', viewer: 'Viewer',
};
