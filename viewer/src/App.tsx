import { useCallback, useEffect, useState } from "react";
import { ComparisonChart, ScalingChart } from "@/components/charts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { KERNELS, type Results, kernelOrder } from "@/types";

/** Display text per workload id emitted by the harness. */
const WORKLOADS: Record<string, { label: string; caption: string }> = {
  offset: {
    label: "Offset openings",
    caption: "Cut planes strictly inside the wall.",
  },
  flush: {
    label: "Coincident faces",
    caption: "Cut planes coincident with the wall's faces — the degenerate case.",
  },
  rotated: {
    label: "Rotated openings",
    caption:
      "Openings rotated 30° in plan. Analytic fast paths are only valid for axis-aligned operands, so they decline here and the row shows the general solver's real cost.",
  },
};

export default function App() {
  const [data, setData] = useState<Results | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [workload, setWorkload] = useState("offset");

  const load = useCallback(async (fresh = false) => {
    setError(null);
    if (fresh) setRunning(true);
    try {
      const res = await fetch(`/api/results${fresh ? "?refresh=1&reps=5" : ""}`);
      const body = await res.json();
      if (!res.ok) throw new Error(body.error ?? `HTTP ${res.status}`);
      setData(body);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const workloads = Array.from(
    new Set((data?.rows ?? []).map((r) => r.workload ?? "offset")),
  );
  const viewRows = (data?.rows ?? []).filter(
    (r) => (r.workload ?? "offset") === workload,
  );
  const maxN = viewRows.at(-1)?.n ?? 64;

  return (
    <div className="mx-auto max-w-6xl px-6 py-10">
      <header className="mb-8 flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Axiolid kernel benchmarks</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Mesh boolean: one wall, N rectangular openings. Kernels called directly through their
            Rust/C ABIs — no IfcConvert, no file I/O, no STEP parsing.
          </p>
        </div>
        <Button onClick={() => void load(true)} disabled={running}>
          {running ? "Running…" : "Re-run benchmark"}
        </Button>
      </header>

      {error && (
        <Card className="mb-6 border-destructive">
          <CardHeader>
            <CardTitle className="text-destructive">Benchmark failed</CardTitle>
            <CardDescription className="font-mono text-xs">{error}</CardDescription>
          </CardHeader>
        </Card>
      )}

      {!data && !error && <p className="text-sm text-muted-foreground">Loading results…</p>}

      {data && (
        <>
          <div className="mb-6 flex flex-wrap items-center gap-2">
            <Badge variant="secondary">best-of-{data.reps}</Badge>
            {kernelOrder(data.built).map((k) => (
              <Badge
                key={k}
                variant="outline"
                style={{ borderColor: KERNELS[k].color, color: KERNELS[k].color }}
              >
                {KERNELS[k].label}
                <span className="ml-1 opacity-60">
                  {KERNELS[k].kind === "analytic" ? "analytic" : KERNELS[k].lang}
                </span>
              </Badge>
            ))}
            {data.mismatches > 0 && (
              <Badge variant="destructive">
                {data.mismatches} volume mismatch{data.mismatches > 1 ? "es" : ""}
              </Badge>
            )}
          </div>

          {workloads.length > 1 && (
            <div className="mb-4 flex flex-wrap items-center gap-2">
              <span className="text-sm text-muted-foreground">Geometry:</span>
              {workloads.map((w) => (
                <Button
                  key={w}
                  size="sm"
                  variant={w === workload ? "default" : "outline"}
                  onClick={() => setWorkload(w)}
                >
                  {WORKLOADS[w]?.label ?? w}
                </Button>
              ))}
              <span className="text-xs text-muted-foreground">
                {WORKLOADS[workload]?.caption ?? ""}
              </span>
            </div>
          )}
          <Tabs defaultValue="scaling">
            <TabsList>
              <TabsTrigger value="scaling">Scaling</TabsTrigger>
              <TabsTrigger value="compare">Head-to-head</TabsTrigger>
              <TabsTrigger value="table">Table</TabsTrigger>
            </TabsList>

            <TabsContent value="scaling">
              <Card>
                <CardHeader>
                  <CardTitle>Time vs opening count</CardTitle>
                  <CardDescription>
                    Log scale — the field spans five orders of magnitude. Solid lines are analytic
                    paths that skip the 3D boolean entirely; dashed lines are general booleans.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <ScalingChart data={{ ...data, rows: viewRows }} />
                </CardContent>
              </Card>
            </TabsContent>

            <TabsContent value="compare">
              <Card>
                <CardHeader>
                  <CardTitle>At {maxN} openings</CardTitle>
                  <CardDescription>
                    The heaviest measured case, where the algorithmic gap is widest.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <ComparisonChart data={{ ...data, rows: viewRows }} n={maxN} />
                </CardContent>
              </Card>
            </TabsContent>

            <TabsContent value="table">
              <Card>
                <CardContent className="pt-6">
                  <div className="overflow-x-auto">
                    <table className="w-full text-sm">
                      <thead>
                        <tr className="border-b border-border text-left">
                          <th className="pb-2 pr-4 font-medium">n</th>
                          {kernelOrder(data.built).map((k) => (
                            <th key={k} className="pb-2 pr-4 font-medium">
                              {KERNELS[k].label}
                            </th>
                          ))}
                        </tr>
                      </thead>
                      <tbody className="font-mono text-xs">
                        {viewRows.map((row) => (
                          <tr key={row.n} className="border-b border-border/50">
                            <td className="py-2 pr-4">{row.n}</td>
                            {kernelOrder(data.built).map((k) => (
                              <td key={k} className="py-2 pr-4">
                                {row[k] == null ? (
                                  <span className="text-muted-foreground">deferred</span>
                                ) : (
                                  `${(row[k] as number).toFixed(3)}`
                                )}
                              </td>
                            ))}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  <p className="mt-4 text-xs text-muted-foreground">
                    <strong>deferred</strong> = kernel declined the input (returned no result). Not a
                    speed win.
                  </p>
                </CardContent>
              </Card>
            </TabsContent>
          </Tabs>
        </>
      )}
    </div>
  );
}
