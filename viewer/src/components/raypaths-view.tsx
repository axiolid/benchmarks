import type { RayPathsDoc } from "@/perf-types";

/*
 * Three ways to cast the same rays, measured at ONE revision.
 *
 * Deliberately NOT a column in the before/after table: the 12 history
 * areas call kernel crates directly, and the broad phase is opt-in
 * through the facade. Showing it as a revision would imply the ray
 * work moved those areas. It did not -- `raymesh` still measures the
 * scan, and still reads ~4.3 s.
 */
export function RayPathsView({ doc }: { doc: RayPathsDoc }) {
  const slowest = Math.max(...doc.variants.map((v) => v.ms));
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Ray casting: three paths</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          The same {doc.rays.toLocaleString()} rays against the same{" "}
          {doc.triangles.toLocaleString()}-triangle mesh, at one kernel
          revision. Median of {doc.reps}, pinned to one core.
        </p>
      </div>

      <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 p-3 text-sm">
        <div className="font-medium">This is a choice, not a free win</div>
        <p className="mt-1 text-muted-foreground">
          The broad phase is opt-in. The <code>raymesh</code> area on the
          cost-breakdown page still calls <code>nearest_hit</code>
          {" "}directly and still measures ~4.3 s: that number did not
          move, and is not meant to.
        </p>
        <p className="mt-2 text-muted-foreground">
          Below ~{doc.crossoverRays} rays per mesh the index never repays
          its build and the scan wins; at a single ray it is ~20x
          faster to scan.
        </p>
      </div>

      <div className="space-y-3">
        {doc.variants.map((v) => (
          <div key={v.variant} className="rounded-lg border p-3">
            <div className="flex items-baseline justify-between gap-3">
              <div className="font-medium">{v.variant}</div>
              <div className="text-sm tabular-nums">
                {v.ms.toLocaleString()} ms
                {v.speedup > 1 && (
                  <span className="ml-2 text-emerald-600">{v.speedup}x</span>
                )}
              </div>
            </div>
            <div className="mt-2 h-2 rounded bg-muted">
              <div
                className="h-2 rounded bg-primary"
                style={{ width: `${Math.max((v.ms / slowest) * 100, 0.6)}%` }}
              />
            </div>
            <p className="mt-2 text-xs text-muted-foreground">{v.note}</p>
            <p className="mt-1 text-xs text-muted-foreground">
              range {v.band[0]}-{v.band[1]} ms
            </p>
          </div>
        ))}
      </div>
    </div>
  );
}
