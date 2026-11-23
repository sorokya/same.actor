/**
 * The admin dashboard.
 *
 * Loaded as its own chunk: the public bundle is ~38 kB and the visitor never needs any of this.
 * Nothing here holds a credential — the login posts the token once and the server answers with an
 * `HttpOnly` cookie, so a 401 on the first request is the only way to know we are signed out.
 */

import type { ComponentChildren } from 'preact';
import { useCallback, useEffect, useState } from 'preact/hooks';
import { SiteHeader } from '../components/site-header';
import {
  AdminError,
  admin,
  type Ban,
  bytes,
  type ComparisonRow,
  type DayValue,
  type Health,
  type LogFilters,
  type LogRow,
  type Suspicion,
  type Window,
  when,
} from './api';
import { BarList, LineChart, Sparkline } from './charts';

type Tab = 'overview' | 'logs' | 'suspicious' | 'bans' | 'health';

const TABS: { id: Tab; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'logs', label: 'Logs' },
  { id: 'suspicious', label: 'Suspicious' },
  { id: 'bans', label: 'Bans' },
  { id: 'health', label: 'Health' },
];

const WINDOWS: Window[] = ['24h', '7d', '30d'];

export function AdminDashboard() {
  const [auth, setAuth] = useState<'checking' | 'in' | 'out'>('checking');
  const [tab, setTab] = useState<Tab>('overview');
  const [window, setWindow] = useState<Window>('7d');
  /** Set when a suspicious row asks to see its own requests. */
  const [ipFilter, setIpFilter] = useState('');

  const checkAuth = useCallback(async () => {
    try {
      await admin.health();
      setAuth('in');
    } catch (error) {
      if (error instanceof AdminError && error.status === 401) {
        setAuth('out');
        return;
      }
      // A 404 means the server was started without ADMIN_TOKEN. That is not something a login
      // form can fix, and pretending otherwise would send the operator round in circles.
      if (error instanceof AdminError && error.status === 404) {
        setAuth('out');
        return;
      }
      setAuth('out');
    }
  }, []);

  useEffect(() => {
    void checkAuth();
  }, [checkAuth]);

  if (auth === 'checking') {
    return <p className="p-8 font-mono text-xs text-dim">Checking credentials…</p>;
  }

  if (auth === 'out') {
    return <Login onSignedIn={() => setAuth('in')} />;
  }

  return (
    <>
      <SiteHeader width="max-w-5xl">
        <div className="flex min-w-0 flex-wrap items-center justify-end gap-x-4 gap-y-1">
          <nav aria-label="Sections" className="flex flex-wrap items-center gap-0.5">
            {TABS.map((entry) => (
              <button
                key={entry.id}
                type="button"
                onClick={() => setTab(entry.id)}
                aria-current={tab === entry.id ? 'page' : undefined}
                className={`rounded-full px-2.5 py-1 font-mono text-[11px] uppercase tracking-[0.12em] transition-colors ${
                  tab === entry.id
                    ? 'bg-paper text-ink'
                    : 'text-dim hover:bg-raise hover:text-paper'
                }`}
              >
                {entry.label}
              </button>
            ))}
          </nav>

          {/* No text label: "24h 7d 30d" explains itself, and a bare "window" beside the tab row
              read as another tab. */}
          <fieldset className="m-0 flex items-center gap-1 border-0 p-0">
            <legend className="sr-only">Time window</legend>
            {WINDOWS.map((option) => (
              <button
                key={option}
                type="button"
                onClick={() => setWindow(option)}
                aria-pressed={window === option}
                className={`rounded-full border px-2 py-0.5 font-mono text-[11px] ${
                  window === option
                    ? 'border-accent/60 text-accent'
                    : 'border-line text-dim hover:text-paper'
                }`}
              >
                {option}
              </button>
            ))}
          </fieldset>
        </div>
      </SiteHeader>

      <main className="mx-auto w-full max-w-5xl px-4 py-6">
        {tab === 'overview' && <OverviewTab window={window} />}
        {tab === 'logs' && <LogsTab initialIp={ipFilter} window={window} />}
        {tab === 'suspicious' && (
          <SuspiciousTab
            window={window}
            onInspect={(ip) => {
              setIpFilter(ip);
              setTab('logs');
            }}
          />
        )}
        {tab === 'bans' && <BansTab />}
        {tab === 'health' && <HealthTab />}
      </main>
    </>
  );
}

