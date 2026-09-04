/** Shape of `/api/results` — mirrors the harness's `--json` output exactly. */
export interface Row {
  n: number;
  /**
   * Which geometry case produced this row. `offset` keeps every cut plane
   * strictly inside the wall; `flush` makes them coincident with its faces,
   * which is where kernels genuinely disagree.
   */
  workload?: string;
  [kernel: string]: number | string | null | undefined;
}

export interface Results {
  reps: number;
  mismatches: number;
  /** Kernels actually compiled into this build; absent ones are omitted. */
  built: string[];
  rows: Row[];
  generatedAt?: string;
}

/** Display metadata per kernel. `analytic` paths skip the 3D boolean entirely. */
export const KERNELS: Record<
  string,
  { label: string; color: string; kind: "general" | "analytic"; lang: "rust" | "c++" }
> = {
  axiolid: { label: "axiolid", color: "#6366f1", kind: "general", lang: "rust" },
  raw_boolmesh: { label: "raw boolmesh", color: "#8b5cf6", kind: "general", lang: "rust" },
  lite_kernel: { label: "ifc-lite kernel", color: "#ec4899", kind: "general", lang: "rust" },
  manifold: { label: "Manifold", color: "#14b8a6", kind: "general", lang: "c++" },
  cgal: { label: "CGAL", color: "#f59e0b", kind: "general", lang: "c++" },
  occt: { label: "OpenCascade", color: "#ef4444", kind: "general", lang: "c++" },
  lite_rectfast: { label: "ifc-lite rect_fast", color: "#22c55e", kind: "analytic", lang: "rust" },
  cellular: { label: "cellular (ours)", color: "#3b82f6", kind: "analytic", lang: "rust" },
};

export const kernelOrder = (built: string[]) =>
  Object.keys(KERNELS).filter((k) => built.includes(k));
