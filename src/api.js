// Thin wrapper around fetch for the DeviceTrustline API. The session lives in an httpOnly cookie.
const BASE = import.meta.env.VITE_API_URL ?? '';

export class ApiError extends Error {
  constructor(status, message, details, code) {
    super(message);
    this.status = status;
    this.details = details;
    this.code = code;
  }
}

export async function api(path, { method = 'GET', body, form } = {}) {
  let res;
  try {
    res = await fetch(`${BASE}/api${path}`, {
      method,
      credentials: 'include',
      headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
      // A FormData body (file uploads) sets its own multipart content type.
      body: form ?? (body === undefined ? undefined : JSON.stringify(body)),
    });
  } catch {
    throw new ApiError(0, 'Cannot reach the server. Check your connection and try again.');
  }
  if (res.status === 204) return null;
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    const detail = data.details?.map((d) => (d.path ? `${d.path}: ${d.message}` : d.message)).join('; ');
    throw new ApiError(res.status, detail ? `${data.error}: ${detail}` : data.error || 'Request failed', data.details, data.code);
  }
  return data;
}

export const query = (params) => {
  const s = new URLSearchParams(Object.entries(params).filter(([, v]) => v !== '' && v != null)).toString();
  return s ? `?${s}` : '';
};

export const fileUrl = (id) => `${BASE}/api/attachments/${id}`;
