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
import {
  AREA_LABELS,
  CATEGORY_COLORS,
  CATEGORY_LABELS,
  type PerfDoc,
} from "@/perf-types";

const label = (c: string) => CATEGORY_LABELS[c] ?? c;
const colour = (c: string) => CATEGORY_COLORS[c] ?? "#64748b";

/**
 * Where time goes per area, drilling from all areas down to symbols.
 *
 * The stacked bar is normalised to 100% per area deliberately: areas
 * differ in absolute runtime by more than an order of magnitude, so an
 * absolute stack would render the fast areas as invisible slivers and
 * hide the very composition the chart exists to show. Absolute time is
 * given alongside as a number.
 */
export function PerfView({ doc }: { doc: PerfDoc }) {
  const [openArea, setOpenArea] = useState<string | null>(null);
  const [openCat, setOpenCat] = useState<string | null>(null);

  const cats = useMemo(() => {
    const seen = new Set<string>();
    doc.areas.forEach((a) => a.categories.forEach((c) => seen.add(c.name)));
    return [...seen];
  }, [doc]);

  const stacked = doc.areas.map((a) => {
    const row: Record<string, string | number> = {
      area: AREA_LABELS[a.area]?.label ?? a.area,
      id: a.area,
    };
    a.categories.forEach((c) => {
      row[c.name] = c.pct;
    });
    return row;
  });

  const area = doc.areas.find((a) => a.area === openArea) ?? null;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Where time goes, per area</h2>
        <p className="text-sm text-muted-foreground">
          Each bar is one area of axiolid, split by what the CPU was
          actually doing. Click a bar to drill in.
        </p>
      </div>

      <div className="rounded-lg border bg-card p-4">
        <ResponsiveContainer width="100%" height={Math.max(300, stacked.length * 34)}>
          <BarChart data={stacked} layout="vertical"
                    margin={{ left: 24, right: 16 }}>
            <CartesianGrid strokeDasharray="3 3" opacity={0.3} />
            <XAxis
              type="number"
              domain={[0, 100]}
              ticks={[0, 25, 50, 75, 100]}
              unit="%"
              fontSize={12}
            />
            <YAxis type="category" dataKey="area" width={110} fontSize={12} interval={0} />
            <Tooltip
              formatter={(v: number, n: string) => [`${v.toFixed(1)}%`, label(n)]}
            />
            <Legend formatter={(v: string) => label(v)} />
            {cats.map((c) => (
              <Bar
                key={c}
                dataKey={c}
                stackId="a"
                fill={colour(c)}
                cursor="pointer"
                onClick={(d: { id?: string }) => {
                  setOpenArea(d?.id ?? null);
                  setOpenCat(null);
                }}
              />
            ))}
          </BarChart>
        </ResponsiveContainer>
      </div>

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {doc.areas.map((a) => (
          <button
            key={a.area}
            onClick={() => {
              setOpenArea(a.area === openArea ? null : a.area);
              setOpenCat(null);
            }}
            className={`rounded-lg border p-3 text-left transition hover:border-primary ${
              a.area === openArea ? "border-primary bg-accent" : "bg-card"
            }`}
          >
            <div className="text-sm font-medium">
              {AREA_LABELS[a.area]?.label ?? a.area}
            </div>
            <div className="text-xs text-muted-foreground">
              {a.wall_ms.toFixed(0)} ms
            </div>
            <div className="mt-1 text-xs">
              <span style={{ color: colour(a.categories[0].name) }}>
                {label(a.categories[0].name)} {a.categories[0].pct.toFixed(0)}%
              </span>
            </div>
          </button>
        ))}
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
