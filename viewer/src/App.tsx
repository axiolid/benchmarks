import { useEffect, useState } from "react";
import { ComparisonView } from "@/components/comparison-view";
import { HistoryView } from "@/components/history-view";
import { ParallelView } from "@/components/parallel-view";
import { PerfView } from "@/components/perf-view";
import type { HistoryDoc, PerfDoc } from "@/perf-types";

type SectionId = "comparison" | "perf" | "history" | "parallel" | "method";

const SECTIONS: { id: SectionId; label: string; blurb: string }[] = [
  { id: "perf", label: "Cost breakdown", blurb: "Where time goes per area" },
  { id: "history", label: "Before / after", blurb: "What each optimisation bought" },
  { id: "parallel", label: "Parallelism", blurb: "Thread scaling per area" },
  { id: "comparison", label: "Kernel comparison", blurb: "Axiolid vs other kernels" },
  { id: "method", label: "Method", blurb: "How these numbers are produced" },
];

export default function App() {
  const [section, setSection] = useState<SectionId>("perf");
  const [perf, setPerf] = useState<PerfDoc | null>(null);
  const [perfError, setPerfError] = useState<string | null>(null);
  const [history, setHistory] = useState<HistoryDoc | null>(null);

  useEffect(() => {
    fetch("/perf.json")
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`))))
      .then(setPerf)
      .catch((e: Error) => setPerfError(e.message));
    // History is optional: the page still works without it, so a missing
    // file must not blank the whole view.
    fetch("/history.json")
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(setHistory)
      .catch(() => setHistory(null));
  }, []);

  return (
    <div className="flex min-h-screen">
      <nav className="w-60 shrink-0 border-r bg-card">
        <div className="sticky top-0 p-4">
          <div className="mb-6">
            <div className="text-sm font-semibold tracking-tight">Axiolid</div>
            <div className="text-xs text-muted-foreground">benchmarks</div>
          </div>
          <ul className="space-y-1">
            {SECTIONS.map((s) => (
              <li key={s.id}>
                <button
                  onClick={() => setSection(s.id)}
                  className={`w-full rounded-md px-3 py-2 text-left transition ${
                    section === s.id
                      ? "bg-primary/10 text-primary"
                      : "hover:bg-accent"
                  }`}
                >
                  <div className="text-sm font-medium">{s.label}</div>
                  <div className="text-xs text-muted-foreground">{s.blurb}</div>
                </button>
              </li>
            ))}
          </ul>
          {perf && (
            <p className="mt-6 text-[11px] leading-relaxed text-muted-foreground">
              Profiled {new Date(perf.generatedAt).toLocaleString()}
            </p>
          )}
        </div>
      </nav>

      <main className="flex-1 px-8 py-10">
        <div className="mx-auto max-w-5xl">
          {section === "comparison" && <ComparisonView />}

          {section === "history" &&
            (history ? (
              <HistoryView doc={history} />
            ) : (
              <p className="text-sm text-muted-foreground">
                No history data yet. Run{" "}
                <code className="rounded bg-muted px-1">scripts/perf-history.sh</code>.
              </p>
            ))}

          {(section === "perf" || section === "parallel") && (
            <>
              {perfError && (
                <div className="rounded-lg border border-destructive p-4 text-sm">
                  <div className="font-medium text-destructive">
                    Profile data unavailable
                  </div>
                  <p className="mt-1 text-muted-foreground">
                    {perfError}. Generate it with
                    <code className="mx-1">python3 scripts/perf-areas.py &gt; viewer/public/perf.json</code>
                  </p>
                </div>
              )}
              {!perf && !perfError && (
                <p className="text-sm text-muted-foreground">Loading profile…</p>
              )}
              {perf && section === "perf" && <PerfView doc={perf} />}
              {perf && section === "parallel" && <ParallelView doc={perf} />}
            </>
          )}

          {section === "method" && (
            <div className="space-y-4 text-sm">
              <h2 className="text-lg font-semibold">Method</h2>
              <p className="text-muted-foreground">
                Each area runs as its own process under
                <code className="mx-1">perf record</code>, so samples belong
                to one area only. Time is attributed to a cost category by
                classifying the symbol perf reports.
              </p>
              <div className="rounded-lg border-l-4 border-l-amber-500 bg-amber-500/5 p-4">
                <div className="font-medium">What this is not</div>
                <p className="mt-1 text-muted-foreground">
                  Symbol classification is a heuristic, not ground truth. A
                  symbol matching no rule is reported as
                  <em className="mx-1">unclassified</em> rather than folded
                  into a bucket — if that share is large, treat the chart for
                  that area with suspicion. Domain rules were written by
                  reading the functions concerned, not by guessing from names.
                </p>
              </div>
              <h3 className="pt-2 font-medium">Classification rules, in order</h3>
              <div className="rounded-lg border">
                <table className="w-full text-xs">
                  <thead className="border-b text-muted-foreground">
                    <tr>
                      <th className="p-2 text-left font-medium">Category</th>
                      <th className="p-2 text-left font-medium">Matches</th>
                    </tr>
                  </thead>
                  <tbody>
                    {perf?.rules.map((r, i) => (
                      <tr key={i} className="border-b last:border-0">
                        <td className="p-2 align-top">{r.category}</td>
                        <td className="p-2 align-top text-muted-foreground">
                          {r.reason}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
