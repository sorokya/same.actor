import type { ApiErrorBody, FindOutResponse, Mode, SearchResponse, Selection } from './types';

/**
 * A failed request. The API always answers errors as `{"error":{"code","message"}}` with a
 * real status code, so a 429 or a 502 is a value here — never `null` swallowed as "nothing
 * happened", which is what the old client did.
 */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly retryAfterSeconds?: number,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

function isErrorBody(value: unknown): value is ApiErrorBody {
  if (typeof value !== 'object' || value === null) return false;
  const error = (value as { error?: unknown }).error;
  return (
    typeof error === 'object' &&
    error !== null &&
    typeof (error as { message?: unknown }).message === 'string'
  );
}

async function readError(response: Response): Promise<ApiError> {
  let code = 'unknown';
  let message = `Request failed (${response.status}).`;

  try {
    const body: unknown = await response.json();
    if (isErrorBody(body)) {
      code = body.error.code;
      message = body.error.message;
    }
  } catch {
    // A non-JSON body means something in front of the API answered — a proxy error page.
    // Keep the generic message rather than showing the user raw HTML.
  }

  const retryAfter = Number(response.headers.get('retry-after'));
  return new ApiError(
    response.status,
    code,
    message,
    Number.isFinite(retryAfter) && retryAfter > 0 ? retryAfter : undefined,
  );
}

async function request<T>(path: string, init: RequestInit): Promise<T> {
  const response = await fetch(path, init);
  if (!response.ok) throw await readError(response);
  return (await response.json()) as T;
}

export function search(term: string, mode: Mode, signal: AbortSignal): Promise<SearchResponse> {
  const query = new URLSearchParams({ term, mode });
  return request<SearchResponse>(`/api/search?${query}`, {
    signal,
    headers: { accept: 'application/json' },
  });
}

export function findOut(
  mode: Mode,
  selection: readonly Selection[],
  signal: AbortSignal,
): Promise<FindOutResponse> {
  return request<FindOutResponse>('/api/findout', {
    method: 'POST',
    signal,
    headers: { 'content-type': 'application/json', accept: 'application/json' },
    body: JSON.stringify({ mode, selection }),
  });
}

/** Human copy for a failure, without pretending a 500 is the user's fault. */
export function describe(error: unknown): string {
  if (error instanceof ApiError) {
    switch (error.code) {
      case 'rate_limited':
        return error.retryAfterSeconds
          ? `Too many requests. Try again in ${error.retryAfterSeconds}s.`
          : 'Too many requests. Give it a second.';
      case 'upstream_unavailable':
        return 'The movie database is not answering right now. Try again in a moment.';
      case 'not_found':
        return 'That title or person no longer exists upstream.';
      case 'validation_error':
        return error.message;
      default:
        return error.message;
    }
  }

  if (error instanceof DOMException && error.name === 'AbortError') {
    return 'Cancelled.';
  }

  return 'Could not reach the server.';
}