function Login({ onSignedIn }: { onSignedIn: () => void }) {
  const [token, setToken] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  async function submit(event: Event) {
    event.preventDefault();
    setBusy(true);
    setError('');

    try {
      await admin.login(token);
      setToken('');
      onSignedIn();
    } catch (problem) {
      setError(problem instanceof AdminError ? problem.message : 'Could not reach the server.');
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto w-full max-w-sm px-4 py-16">
      <h1 className="text-sm font-medium tracking-tight text-paper">
        same<span className="text-accent">.</span>actor
        <span className="ml-2 label">admin</span>
      </h1>

      <form onSubmit={submit} className="mt-6 space-y-3">
        <label className="block">
          <span className="label">Admin token</span>
          <input
            type="password"
            value={token}
            onInput={(event) => setToken((event.target as HTMLInputElement).value)}
            autoComplete="current-password"
            className="mt-1 w-full rounded border border-line bg-raise px-3 py-2 font-mono text-sm text-paper outline-none focus:border-accent/60"
            placeholder="ADMIN_TOKEN"
          />
        </label>

        {error && (
          <p role="alert" className="text-xs text-danger">
            {error}
          </p>
        )}

        <button
          type="submit"
          disabled={busy || token.length === 0}
          className="w-full rounded bg-paper px-3 py-2 text-sm font-medium text-ink disabled:opacity-40"
        >
          {busy ? 'Signing in…' : 'Sign in'}
        </button>

        <p className="text-[11px] leading-relaxed text-dim">
          The token is exchanged for an <code className="font-mono">HttpOnly</code> cookie and is
          never stored in this page. If you see “Not found”, the server was started without{' '}
          <code className="font-mono">ADMIN_TOKEN</code>.
        </p>
      </form>
    </div>
  );
}

function Panel({
  title,
  children,
  action,
}: {
  title: string;
  children: ComponentChildren;
  action?: preact.ComponentChildren;
}) {
  return (
    <section className="rounded border border-line bg-raise/40 p-4">
      <div className="mb-3 flex items-baseline justify-between gap-2">
        <h2 className="label">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}

function Stat({ label, value, note }: { label: string; value: string; note?: string | undefined }) {
  return (
    <div className="rounded border border-line bg-raise/40 p-3">
      <div className="label">{label}</div>
      <div className="tnum mt-1 text-xl text-paper">{value}</div>
      {note && <div className="mt-0.5 text-[11px] text-dim">{note}</div>}
    </div>
  );
}

function useAsync<T>(load: () => Promise<T>, deps: unknown[]) {
  const [state, setState] = useState<{ data: T | null; error: string | null }>({
    data: null,
    error: null,
  });

  useEffect(() => {
    let live = true;
    setState({ data: null, error: null });

    load().then(
      (data) => live && setState({ data, error: null }),
      (problem) =>
        live &&
        setState({
          data: null,
          error: problem instanceof Error ? problem.message : 'Request failed.',
        }),
    );

    return () => {
      live = false;
    };
  }, deps);

  return state;
}

function OverviewTab({ window }: { window: Window }) {
  const overview = useAsync(() => admin.overview(window), [window]);
  const history = useAsync(() => admin.timeseries('requests', window), [window]);
  const errors = useAsync(() => admin.timeseries('errors', window), [window]);
  const searches = useAsync(() => admin.searches(window), [window]);
  const comparisons = useAsync(() => admin.comparisons(8), []);

  if (overview.error) return <Failure message={overview.error} />;
  if (!overview.data) return <Loading />;

  const { counts, p95_ms, top_paths, top_countries, log_rows, db_bytes } = overview.data;
  const points: DayValue[] = history.data?.points ?? [];
  const errorPoints: DayValue[] = errors.data?.points ?? [];

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat label="Requests" value={counts.requests.toLocaleString()} note={`p95 ${p95_ms} ms`} />
        <Stat label="Visitors" value={counts.visitors.toLocaleString()} note="distinct sessions" />
        <Stat
          label="Searches"
          value={counts.searches.toLocaleString()}
          note={
            counts.searches > 0
              ? `${((counts.searches / Math.max(counts.requests, 1)) * 100).toFixed(0)}% of requests`
              : undefined
          }
        />
        <Stat
          label="Errors"
          value={counts.errors.toLocaleString()}
          note={
            counts.requests > 0
              ? `${((counts.errors / counts.requests) * 100).toFixed(1)}% of requests`
              : undefined
          }
        />
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <div className="rounded border border-line bg-raise/40 p-4">
          <LineChart points={points} label="Requests per day" />
          <div className="mt-4">
            <Sparkline points={errorPoints} />
            <p className="mt-1 text-[11px] text-dim">
              Errors over the same window —{' '}
              {errorPoints.reduce((sum, point) => sum + point.value, 0).toLocaleString()} total.
            </p>
          </div>
        </div>

        <div className="space-y-4">
          <Panel title="Top paths">
            <BarList rows={top_paths.map((row) => ({ label: row.path, value: row.requests }))} />
          </Panel>
          <Panel title="Countries">
            <BarList
              rows={top_countries.map((row) => ({
                label: row.country,
                value: row.requests,
                note: row.errors > 0 ? `(${row.errors} err)` : undefined,
              }))}
              empty="No country data yet — the geo database may still be downloading."
            />
          </Panel>
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <Panel title="Searches">
          <BarList
            rows={(searches.data?.top ?? []).slice(0, 8).map((term) => ({
              label: term.term,
              value: term.searches,
            }))}
            empty="Nothing searched yet."
          />
          <div className="mt-4">
            <div className="label mb-2">Searched and not found</div>
            <BarList
              rows={(searches.data?.zero_results ?? []).slice(0, 6).map((term) => ({
                label: term.term,
                value: term.searches,
              }))}
              empty="Nothing came up empty — either good coverage or nobody looking."
            />
          </div>
        </Panel>

        <Panel title="Most viewed comparisons">
          <ul className="space-y-1.5">
            {(comparisons.data ?? []).map((row: ComparisonRow) => (
              <li key={row.key} className="flex items-baseline justify-between gap-3">
                <a
                  href={`/${row.mode === 'person' ? 'person' : 'title'}/${row.key.split(':')[1]}`}
                  className="truncate text-xs text-paper hover:text-accent"
                >
                  {row.label}
                </a>
                <span className="tnum shrink-0 text-[11px] text-dim">{row.views}</span>
              </li>
            ))}
            {(comparisons.data ?? []).length === 0 && (
              <li className="text-xs text-dim">Nothing viewed yet.</li>
            )}
          </ul>
        </Panel>
      </div>

      <p className="text-[11px] text-dim">
        {log_rows.toLocaleString()} log rows retained at full detail · database {bytes(db_bytes)}
      </p>
    </div>
  );
}

/** The window control drives the log range too. One time control, not two. */
function sinceFor(window: Window): number {
  const seconds = window === '24h' ? 86_400 : window === '7d' ? 604_800 : 2_592_000;
  return Math.round(Date.now() / 1000 - seconds);
}

function LogsTab({ initialIp, window }: { initialIp: string; window: Window }) {
  const [filters, setFilters] = useState<LogFilters>({
    limit: 50,
    ip: initialIp,
    since: sinceFor(window),
  });

  // Following the window control keeps the table and the header in agreement.
  useEffect(() => {
    setFilters((current) => ({ ...current, since: sinceFor(window), before_id: undefined }));
  }, [window]);
  const [rows, setRows] = useState<LogRow[]>([]);
  const [cursor, setCursor] = useState<number | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (next: LogFilters, append: boolean) => {
    setBusy(true);
    setError('');
    try {
      const page = await admin.logs(next);
      setRows((current) => (append ? [...current, ...page.rows] : page.rows));
      setCursor(page.next_before_id);
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Request failed.');
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load(filters, false);
    // Only re-run when the filter object changes, which is every applied edit.
  }, [filters, load]);

  function update(patch: Partial<LogFilters>) {
    setFilters((current) => ({ ...current, ...patch, before_id: undefined }));
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end gap-2 rounded border border-line bg-raise/40 p-3">
        <Field label="kind" htmlFor="filter-kind">
          <select
            id="filter-kind"
            value={filters.kind ?? ''}
            onChange={(event) =>
              update({ kind: (event.target as HTMLSelectElement).value || undefined })
            }
            className="rounded border border-line bg-ink px-2 py-1 text-xs text-paper"
          >
            <option value="">all</option>
            {['api', 'page', 'card', 'admin', 'asset'].map((kind) => (
              <option key={kind} value={kind}>
                {kind}
              </option>
            ))}
          </select>
        </Field>

        <Field label="status" htmlFor="filter-status">
          <TextInput
            id="filter-status"
            value={filters.status ?? ''}
            placeholder="404 or 4xx"
            onInput={(value) => update({ status: value || undefined })}
          />
        </Field>

        <Field label="ip" htmlFor="filter-ip">
          <TextInput
            id="filter-ip"
            value={filters.ip ?? ''}
            placeholder="203.0.113.7"
            onInput={(value) => update({ ip: value || undefined })}
          />
        </Field>

        <Field label="search" htmlFor="filter-q">
          <TextInput
            id="filter-q"
            value={filters.q ?? ''}
            placeholder="path, message, user agent"
            onInput={(value) => update({ q: value || undefined })}
          />
        </Field>
      </div>

      {error && <Failure message={error} />}

      <div className="overflow-x-auto rounded border border-line">
        <table className="w-full border-collapse text-left">
          <thead>
            <tr className="border-b border-line text-dim">
              <Th>When</Th>
              <Th>Status</Th>
              <Th>Kind</Th>
              <Th>Path</Th>
              <Th>IP</Th>
              <Th>Country</Th>
              <Th>ms</Th>
              <Th>Detail</Th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.id} className="border-b border-line-soft align-top">
                <Td className="whitespace-nowrap font-mono text-[11px] text-dim">{when(row.ts)}</Td>
                <Td>
                  <StatusPill status={row.status} />
                </Td>
                <Td className="font-mono text-[11px] text-dim">{row.kind}</Td>
                <Td className="max-w-[22rem] truncate font-mono text-[11px] text-paper">
                  {row.path}
                </Td>
                <Td className="font-mono text-[11px] text-dim">{row.ip ?? '—'}</Td>
                <Td className="font-mono text-[11px] text-dim">{row.country ?? '—'}</Td>
                <Td className="tnum text-[11px] text-dim">{row.duration_ms}</Td>
                <Td className="max-w-[18rem] font-mono text-[11px] text-danger/90">
                  {row.code ? (
                    <span title={row.message ?? ''}>
                      {row.code}
                      {row.message ? ` — ${row.message}` : ''}
                    </span>
                  ) : (
                    ''
                  )}
                </Td>
              </tr>
            ))}
            {rows.length === 0 && !busy && (
              <tr>
                <Td className="text-xs text-dim">Nothing matches those filters.</Td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {cursor !== null && (
        <button
          type="button"
          onClick={() => void load({ ...filters, before_id: cursor }, true)}
          className="rounded border border-line px-3 py-1.5 text-xs text-dim hover:text-paper"
        >
          Load older
        </button>
      )}
    </div>
  );
}

