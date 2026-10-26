export const ROLES = ['admin', 'manager', 'technician', 'auditor', 'viewer'];

// Which roles hold each permission. New phases add their permissions here.
const GRANTS = {
  'assets:read': ['admin', 'manager', 'technician', 'auditor', 'viewer'],
  'assets:write': ['admin', 'manager', 'technician'],
  'sites:read': ['admin', 'manager', 'technician', 'auditor', 'viewer'],
  'sites:write': ['admin', 'manager'],
  'people:write': ['admin', 'manager', 'technician'],
  'audits:read': ['admin', 'manager', 'technician', 'auditor', 'viewer'],
  'audits:perform': ['admin', 'manager', 'auditor'],
  'maintenance:read': ['admin', 'manager', 'technician', 'auditor', 'viewer'],
  'maintenance:write': ['admin', 'manager', 'technician'],
  'users:read': ['admin', 'manager'],
  'users:manage': ['admin'],
};

export const PERMISSIONS = Object.keys(GRANTS);

export function can(role, permission) {
  return GRANTS[permission]?.includes(role) ?? false;
}

export function permissionsFor(role) {
  return PERMISSIONS.filter((p) => can(role, p));
}
