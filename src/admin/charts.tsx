/**
 * Charts, as SVG.
 *
 * No chart library: the whole app is ~38 kB and Chart.js alone would triple it to draw four
 * small charts. `og.rs` already generates SVG on the server for share cards; this is the same
 * idea in the browser.
 *
 * `vector-effect="non-scaling-stroke"` is what lets a fixed viewBox stretch to any width without
 * the line getting fat on a wide screen.
 */

import type { DayValue } from './api';

interface ChartProps {
  points: DayValue[];
  height?: number;
  label?: string;
}

const WIDTH = 600;

function path(values: number[], width: number, height: number, pad: number): string {
  if (values.length === 0) return '';

  const max = Math.max(...values, 1);
  const step = values.length > 1 ? (width - pad * 2) / (values.length - 1) : 0;

  return values
    .map((value, index) => {
      const x = pad + index * step;
      // Inverted: SVG's origin is the top-left.
      const y = height - pad - (value / max) * (height - pad * 2);
      return `${index === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');
}

export function LineChart({ points, height = 140, label }: ChartProps) {
  const values = points.map((point) => point.value);
  const max = Math.max(...values, 0);
  const total = values.reduce((sum, value) => sum + value, 0);
  const pad = 12;
  const line = path(values, WIDTH, height, pad);
  const area = line
    ? `${line} L${(WIDTH - pad).toFixed(1)},${height - pad} L${pad},${height - pad} Z`
    : '';

  return (
    <div>
      <div className="mb-1 flex items-baseline justify-between">
        <span className="label">{label}</span>
        <span className="tnum text-xs text-dim">
          {total.toLocaleString()} total · peak {max.toLocaleString()}
        </span>
      </div>

      <svg
        viewBox={`0 0 ${WIDTH} ${height}`}
        className="h-36 w-full"
        preserveAspectRatio="none"
        role="img"
        aria-label={`${label}: ${total} over ${points.length} days, peaking at ${max}`}
      >
        <line
          x1={pad}
          y1={height - pad}
          x2={WIDTH - pad}
          y2={height - pad}
          stroke="var(--color-line)"
          strokeWidth="1"
          vectorEffect="non-scaling-stroke"
        />
        {area && <path d={area} fill="var(--color-accent)" opacity="0.12" />}
        {line && (
          <path
            d={line}
            fill="none"
            stroke="var(--color-accent)"
            strokeWidth="1.5"
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        )}
      </svg>

      <div className="mt-1 flex justify-between font-mono text-[10px] text-dim">
        <span>{points[0]?.day ?? ''}</span>
        <span>{points[points.length - 1]?.day ?? ''}</span>
      </div>
    </div>
  );
}

interface BarsProps {
  rows: { label: string; value: number; note?: string | undefined }[];
  label?: string;
  empty?: string;
}

/** Horizontal bars, which read better than a line for "top N" data. */
export function BarList({ rows, label, empty = 'Nothing yet.' }: BarsProps) {
  const max = Math.max(...rows.map((row) => row.value), 1);

  return (
    <div>
      {label && <div className="label mb-2">{label}</div>}
      {rows.length === 0 ? (
        <p className="text-xs text-dim">{empty}</p>
      ) : (
        <ul className="space-y-1.5">
          {rows.map((row) => (
            <li key={row.label} className="group">
              <div className="flex items-baseline justify-between gap-3">
                <span className="truncate font-mono text-[11px] text-paper">{row.label}</span>
                <span className="tnum shrink-0 text-[11px] text-dim">
                  {row.value.toLocaleString()}
                  {row.note ? ` ${row.note}` : ''}
                </span>
              </div>
              <div className="mt-0.5 h-1 w-full overflow-hidden rounded bg-line-soft">
                <div
                  className="h-full rounded bg-accent/70"
                  style={{ width: `${Math.max(2, (row.value / max) * 100)}%` }}
                />
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** A tiny inline trend, for a stat card. */
export function Sparkline({ points }: { points: DayValue[] }) {
  const values = points.map((point) => point.value);
  const line = path(values, 100, 24, 2);

  return (
    <svg viewBox="0 0 100 24" className="h-6 w-full" preserveAspectRatio="none" aria-hidden="true">
      {line && (
        <path
          d={line}
          fill="none"
          stroke="var(--color-accent)"
          strokeWidth="1.5"
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  );
}