function SuspiciousTab({ window, onInspect }: { window: Window; onInspect: (ip: string) => void }) {
  const { data, error } = useAsync(() => admin.suspicious(window), [window]);

  if (error) return <Failure message={error} />;
  if (!data) return <Loading />;

  return (
    <div className="space-y-3">
      <p className="text-[11px] leading-relaxed text-dim">
        Scored from volume against the median address, failure rate, id walking, missing user agents
        and rate-limit refusals. Nothing here bans anyone on its own — that is the automatic rules,
        and they are off unless configured.
      </p>

      {data.length === 0 ? (
        <p className="text-xs text-dim">Nothing looks suspicious in this window.</p>
      ) : (
        <div className="overflow-x-auto rounded border border-line">
          <table className="w-full border-collapse text-left">
            <thead>
              <tr className="border-b border-line text-dim">
                <Th>Score</Th>
                <Th>IP</Th>
                <Th>Requests</Th>
                <Th>Errors</Th>
                <Th>Paths</Th>
                <Th>Why</Th>
                <Th />
              </tr>
            </thead>
            <tbody>
              {data.map((row: Suspicion) => (
                <tr key={row.ip} className="border-b border-line-soft align-top">
                  <Td>
                    <span className="tnum text-sm text-danger">{row.score}</span>
                  </Td>
                  <Td className="font-mono text-[11px] text-paper">{row.ip}</Td>
                  <Td className="tnum text-[11px] text-dim">{row.requests}</Td>
                  <Td className="tnum text-[11px] text-dim">{row.errors}</Td>
                  <Td className="tnum text-[11px] text-dim">{row.paths}</Td>
                  <Td>
                    <ul className="space-y-0.5">
                      {row.reasons.map((reason) => (
                        <li key={reason} className="text-[11px] text-dim">
                          {reason}
                        </li>
                      ))}
                    </ul>
                  </Td>
                  <Td>
                    <button
                      type="button"
                      onClick={() => onInspect(row.ip)}
                      className="whitespace-nowrap rounded border border-line px-2 py-0.5 text-[11px] text-dim hover:text-paper"
                    >
                      View logs
                    </button>
                  </Td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function BansTab() {
  const [bans, setBans] = useState<Ban[]>([]);
  const [target, setTarget] = useState('');
  const [reason, setReason] = useState('');
  const [duration, setDuration] = useState('3600');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setBans(await admin.bans());
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Request failed.');
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function create(event: Event) {
    event.preventDefault();
    setBusy(true);
    setError('');

    try {
      const seconds = duration === '0' ? undefined : Number(duration);
      await admin.createBan(target, reason, seconds);
      setTarget('');
      setReason('');
      await refresh();
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Request failed.');
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: number) {
    setError('');
    try {
      await admin.deleteBan(id);
      await refresh();
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : 'Request failed.');
    }
  }

  const now = Math.floor(Date.now() / 1000);

  return (
    <div className="space-y-4">
      <form
        onSubmit={create}
        className="flex flex-wrap items-end gap-2 rounded border border-line bg-raise/40 p-3"
      >
        <Field label="address or range" htmlFor="ban-target">
          <TextInput
            id="ban-target"
            value={target}
            placeholder="203.0.113.7 or 203.0.113.0/24"
            onInput={setTarget}
          />
        </Field>
        <Field label="reason" htmlFor="ban-reason">
          <TextInput id="ban-reason" value={reason} placeholder="optional" onInput={setReason} />
        </Field>
        <Field label="duration" htmlFor="ban-duration">
          <select
            id="ban-duration"
            value={duration}
            onChange={(event) => setDuration((event.target as HTMLSelectElement).value)}
            className="rounded border border-line bg-ink px-2 py-1 text-xs text-paper"
          >
            <option value="3600">1 hour</option>
            <option value="86400">1 day</option>
            <option value="604800">1 week</option>
            <option value="0">permanent</option>
          </select>
        </Field>
        <button
          type="submit"
          disabled={busy || target.trim().length === 0}
          className="rounded bg-paper px-3 py-1.5 text-xs font-medium text-ink disabled:opacity-40"
        >
          Ban
        </button>
        {error && <span className="pb-1 text-[11px] text-danger">{error}</span>}
      </form>

      <div className="overflow-x-auto rounded border border-line">
        <table className="w-full border-collapse text-left">
          <thead>
            <tr className="border-b border-line text-dim">
              <Th>Target</Th>
              <Th>Kind</Th>
              <Th>Reason</Th>
              <Th>By</Th>
              <Th>Expires</Th>
              <Th>Hits</Th>
              <Th />
            </tr>
          </thead>
          <tbody>
            {bans.map((ban) => (
              <tr key={ban.id} className="border-b border-line-soft">
                <Td className="font-mono text-[11px] text-paper">{ban.target}</Td>
                <Td className="font-mono text-[11px] text-dim">{ban.kind}</Td>
                <Td className="text-[11px] text-dim">{ban.reason ?? '—'}</Td>
                <Td className="font-mono text-[11px] text-dim">{ban.created_by}</Td>
                <Td className="text-[11px] text-dim">
                  {ban.expires_ts === null
                    ? 'never'
                    : ban.expires_ts <= now
                      ? 'expired'
                      : when(ban.expires_ts)}
                </Td>
                <Td className="tnum text-[11px] text-dim">{ban.hits}</Td>
                <Td>
                  <button
                    type="button"
                    onClick={() => void remove(ban.id)}
                    className="rounded border border-line px-2 py-0.5 text-[11px] text-dim hover:text-danger"
                  >
                    Remove
                  </button>
                </Td>
              </tr>
            ))}
            {bans.length === 0 && (
              <tr>
                <Td className="text-xs text-dim">No bans.</Td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function HealthTab() {
  const { data, error } = useAsync(() => admin.health(), []);

  if (error) return <Failure message={error} />;
  if (!data) return <Loading />;

  const health: Health = data;
  const perRow = health.commits > 0 ? health.rows_written / health.commits : 0;

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat label="Database" value={bytes(health.db_bytes)} note="including sidecars" />
        <Stat
          label="Log rows"
          value={health.log_rows.toLocaleString()}
          note={`${health.search_rows.toLocaleString()} searches`}
        />
        <Stat label="Bans" value={String(health.bans)} />
        <Stat
          label="Dropped"
          value={health.dropped.toLocaleString()}
          note="log records lost to a full queue"
        />
      </div>

      <Panel title="Writer">
        <dl className="grid grid-cols-2 gap-2 text-xs sm:grid-cols-3">
          <Fact label="Rows written" value={health.rows_written.toLocaleString()} />
          <Fact label="Transactions" value={health.commits.toLocaleString()} />
          <Fact label="Rows per transaction" value={perRow.toFixed(1)} />
        </dl>
        <p className="mt-3 text-[11px] leading-relaxed text-dim">
          One writer thread, batched. Rows per transaction well above 1 means batching is working; a
          dropped count above zero means the queue filled and records were discarded rather than
          slowing a request.
        </p>
      </Panel>

      <Panel title="Country lookup">
        <p className="text-xs text-dim">
          {!health.geo_enabled
            ? 'Disabled (GEO_ENABLED=false).'
            : health.geo_age_secs === null
              ? 'No database loaded — the download has not succeeded yet.'
              : `Database loaded ${Math.round((health.geo_age_secs ?? 0) / 3600)}h ago.`}
        </p>
      </Panel>
    </div>
  );
}

/**
 * A labelled control. `htmlFor` is explicit rather than relying on nesting: a label wrapping an
 * arbitrary subtree cannot be statically checked, and an unassociated label is invisible to a
 * screen reader.
 */
function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: ComponentChildren;
}) {
  return (
    <div className="block">
      <label htmlFor={htmlFor} className="label">
        {label}
      </label>
      <div className="mt-0.5">{children}</div>
    </div>
  );
}

function TextInput({
  id,
  value,
  placeholder,
  onInput,
}: {
  id: string;
  value: string;
  placeholder?: string;
  onInput: (value: string) => void;
}) {
  return (
    <input
      id={id}
      type="text"
      value={value}
      placeholder={placeholder}
      onInput={(event) => onInput((event.target as HTMLInputElement).value)}
      className="w-44 rounded border border-line bg-ink px-2 py-1 font-mono text-[11px] text-paper outline-none placeholder:text-dim/60 focus:border-accent/60"
    />
  );
}

function Th({ children }: { children?: preact.ComponentChildren }) {
  return <th className="label px-2 py-1.5 font-normal">{children}</th>;
}

function Td({ children, className = '' }: { children?: ComponentChildren; className?: string }) {
  return <td className={`px-2 py-1.5 ${className}`}>{children}</td>;
}

function StatusPill({ status }: { status: number }) {
  const tone =
    status >= 500
      ? 'border-danger/50 text-danger'
      : status >= 400
        ? 'border-accent/40 text-accent'
        : 'border-line text-dim';

  return (
    <span className={`tnum rounded border px-1.5 py-0.5 font-mono text-[10px] ${tone}`}>
      {status}
    </span>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="label">{label}</dt>
      <dd className="tnum text-sm text-paper">{value}</dd>
    </div>
  );
}

function Loading() {
  return <p className="py-8 font-mono text-xs text-dim">Loading…</p>;
}

function Failure({ message }: { message: string }) {
  return (
    <p role="alert" className="rounded border border-danger/40 bg-raise/40 p-3 text-xs text-danger">
      {message}
    </p>
  );
}
