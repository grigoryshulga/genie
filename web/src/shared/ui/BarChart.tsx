// A bar chart by day for the server's statistics: side by side or stacked, with a legend
// and the numbers of the day under the pointer.

import { useState } from "react";

const DAYS_SHORT = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];

/** A column's caption: the weekday over a week, every fifth date over a month. */
export function dayLabel(day: string, i: number, count: number): string {
  const d = new Date(`${day}T00:00:00Z`);
  if (count <= 10) return DAYS_SHORT[d.getUTCDay()];
  return i % 5 === 0 || i === count - 1 ? String(d.getUTCDate()) : "";
}

export function dayTitle(day: string): string {
  return new Date(`${day}T00:00:00Z`).toLocaleDateString("ru-RU", { day: "numeric", month: "long", weekday: "short", timeZone: "UTC" });
}

export type Series<T> = { key: string; label: string; color: string; value?: (d: T) => number };

/**
 * Bars by day: side by side, or stacked (bottom to top in the order given). Hovering a day
 * shows its numbers; the legend names every series, so color never carries meaning alone.
 */
export function BarChart<T extends { day: string }>({
  title,
  days,
  series,
  stacked,
  format = String,
}: {
  title: string;
  days: T[];
  series: Series<T>[];
  stacked?: boolean;
  /** How a number reads in the legend, the scale and the tooltip. */
  format?: (n: number) => string;
}) {
  const [hover, setHover] = useState<number>();
  const val = (s: Series<T>, d: T) => (s.value ? s.value(d) : (d[s.key as keyof T] as number));
  const top = Math.max(1, ...days.map((d) => (stacked ? series.reduce((n, s) => n + val(s, d), 0) : Math.max(...series.map((s) => val(s, d))))));
  const totals = series.map((s) => days.reduce((n, d) => n + val(s, d), 0));
  const h = hover === undefined ? undefined : days[hover];
  return (
    <figure className="st-card sv-chart">
      <figcaption>
        <b>{title}</b>
        <span className="sv-legend">
          {series.map((s, i) => (
            <span key={s.key}>
              <i style={{ background: s.color }} />
              {s.label} <em>{format(totals[i])}</em>
            </span>
          ))}
        </span>
      </figcaption>
      <div className="sv-plot-wrap">
        <span className="sv-max" aria-hidden="true">
          {format(top)}
        </span>
        <div
          className={`sv-plot${stacked ? " stacked" : ""}${days.length > 10 ? " dense" : ""}`}
          role="img"
          aria-label={`${title}: ${series.map((s, i) => `${s.label} ${format(totals[i])}`).join(", ")}`}
          onMouseLeave={() => setHover(undefined)}
        >
          {days.map((d, i) => (
            <div key={d.day} className={`sv-col${hover === i ? " on" : ""}`} onMouseEnter={() => setHover(i)}>
              <div className="sv-bars">
                {(stacked ? [...series].reverse() : series).map((s) => {
                  const v = val(s, d);
                  return <span key={s.key} style={{ height: `${(v / top) * 100}%`, background: s.color, minHeight: v ? 2 : 0 }} />;
                })}
              </div>
            </div>
          ))}
          {h && (
            <div className="sv-tip" style={{ left: `${((hover! + 0.5) / days.length) * 100}%` }}>
              <b>{dayTitle(h.day)}</b>
              {series.map((s) => (
                <span key={s.key}>
                  <i style={{ background: s.color }} />
                  {s.label}: {format(val(s, h))}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>
      <div className="sv-axis" aria-hidden="true">
        {days.map((d, i) => (
          <span key={d.day}>{dayLabel(d.day, i, days.length)}</span>
        ))}
      </div>
    </figure>
  );
}
