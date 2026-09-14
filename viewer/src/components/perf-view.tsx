import { useMemo, useState } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Legend,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { AREA_LABELS, CATEGORY_LABELS, CATEGORY_COLORS } from "@/perf-types";
import type { PerfDoc } from "@/perf-types";

const colour = (c: string) => CATEGORY_COLORS[c] ?? "#64748b";

type Mode = "relative" | "absolute";
type SortKey = "time" | "name" | "category";
type Group = "none" | "dominant";

/**
 * Cost composition per area.
 *
 * Two things the chart must not do: imply a measured 0 where a category
 * was merely unlisted, and let a 4.4s area flatten a 78ms one into an
 * invisible sliver. Hence relative mode by default, absolute available,
 * and unclassified always drawn.
 */
export function PerfView({ doc }: { doc: PerfDoc }) {
  const [openArea, setOpenArea] = useState<string | null>(null);
  const [openCat, setOpenCat] = useState<string | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("relative");
  const [sortKey, setSortKey] = useState<SortKey>("time");
  const [group, setGroup] = useState<Group>("none");
  const [minPct, setMinPct] = useState(0);

  const cats = useMemo(() => {
    const seen = new Set<string>();
    doc.areas.forEach((a) => a.categories.forEach((c) => seen.add(c.name)));
    return [...seen];
  }, [doc]);

  // Focused category is drawn FIRST so every bar starts at x=0 with it:
  // comparing segment lengths across areas is only sound from a shared
  // origin, which is the whole point of clicking a category.
  const series = useMemo(() => {
    if (!focus) return cats;
    return [focus, ...cats.filter((c) => c !== focus)];
  }, [cats, focus]);

  const rows = useMemo(() => {
    const pctOf = (a: PerfDoc["areas"][number], name: string) =>
      a.categories.find((c) => c.name === name)?.pct ?? 0;

    let list = doc.areas.filter(
      (a) => !focus || pctOf(a, focus) >= minPct,
    );

    const dominant = (a: PerfDoc["areas"][number]) =>
      [...a.categories].sort((x, y) => y.pct - x.pct)[0]?.name ?? "unclassified";

    list = [...list].sort((a, b) => {
      if (sortKey === "name") return a.area.localeCompare(b.area);
      // Sorting by category is only meaningful once one is focused;
      // fall back to time so the control never silently does nothing.
      if (sortKey === "category" && focus) return pctOf(b, focus) - pctOf(a, focus);
      return b.wall_ms - a.wall_ms;
    });

    if (group === "dominant") {
      list = [...list].sort((a, b) => dominant(a).localeCompare(dominant(b)));
    }

    return list.map((a) => {
      const row: Record<string, string | number> = {
        area: AREA_LABELS[a.area]?.label ?? a.area,
        id: a.area,
        wall: a.wall_ms,
        dominant: dominant(a),
      };
      a.categories.forEach((c) => {
        // Absolute mode charts pct of measured wall time, so a category
        // that is 30% of a 4s area outweighs 90% of a 78ms one.
        row[c.name] = mode === "absolute" ? (c.pct / 100) * a.wall_ms : c.pct;
      });
      return row;
    });
  }, [doc, focus, minPct, sortKey, group, mode]);

  const area = doc.areas.find((a) => a.area === openArea) ?? null;
  const label = (c: string) => CATEGORY_LABELS[c] ?? c;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Where time goes, per area</h2>
        <p className="text-sm text-muted-foreground">
          Each bar is one area, split by what the CPU was actually doing.
          Click a category to isolate it, or a bar to drill in.
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-2 text-xs">
        <div className="inline-flex rounded-md border p-0.5">
          {(["relative", "absolute"] as Mode[]).map((m) => (
            <button
              key={m}
              onClick={() => setMode(m)}
              className={`rounded px-2.5 py-1 ${mode === m ? "bg-primary text-primary-foreground" : "hover:bg-muted"}`}
            >
              {m === "relative" ? "% of area" : "milliseconds"}
            </button>
          ))}
        </div>

        <label className="flex items-center gap-1">
          <span className="text-muted-foreground">Sort</span>
          <select
            value={sortKey}
            onChange={(e) => setSortKey(e.target.value as SortKey)}
            className="rounded border bg-background px-1.5 py-1"
          >
            <option value="time">total time</option>
            <option value="name">name</option>
            <option value="category" disabled={!focus}>
              focused category
            </option>
          </select>
        </label>

        <label className="flex items-center gap-1">
          <span className="text-muted-foreground">Group</span>
          <select
            value={group}
            onChange={(e) => setGroup(e.target.value as Group)}
            className="rounded border bg-background px-1.5 py-1"
          >
            <option value="none">none</option>
            <option value="dominant">dominant cost</option>
          </select>
        </label>

        {focus && (
          <label className="flex items-center gap-1">
            <span className="text-muted-foreground">
              {label(focus)} &ge;
            </span>
            <input
              type="number"
              min={0}
              max={100}
              value={minPct}
              onChange={(e) => setMinPct(Number(e.target.value))}
              className="w-16 rounded border bg-background px-1.5 py-1"
            />
            <span className="text-muted-foreground">%</span>
          </label>
        )}

        {focus && (
          <button
            onClick={() => { setFocus(null); setMinPct(0); }}
            className="rounded border px-2 py-1 hover:bg-muted"
          >
            Clear focus: {label(focus)}
          </button>
        )}
        <span className="text-muted-foreground">
          {rows.length} of {doc.areas.length} areas
        </span>
      </div>

      <div className="rounded-lg border bg-card p-4">
        <ResponsiveContainer width="100%" height={Math.max(300, rows.length * 34)}>
          <BarChart data={rows} layout="vertical" margin={{ left: 8, right: 16, top: 8, bottom: 8 }}>
            <CartesianGrid strokeDasharray="3 3" opacity={0.25} />
            <XAxis
              type="number"
              domain={mode === "relative" ? [0, 100] : [0, "auto"]}
              ticks={mode === "relative" ? [0, 25, 50, 75, 100] : undefined}
              tickFormatter={(v: number) => (mode === "relative" ? `${v}%` : `${Math.round(v)}ms`)}
              fontSize={12}
            />
            <YAxis type="category" dataKey="area" width={110} interval={0} fontSize={12} />

            <Tooltip
              formatter={(v: number, n: string) => [
                mode === "relative" ? `${v.toFixed(1)}%` : `${v.toFixed(1)}ms`,
                label(n),
              ]}
            />
            <Legend
              onClick={(e) => {
                const k = String((e as { dataKey?: string | number }).dataKey ?? "");
                setFocus((cur) => (cur === k ? null : k));
                setMinPct(0);
              }}
              formatter={(v: string) => label(v)}
              wrapperStyle={{ fontSize: 12, cursor: "pointer" }}
            />

            {series.map((c) => (
              <Bar
                key={c}
                dataKey={c}
                stackId="a"
                fill={CATEGORY_COLORS[c] ?? "#94a3b8"}
                // Muted rather than hidden: dropping the other segments
                // would rescale the bar and imply the area got cheaper.
                fillOpacity={!focus || focus === c ? 1 : 0.15}
                onClick={(d: { id?: string }) => setOpenArea(d?.id ?? null)}
                cursor="pointer"
              />
            ))}
          </BarChart>
        </ResponsiveContainer>
      </div>

      {area && (
        <div className="rounded-lg border bg-card p-4 space-y-4">
          <div className="flex items-baseline justify-between gap-3">
            <div>
              <h3 className="font-semibold">
                {AREA_LABELS[area.area]?.label ?? area.area}
              </h3>
              <p className="text-sm text-muted-foreground">
                {AREA_LABELS[area.area]?.blurb}
              </p>
            </div>
            <div className="text-right text-xs text-muted-foreground">
              <div>{area.wall_ms.toFixed(0)} ms wall</div>
              <div>{area.sampled_pct.toFixed(1)}% of samples attributed</div>
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <ResponsiveContainer width="100%" height={240}>
              <PieChart>
                <Pie
                  data={area.categories}
                  dataKey="pct"
                  nameKey="name"
                  innerRadius={50}
                  outerRadius={90}
                  cursor="pointer"
                  onClick={(d: { name?: string }) =>
                    setOpenCat(d?.name === openCat ? null : (d?.name ?? null))
                  }
                >
                  {area.categories.map((c) => (
                    <Cell key={c.name} fill={colour(c.name)} />
                  ))}
                </Pie>
                <Tooltip
                  formatter={(v: number, n: string) => [`${v.toFixed(1)}%`, label(n)]}
                />
              </PieChart>
            </ResponsiveContainer>

            <div className="space-y-1">
              {area.categories.map((c) => (
                <button
                  key={c.name}
                  onClick={() => setOpenCat(c.name === openCat ? null : c.name)}
                  className={`flex w-full items-center gap-2 rounded px-2 py-1 text-left text-sm transition hover:bg-accent ${
                    c.name === openCat ? "bg-accent" : ""
                  }`}
                  title={c.reason}
                >
                  <span
                    className="h-3 w-3 shrink-0 rounded-sm"
                    style={{ background: colour(c.name) }}
                  />
                  <span className="flex-1">{label(c.name)}</span>
                  <span className="tabular-nums text-muted-foreground">
                    {c.pct.toFixed(1)}%
                  </span>
                </button>
              ))}
            </div>
          </div>

          {openCat && (
            <div className="rounded border bg-background p-3">
              <div className="mb-2 text-sm font-medium">
                {label(openCat)} — functions
              </div>
              <p className="mb-2 text-xs text-muted-foreground">
                {area.categories.find((c) => c.name === openCat)?.reason}
              </p>
              <div className="space-y-1">
                {(area.symbols[openCat] ?? []).map((s) => (
                  <div key={s.symbol} className="flex items-center gap-2 text-xs">
                    <div className="h-2 flex-1 overflow-hidden rounded bg-muted">
                      <div
                        className="h-full"
                        style={{
                          width: `${Math.min(100, (s.pct / (area.categories.find((c) => c.name === openCat)?.pct || 1)) * 100)}%`,
                          background: colour(openCat),
                        }}
                      />
                    </div>
                    <span className="w-12 shrink-0 text-right tabular-nums">
                      {s.pct.toFixed(1)}%
                    </span>
                    <code className="w-[52%] shrink-0 truncate" title={s.symbol}>
                      {s.symbol}
                    </code>
                  </div>
                ))}
              </div>
            </div>
          )}

          {area.unclassified.length > 0 && (
            <details className="rounded border bg-background p-3 text-xs">
              <summary className="cursor-pointer font-medium">
                Unclassified symbols ({area.unclassified.length})
              </summary>
              <p className="mt-2 text-muted-foreground">
                Symbols no rule matched. Listed rather than folded into a
                bucket, so the classification stays checkable.
              </p>
              <div className="mt-2 space-y-1">
                {area.unclassified.map((s) => (
                  <div key={s.symbol} className="flex gap-2">
                    <span className="w-12 shrink-0 text-right tabular-nums">
                      {s.pct.toFixed(1)}%
                    </span>
                    <code className="truncate" title={s.symbol}>{s.symbol}</code>
                  </div>
                ))}
              </div>
            </details>
          )}
        </div>
      )}
    </div>
  );
}
