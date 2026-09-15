/** Shape of `/perf.json` — emitted by scripts/perf-areas.py. */
export interface Category {
  name: string;
  pct: number;
  /** Why the classifier assigned this category — shown so a reader can
   *  judge the attribution instead of trusting the label. */
  reason: string;
}

export interface SymbolShare {
  symbol: string;
  pct: number;
}

export interface PerfArea {
  area: string;
  wall_ms: number;
  /** Share of samples perf actually attributed; below 100 means some
   *  samples fell under the reporting threshold. */
  sampled_pct: number;
  categories: Category[];
  symbols: Record<string, SymbolShare[]>;
  unclassified: SymbolShare[];
}

export interface ScalingPoint {
  threads: number;
  ms: number;
  speedup: number;
  efficiency: number;
  /** CPU-seconds / wall-seconds. ~1 means the work never left one core. */
  cpu_ratio: number;
}

export interface AreaScaling {
  area: string;
  points: ScalingPoint[];
  peak_speedup: number;
  verdict: string;
  cpu_ratio_at_max: number;
  evidence: string;
}

export interface PerfDoc {
  generatedAt: string;
  areas: PerfArea[];
  scaling: AreaScaling[];
  rules: { category: string; pattern: string; reason: string }[];
}

/** One colour per cost category, reused by every chart and legend. */
export const CATEGORY_COLORS: Record<string, string> = {
  sorting: "#6366f1",
  allocation: "#f59e0b",
  pointer_chasing: "#ec4899",
  hashing: "#14b8a6",
  branch_bookkeeping: "#8b5cf6",
  math: "#22c55e",
  unclassified: "#94a3b8",
  // Kernel-profile categories: distinct hues so a competitor breakdown
  // is readable instead of four shades of the fallback grey.
  "exact arithmetic": "#ef4444",
  "tree/graph traversal": "#0ea5e9",
  "memory allocation": "#f59e0b",
  "sorting/searching": "#6366f1",
  "other kernel work": "#94a3b8",
};

export const CATEGORY_LABELS: Record<string, string> = {
  sorting: "Sorting",
  allocation: "Memory allocation",
  pointer_chasing: "Pointer chasing",
  hashing: "Hashing",
  branch_bookkeeping: "Branch bookkeeping",
  math: "Maths",
  unclassified: "Unclassified",
};

/** What each area actually exercises, for the reader who is not in the code. */
export const AREA_LABELS: Record<string, { label: string; blurb: string }> = {
  boolean: { label: "Boolean", blurb: "Mesh union of two offset spheres." },
  audit: { label: "Mesh audit", blurb: "Structural health: edges, winding, manifoldness." },
  measure: { label: "Measure", blurb: "Volume, surface area and second moments." },
  levelset: { label: "Level set", blurb: "Dense field sampling to a triangle mesh." },
  inspect: { label: "Inspect", blurb: "Winding number and point containment queries." },
  heal: { label: "Heal", blurb: "Self-intersection detection and defect diagnosis." },
  genus: { label: "Genus", blurb: "Euler characteristic over the edge structure." },
  decimate: { label: "Decimate", blurb: "Edge-collapse simplification to a triangle budget." },
  refine: { label: "Refine", blurb: "Uniform subdivision, quadrupling triangles per pass." },
  raymesh: {
    label: "Ray/mesh",
    blurb:
      "Narrow-phase ray-triangle tests. nearest_hit scans every triangle by design \u2014 the broad phase lives in axiolid-spatial \u2014 so this is not a BVH measurement.",
  },
  project: { label: "Project", blurb: "Planar projection with 2D polygon overlay." },
  decompose: { label: "Decompose", blurb: "Convex decomposition by repeated plane splits." },
};

/** One optimisation step: a kernel commit and what it did to each area. */
export interface HistoryRev {
  rev: string;
  label: string;
  /** What landed in this commit, in one line. */
  note: string;
}

export interface HistoryPoint {
  area: string;
  /** Median ms per revision, keyed by short sha. */
  ms: Record<string, number>;
  /** Worst observed spread for this area, as a percentage of median. */
  noisePct: number;
}

export interface HistoryDoc {
  revs: HistoryRev[];
  /** One entry per area. Named `areas` to match the emitted JSON. */
  areas: HistoryPoint[];
  /** Deltas below this are indistinguishable from jitter. */
  noiseFloorPct: number;
  reps: number;
}

/** One way of casting rays, measured against the others. */
export interface RayPathVariant {
  variant: string;
  note: string;
  ms: number;
  band: [number, number];
  speedup: number;
  real: boolean;
}

export interface RayPathsDoc {
  noiseFloorPct: number;
  reps: number;
  rays: number;
  triangles: number;
  crossoverRays: number;
  variants: RayPathVariant[];
}

/** One kernel's measured cost breakdown, from scripts/perf-kernels.py. */
export interface KernelProfile {
  kernel: string;
  profile_share_pct: number;
  /** False when too few samples landed for the split to mean anything. */
  breakdown_trustworthy: boolean;
  total_ms?: number;
  categories: { name: string; pct: number }[];
}

export interface KernelsDoc {
  kernels: KernelProfile[];
  built: string[];
  unavailable: string[];
  note: string;
}
