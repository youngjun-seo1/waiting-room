let baseUrl = localStorage.getItem('wr_url') || 'http://localhost:8080';
let apiKey = localStorage.getItem('wr_api_key') || '';

export function setConnection(url: string, key: string) {
  baseUrl = url.replace(/\/+$/, '');
  apiKey = key;
  localStorage.setItem('wr_url', baseUrl);
  localStorage.setItem('wr_api_key', apiKey);
}

export function getConnection() {
  return { url: baseUrl, apiKey };
}

export function clearConnection() {
  localStorage.removeItem('wr_url');
  localStorage.removeItem('wr_api_key');
  apiKey = '';
}

async function request(path: string, options: RequestInit = {}) {
  const res = await fetch(`${baseUrl}${path}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      'X-Api-Key': apiKey,
      ...options.headers,
    },
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(body || `${res.status} ${res.statusText}`);
  }
  return res.json();
}

export const api = {
  getConfig: () => request('/__wr/admin/config'),
  getStatus: () => request('/__wr/status'),
  getSchedules: () => request('/__wr/admin/schedules'),
  createSchedule: (data: Record<string, unknown>) =>
    request('/__wr/admin/schedules', { method: 'POST', body: JSON.stringify(data) }),
  deleteSchedule: (id: string) =>
    request(`/__wr/admin/schedules/${id}`, { method: 'DELETE' }),
};
