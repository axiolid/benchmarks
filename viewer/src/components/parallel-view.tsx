import {
  CartesianGrid,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { AREA_LABELS, type PerfDoc } from "@/perf-types";

const VERDICT_STYLE: Record<string, string> = {
  scales: "bg-green-500/15 text-green-700 dark:text-green-400",
  partial: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
  "does not scale": "bg-red-500/15 text-red-700 dark:text-red-400",
};

/**
 * Thread scaling per area, with the evidence for each verdict.
 *
 * Speedup alone cannot distinguish "this code is serial" from "the
 * harness never applied the thread count", so every row also carries
 * the CPU/wall ratio: if N threads were genuinely busy it approaches N.
 * Publishing the verdict without that number would be an unfalsifiable
 * claim.
 */
export function ParallelView({ doc }: { doc: PerfDoc }) {
  const merged = doc.scaling[0]?.points.map((_, i) => {
    const row: Record<string, number | string> = {
      threads: doc.scaling[0].points[i].threads,
    };
    doc.scaling.forEach((s) => {
      row[s.area] = s.points[i]?.speedup ?? 0;
    });
    return row;
  }) ?? [];

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Parallelism</h2>
        <p className="text-sm text-muted-foreground">
          Same work, 1 to 16 threads. The dashed line is linear speedup —
          the shape you would see if the work parallelised perfectly.
        </p>
      </div>

      <div className="rounded-lg border bg-card p-4">
        <ResponsiveContainer width="100%" height={280}>
          <LineChart data={merged}>
            <CartesianGrid strokeDasharray="3 3" opacity={0.3} />
            <XAxis dataKey="threads" fontSize={12}
                   label={{ value: "threads", position: "insideBottom", offset: -4 }} />
            <YAxis fontSize={12} domain={[0, "auto"]}
                   label={{ value: "speedup", angle: -90, position: "insideLeft" }} />
            <Tooltip formatter={(v: number) => `${v.toFixed(2)}x`} />
            <ReferenceLine
              stroke="#94a3b8"
              strokeDasharray="4 4"
              segment={[{ x: 1, y: 1 }, { x: 16, y: 16 }]}
            />
            {doc.scaling.map((s, i) => (
              <Line
                key={s.area}
                type="monotone"
                dataKey={s.area}
                stroke={["#6366f1", "#ec4899", "#22c55e", "#f59e0b", "#14b8a6", "#8b5cf6", "#ef4444"][i % 7]}
                strokeWidth={2}
                dot={{ r: 3 }}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </div>

      <div className="rounded-lg border bg-card">
        <table className="w-full text-sm">
          <thead className="border-b text-xs text-muted-foreground">
            <tr>
              <th className="p-2 text-left font-medium">Area</th>
              <th className="p-2 text-right font-medium">1 thread</th>
              <th className="p-2 text-right font-medium">16 threads</th>
              <th className="p-2 text-right font-medium">Peak</th>
              <th className="p-2 text-right font-medium" title="CPU-seconds divided by wall-seconds. ~1 means the work never left one core.">CPU/wall</th>
              <th className="p-2 text-left font-medium">Verdict</th>
            </tr>
          </thead>
          <tbody>
            {doc.scaling.map((s) => (
              <tr key={s.area} className="border-b last:border-0">
                <td className="p-2">
                  {AREA_LABELS[s.area]?.label ?? s.area}
                </td>
                <td className="p-2 text-right tabular-nums">
                  {s.points[0]?.ms.toFixed(0)} ms
                </td>
                <td className="p-2 text-right tabular-nums">
                  {s.points[s.points.length - 1]?.ms.toFixed(0)} ms
                </td>
                <td className="p-2 text-right tabular-nums">
                  {s.peak_speedup.toFixed(2)}x
                </td>
                <td className="p-2 text-right tabular-nums">
                  {s.cpu_ratio_at_max.toFixed(2)}
                </td>
                <td className="p-2">
                  <span
                    className={`rounded px-2 py-0.5 text-xs ${VERDICT_STYLE[s.verdict] ?? ""}`}
                    title={s.evidence}
                  >
                    {s.verdict}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="rounded-lg border-l-4 border-l-amber-500 bg-amber-500/5 p-4 text-sm">
        <div className="font-medium">Reading this table</div>
        <p className="mt-1 text-muted-foreground">
          Every area currently reports a CPU/wall ratio near 1. That is
          the measurement saying the work stayed on one core — not that
          the thread count failed to apply. rayon <em>is</em> called on the
          boolean path, so the parallel regions exist; they are simply too
          small a share of the work to move the total. Cross-reference the
          cost breakdown: an area dominated by sorting or allocation will
          not improve by adding threads until that work shrinks.
        </p>
      </div>
    </div>
  );
}
