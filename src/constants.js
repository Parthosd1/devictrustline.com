// Mirrors ASSET_TYPES and ASSET_STATUSES in server/src/routes/assets.js.
export const ASSET_TYPES = [
  'Laptop', 'Desktop', 'Workstation', 'Monitor', 'Scanner', 'Printer', 'Docking Station',
  'Phone', 'Tablet', 'Network', 'Accessory', 'Other',
];
export const ASSET_STATUSES = ['Available', 'Deployed', 'Maintenance', 'Retired'];
export const ROLES = ['admin', 'manager', 'technician', 'auditor', 'viewer'];
export const ROLE_LABELS = {
  admin: 'Admin', manager: 'Manager', technician: 'Technician', auditor: 'Auditor', viewer: 'Viewer',
};
