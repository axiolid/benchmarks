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
};
