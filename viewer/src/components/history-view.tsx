import { useMemo, useState } from "react";
import { AREA_LABELS } from "@/perf-types";
import type { HistoryDoc } from "@/perf-types";

/**
 * What each optimisation actually bought, per area.
 *
 * Every number here is a fresh measurement of that exact kernel commit,
 * not a figure remembered from when the change landed. Deltas inside the
 * noise band are labelled as such rather than dressed up as wins.
 */
export function HistoryView({ doc }: { doc: HistoryDoc }) {
  const [onlyReal, setOnlyReal] = useState(true);

  const first = doc.revs[0]?.rev ?? "";
  const last = doc.revs[doc.revs.length - 1]?.rev ?? "";

  const rows = useMemo(() => {
    const list = doc.areas.map((p) => {
      const base = p.ms[first] ?? 0;
      const now = p.ms[last] ?? 0;
      const gain = base ? ((base - now) / base) * 100 : 0;
      return { ...p, base, now, gain, real: Math.abs(gain) > doc.noiseFloorPct };
    });
    const shown = onlyReal ? list.filter((r) => r.real) : list;
    return [...shown].sort((a, b) => b.gain - a.gain);
  }, [doc, onlyReal, first, last]);

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">What each optimisation bought</h2>
        <p className="text-sm text-muted-foreground">
          Every column is a re-measurement of that kernel commit, median of
          {" "}{doc.reps} runs pinned to one core. Changes smaller than the
          {" "}{doc.noiseFloorPct.toFixed(0)}% noise floor are not results.
        </p>
      </div>

      <div className="flex flex-wrap gap-3 text-xs">
        {doc.revs.map((r) => (
          <div key={r.rev} className="rounded border bg-card px-2.5 py-1.5">
            <code className="font-mono">{r.rev}</code>
            <span className="ml-2 text-muted-foreground">{r.note}</span>
          </div>
        ))}
      </div>

      <label className="flex items-center gap-2 text-xs">
        <input
          type="checkbox"
          checked={onlyReal}
          onChange={(e) => setOnlyReal(e.target.checked)}
        />
        <span>Hide areas whose change is within the noise band</span>
      </label>

      <div className="overflow-x-auto rounded-lg border bg-card">
        <table className="w-full text-sm">
          <thead className="border-b text-xs text-muted-foreground">
            <tr>
              <th className="px-3 py-2 text-left">Area</th>
              {doc.revs.map((r) => (
                <th key={r.rev} className="px-3 py-2 text-right font-mono">
                  {r.rev}
                </th>
              ))}
              <th className="px-3 py-2 text-right">Net</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.area} className="border-b last:border-0">
                <td className="px-3 py-2">
                  {AREA_LABELS[r.area]?.label ?? r.area}
                </td>
                {doc.revs.map((rev, i) => {
                  const ms = r.ms[rev.rev] ?? 0;
                  const prev = i > 0 ? r.ms[doc.revs[i - 1].rev] ?? 0 : 0;
                  const step = i > 0 && prev ? ((prev - ms) / prev) * 100 : 0;
                  const moved = i > 0 && Math.abs(step) > doc.noiseFloorPct;
                  return (
                    <td key={rev.rev} className="px-3 py-2 text-right tabular-nums">
                      {ms.toFixed(0)}ms
                      {moved && (
                        <span className={step > 0 ? "ml-1 text-emerald-600" : "ml-1 text-red-600"}>
                          {step > 0 ? "-" : "+"}{Math.abs(step).toFixed(0)}%
                        </span>
                      )}
                    </td>
                  );
                })}
                <td className="px-3 py-2 text-right font-medium tabular-nums">
                  {r.real ? (
                    <span className={r.gain > 0 ? "text-emerald-600" : "text-red-600"}>
                      {r.gain > 0 ? "-" : "+"}{Math.abs(r.gain).toFixed(0)}%
                    </span>
                  ) : (
                    <span className="text-muted-foreground">within noise</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
