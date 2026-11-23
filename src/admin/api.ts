/**
 * Typed access to `/api/admin`.
 *
 * The token itself never lives here: the login posts it once and the server answers with an
 * `HttpOnly` cookie, so there is nothing for this module — or an injected script — to read back.
 */

export class AdminError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = 'AdminError';
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { accept: 'application/json', ...(init?.headers ?? {}) },
  });

  if (!response.ok) {
    let code = 'unknown';
    let message = `Request failed (${response.status}).`;
    try {
      const body = await response.json();
      if (typeof body?.error?.message === 'string') {
        code = body.error.code ?? code;
        message = body.error.message;
      }
    } catch {
      // A non-JSON body means a proxy answered, not the API.
    }
    throw new AdminError(response.status, code, message);
  }

  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export interface Counts {
  requests: number;
  errors: number;
  visitors: number;
  searches: number;
}

export interface Overview {
  window: string;
  counts: Counts;
  p95_ms: number;
  top_paths: { path: string; requests: number }[];
  top_countries: { country: string; requests: number; errors: number }[];
  log_rows: number;
  db_bytes: number;
}

export interface DayValue {
  day: string;
  value: number;
}

export interface LogRow {
  id: number;
  ts: number;
  duration_ms: number;
  ip: string | null;
  session: string | null;
  ua: string | null;
  method: string;
  path: string;
  status: number;
  code: string | null;
  message: string | null;
  kind: string;
  country: string | null;
}

export interface LogPage {
  rows: LogRow[];
  next_before_id: number | null;
}

export interface Suspicion {
  ip: string;
  requests: number;
  errors: number;
  limited: number;
  paths: number;
  sessions: number;
  score: number;
  reasons: string[];
  first_seen: number;
  last_seen: number;
}

export interface Ban {
  id: number;
  created_ts: number;
  expires_ts: number | null;
  target: string;
  kind: string;
  reason: string | null;
  created_by: string;
  hits: number;
  last_hit: number | null;
}

export interface Health {
  db_bytes: number;
  log_rows: number;
  search_rows: number;
  bans: number;
  dropped: number;
  rows_written: number;
  commits: number;
  geo_age_secs: number | null;
  geo_enabled: boolean;
}

export interface Term {
  term: string;
  searches: number;
}

export interface SearchReport {
  window: string;
  top: Term[];
  zero_results: Term[];
}

export interface ComparisonRow {
  key: string;
  label: string;
  mode: string;
  views: number;
  last_seen: number;
}

export type Metric = 'requests' | 'errors' | 'visitors' | 'searches';
export type Window = '24h' | '7d' | '30d';

/**
 * `| undefined` on every field because `exactOptionalPropertyTypes` is on: clearing a filter
 * means assigning `undefined`, and a bare `kind?: string` does not permit that.
 */
export interface LogFilters {
  kind?: string | undefined;
  status?: string | undefined;
  ip?: string | undefined;
  q?: string | undefined;
  since?: number | undefined;
  limit?: number | undefined;
  before_id?: number | undefined;
}

export const admin = {
  async login(token: string): Promise<void> {
    await request<{ ok: boolean }>('/api/admin/session', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token }),
    });
  },

  overview: (window: Window) => request<Overview>(`/api/admin/overview?window=${window}`),

  logs: (filters: LogFilters) => {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(filters)) {
      if (value !== undefined && value !== '') query.set(key, String(value));
    }
    return request<LogPage>(`/api/admin/logs?${query}`);
  },

  timeseries: (metric: Metric, window: Window) =>
    request<{ points: DayValue[] }>(
      `/api/admin/stats/timeseries?metric=${metric}&window=${window}`,
    ),

  geo: (days: number) =>
    request<{ countries: Overview['top_countries'] }>(`/api/admin/stats/geo?days=${days}`),

  searches: (window: Window) => request<SearchReport>(`/api/admin/stats/searches?window=${window}`),

  comparisons: (limit = 10) =>
    request<ComparisonRow[]>(`/api/admin/stats/comparisons?limit=${limit}`),

  suspicious: (window: Window) => request<Suspicion[]>(`/api/admin/suspicious?window=${window}`),

  bans: () => request<Ban[]>('/api/admin/bans'),

  createBan: (target: string, reason: string, expiresInSecs?: number) =>
    request<{ id: number }>('/api/admin/bans', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        target,
        reason: reason.trim() || undefined,
        expires_in_secs: expiresInSecs,
      }),
    }),

  deleteBan: (id: number) => request<void>(`/api/admin/bans/${id}`, { method: 'DELETE' }),

  health: () => request<Health>('/api/admin/health'),
};

/** Bytes as something a human reads without counting zeros. */
export function bytes(value: number): string {
  const units = ['B', 'kB', 'MB', 'GB'];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size < 10 && unit > 0 ? size.toFixed(1) : Math.round(size)} ${units[unit]}`;
}

export function when(unix: number): string {
  const date = new Date(unix * 1000);
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}
