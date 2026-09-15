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
  orient3: { label: "Orient3D", blurb: "Exact 3D orientation on clean input: the filter settles it." },
  orient3degen: { label: "Orient3D (degenerate)", blurb: "Same predicate, frequent degeneracy: exact arithmetic takes over." },
  orient2: { label: "Orient2D", blurb: "2D orientation, the most-called predicate in planar code." },
  incircle: { label: "In-circle", blurb: "Delaunay in-circle test with exact fallback." },
  insphere: { label: "In-sphere", blurb: "3D in-sphere: the most expensive certified predicate." },
  hull: { label: "Convex hull", blurb: "Incremental hull of a point cloud, driven by orientation tests." },
  extrude: { label: "Extrude", blurb: "Profile with holes swept to a solid: triangulation plus side walls." },
  revolve: { label: "Revolve", blurb: "Profile swept about an axis, station by station." },
  loft: { label: "Loft", blurb: "Blend through stations: topological stitching, little arithmetic." },
  offsetsolid: { label: "Offset solid", blurb: "Grow a solid: plane offsetting and face re-intersection." },
  shell: { label: "Shell", blurb: "Hollow a solid to a wall thickness: offset plus cavity." },
  frenet: { label: "Frenet frame", blurb: "Frame transport along a 3D spine: integration per evaluation." },
  arclength: { label: "Arc length", blurb: "Arc-length parameterisation via Gauss-Legendre panels." },
  curvedist: { label: "Certified curve distance", blurb: "Subdivision to a proven bound between two curves." },
  curveproject: { label: "Certified projection", blurb: "Point projected onto a curve with a proven bound." },
  raybvh: { label: "Ray/BVH", blurb: "Ray casts against a prebuilt BVH, build excluded." },
  facaderay: { label: "Facade rays", blurb: "Cached ray index over a facade: repeated nearest-hit." },
  handleray: { label: "Handle rays", blurb: "Caller-held index: rays without cache lookup." },
  minkowski: { label: "Minkowski sum", blurb: "Convex sum of two solids: cost grows with face pairs." },
  minkdiff: { label: "Minkowski difference", blurb: "Erosion by a tool solid, via the boolean provider." },
  pointindex: { label: "Point index build", blurb: "Building a spatial point index: dominated by the sort." },
  pointnear: { label: "Nearest point", blurb: "Nearest-neighbour queries against a prebuilt index." },
  bvhpairs: { label: "BVH pair query", blurb: "Self-overlap candidate pairs from the bounding hierarchy." },
  overlay: { label: "Planar overlay", blurb: "2D boolean on rings: sweep line plus intersections." },
  route: { label: "Route", blurb: "Shortest path around planar barriers." },
  fieldsample: { label: "Field sampling", blurb: "Rasterising triangles into a layered height field." },
  components: { label: "Components", blurb: "Connected-component labelling over mesh adjacency." },
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
  /** Whole-suite total across all workload rows -- NOT a per-area figure. */
  suite_ms?: number;
  /** Rows this kernel completed, of rows_total. Partial coverage makes a
   *  small suite_ms mean "did less", not "was faster". */
  rows_done?: number;
  rows_total?: number;
  /** Per-workload milliseconds, keyed "offset n=4". Null where the
   *  kernel could not complete that workload. This is the only
   *  apples-to-apples axis: every kernel runs these same rows. */
  workloads?: Record<string, number | null>;
  categories: { name: string; pct: number }[];
}

export interface KernelsDoc {
  kernels: KernelProfile[];
  built: string[];
  unavailable: string[];
  note: string;
}
