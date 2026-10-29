// Date display shared by the pages. Plain dates (2026-11-05) are shown in local time, not UTC.
export const fmtDate = (d) => (d ? new Date(d.length === 10 ? `${d}T00:00:00` : d).toLocaleDateString() : '—');
export const fmtDateTime = (d) => (d ? new Date(d).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : '—');
export const today = () => new Date().toLocaleDateString('en-CA');
export const addDays = (n) => { const d = new Date(); d.setDate(d.getDate() + n); return d.toLocaleDateString('en-CA'); };
export const plural = (n, word) => `${n} ${word}${n === 1 ? '' : 's'}`;
