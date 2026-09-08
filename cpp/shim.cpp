// Op codes shared with the Rust side: 0 = difference, 1 = union,
// 2 = intersection. An unknown code must fail loudly rather than defaulting to
// difference, or a caller typo would silently measure the wrong operation.
#define BENCH_OP_DIFFERENCE 0
#define BENCH_OP_UNION 1
#define BENCH_OP_INTERSECTION 2

#ifdef HAS_MANIFOLD
#include <manifold/manifold.h>
#include <cstdint>
#include <vector>

using manifold::Manifold;
using manifold::OpType;
using manifold::vec3;

static Manifold box_of(const double* mn, const double* mx) {
  vec3 size(mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]);
  vec3 ctr(mn[0] + size.x * 0.5, mn[1] + size.y * 0.5, mn[2] + size.z * 0.5);
  return Manifold::Cube(size, true).Translate(ctr);
}

// Build a hexahedron from 8 explicit corners, in the SAME order every Rust
// column uses, so a rotated cutter is the identical solid on every kernel.
// Corners 0-3 are the -Z face (CCW seen from +Z), 4-7 the +Z face.
static Manifold hex_of(const double* c) {
  manifold::MeshGL64 mesh;
  mesh.numProp = 3;
  for (int i = 0; i < 24; ++i) mesh.vertProperties.push_back(c[i]);
  static const uint64_t TRIS[36] = {
      0, 2, 1, 0, 3, 2,  // -Z
      4, 5, 6, 4, 6, 7,  // +Z
      0, 1, 5, 0, 5, 4,  // -Y
      1, 2, 6, 1, 6, 5,  // +X
      2, 3, 7, 2, 7, 6,  // +Y
      3, 0, 4, 3, 4, 7,  // -X
  };
  for (int i = 0; i < 36; ++i) mesh.triVerts.push_back(TRIS[i]);
  return Manifold(mesh);
}

extern "C" double bench_manifold_subtract(const double* host_min,
                                          const double* host_max,
                                          const double* cutters, int n) {
  Manifold acc = box_of(host_min, host_max);
  for (int i = 0; i < n; ++i) {
    acc = acc.Boolean(hex_of(cutters + i * 24), OpType::Subtract);
  }
  return acc.Volume();
}

// Single operation between the host and ONE operand, for algebraic identity
// testing. Kept separate from the N-cutter loop above so the identity harness
// measures exactly one boolean, with no accumulation to hide error.
extern "C" double bench_manifold_op(const double* host_min,
                                    const double* host_max,
                                    const double* operand, int op) {
  Manifold a = box_of(host_min, host_max);
  Manifold b = hex_of(operand);
  switch (op) {
    case BENCH_OP_DIFFERENCE: return a.Boolean(b, OpType::Subtract).Volume();
    case BENCH_OP_UNION: return a.Boolean(b, OpType::Add).Volume();
    case BENCH_OP_INTERSECTION: return a.Boolean(b, OpType::Intersect).Volume();
    default: return -1.0;
  }
}

// -------------------------------------------------------------- Menger sponge
// Two constructions, deliberately both present.
//
// `bench_manifold_menger_composed` is Manifold's OWN samples/src/
// menger_sponge.cpp algorithm, reproduced so the published depth-4 timing has
// something comparable here. It exploits the fact that a Menger sponge is the
// intersection of three orthogonal Sierpinski-carpet prisms: build the carpet
// holes once, batch-union them, then subtract three rotations. That is
// (8^d-1)/7 holes and FOUR boolean operations, regardless of depth.
//
// The flattened form measured elsewhere in this harness subtracts every void
// box separately -- 58947 operations at depth 4. Comparing the two numbers
// directly is meaningless; they are different algorithms.
static void menger_fractal(std::vector<Manifold>& holes, Manifold& hole,
                           double w, double px, double py, int depth,
                           int max_depth) {
  w /= 3;
  holes.push_back(hole.Scale(vec3(w, w, 1.0)).Translate(vec3(px, py, 0.0)));
  if (depth == max_depth) return;
  const double off[8][2] = {{-w, -w}, {-w, 0.0}, {-w, w}, {0.0, w},
                            {w, w},   {w, 0.0},  {w, -w}, {0.0, -w}};
  for (int i = 0; i < 8; ++i) {
    menger_fractal(holes, hole, w, px + off[i][0], py + off[i][1], depth + 1,
                   max_depth);
  }
}

extern "C" double bench_manifold_menger_composed(int depth) {
  Manifold result = Manifold::Cube(vec3(1.0), true);
  std::vector<Manifold> holes;
  menger_fractal(holes, result, 1.0, 0.0, 0.0, 1, depth);
  Manifold hole = Manifold::BatchBoolean(holes, OpType::Add);
  result = result.Boolean(hole, OpType::Subtract);
  hole = hole.Rotate(90);
  result = result.Boolean(hole, OpType::Subtract);
  hole = hole.Rotate(0, 0, 90);
  result = result.Boolean(hole, OpType::Subtract);
  return result.Volume();
}

// How many carpet holes one orientation uses: (8^depth - 1) / 7.
extern "C" int bench_manifold_menger_holes(int depth) {
  int total = 0, level = 1;
  for (int i = 0; i < depth; ++i) {
    total += level;
    level *= 8;
  }
  return total;
}
// ---------------------------------------------------- arbitrary mesh operands
//
// Every entry point above is box-parameterised: the caller passes corners and
// the shim builds the solid. That makes non-box geometry unreachable, which is
// why the harness had no curved workload at all.
//
// These take triangle meshes directly -- (verts, nverts, tris, ntris) per
// operand -- so any closed mesh the Rust side can build is measurable on every
// kernel. Vertices are 3 doubles each, triangles 3 uint32 indices each, in the
// same layout `TriMesh` already uses, so no conversion happens on the Rust
// side where it would be timed.
//
// Returns the result volume, or -1.0 if the kernel declined. The caller
// renders a negative return as "no answer" rather than feeding it to an error
// formula, where it would look like a merely inaccurate number.
static Manifold mesh_of(const double* verts, int nverts, const uint32_t* tris,
                        int ntris) {
  manifold::MeshGL64 mesh;
  mesh.numProp = 3;
  mesh.vertProperties.reserve(static_cast<size_t>(nverts) * 3);
  for (int i = 0; i < nverts * 3; ++i) mesh.vertProperties.push_back(verts[i]);
  mesh.triVerts.reserve(static_cast<size_t>(ntris) * 3);
  for (int i = 0; i < ntris * 3; ++i) mesh.triVerts.push_back(tris[i]);
  return Manifold(mesh);
}

extern "C" double bench_manifold_mesh_op(const double* verts_a, int nverts_a,
                                         const uint32_t* tris_a, int ntris_a,
                                         const double* verts_b, int nverts_b,
                                         const uint32_t* tris_b, int ntris_b,
                                         int op) {
  Manifold a = mesh_of(verts_a, nverts_a, tris_a, ntris_a);
  Manifold b = mesh_of(verts_b, nverts_b, tris_b, ntris_b);
  OpType type;
  switch (op) {
    case BENCH_OP_DIFFERENCE:
      type = OpType::Subtract;
      break;
    case BENCH_OP_UNION:
      type = OpType::Add;
      break;
    case BENCH_OP_INTERSECTION:
      type = OpType::Intersect;
      break;
    default:
      return -1.0;
  }
  return a.Boolean(b, type).Volume();
}

#endif  // HAS_MANIFOLD

#ifdef HAS_CGAL

// ---------------------------------------------------------------------- CGAL
// The exact-predicate incumbent. Uses Polygon_mesh_processing corefinement on
// closed triangle meshes, with an exact kernel, so it is the correctness
// reference rather than a speed contender. Returns -1.0 if a boolean fails.
//
// NOTE: OpenCascade is deliberately absent. Debian's libocct-*-dev 7.8.1 ships
// Poly_ArrayOfNodes.hxx but NOT the NCollection_AliasedArray.hxx it includes,
// so BRepPrimAPI_MakeBox cannot compile; the newer header in
// ~/occt-research/occt is a different API and produces 8 errors against the
// installed headers. Building OCCT from source is the only fix. Not faked.
#include <CGAL/Exact_predicates_exact_constructions_kernel.h>
#include <CGAL/Polygon_mesh_processing/corefinement.h>
#include <CGAL/Polygon_mesh_processing/measure.h>
#include <CGAL/Surface_mesh.h>

namespace PMP = CGAL::Polygon_mesh_processing;
using CgalK = CGAL::Exact_predicates_exact_constructions_kernel;
using CgalMesh = CGAL::Surface_mesh<CgalK::Point_3>;

static CgalMesh cgal_box(const double* mn, const double* mx) {
  CgalMesh m;
  auto v = [&](double x, double y, double z) {
    return m.add_vertex(CgalK::Point_3(x, y, z));
  };
  auto a = v(mn[0], mn[1], mn[2]), b = v(mx[0], mn[1], mn[2]);
  auto c = v(mx[0], mx[1], mn[2]), d = v(mn[0], mx[1], mn[2]);
  auto e = v(mn[0], mn[1], mx[2]), f = v(mx[0], mn[1], mx[2]);
  auto g = v(mx[0], mx[1], mx[2]), h = v(mn[0], mx[1], mx[2]);
  // Outward-wound TRIANGLES: CGAL PMP requires a triangle mesh.
  m.add_face(a, d, c); m.add_face(a, c, b);
  m.add_face(e, f, g); m.add_face(e, g, h);
  m.add_face(a, b, f); m.add_face(a, f, e);
  m.add_face(b, c, g); m.add_face(b, g, f);
  m.add_face(c, d, h); m.add_face(c, h, g);
  m.add_face(d, a, e); m.add_face(d, e, h);
  return m;
}

// Same 8-corner convention as the Manifold path above.
static CgalMesh cgal_hex(const double* c) {
  CgalMesh m;
  CgalMesh::Vertex_index v[8];
  for (int i = 0; i < 8; ++i) {
    v[i] = m.add_vertex(CgalK::Point_3(c[i * 3], c[i * 3 + 1], c[i * 3 + 2]));
  }
  auto f = [&](int a, int b, int cc) { m.add_face(v[a], v[b], v[cc]); };
  f(0, 2, 1); f(0, 3, 2);
  f(4, 5, 6); f(4, 6, 7);
  f(0, 1, 5); f(0, 5, 4);
  f(1, 2, 6); f(1, 6, 5);
  f(2, 3, 7); f(2, 7, 6);
  f(3, 0, 4); f(3, 4, 7);
  return m;
}

extern "C" double bench_cgal_op(const double* host_min, const double* host_max,
                                const double* operand, int op) {
  CgalMesh a = cgal_box(host_min, host_max);
  CgalMesh b = cgal_hex(operand), out;
  bool ok = false;
  switch (op) {
    case BENCH_OP_DIFFERENCE:
      ok = PMP::corefine_and_compute_difference(a, b, out);
      break;
    case BENCH_OP_UNION:
      ok = PMP::corefine_and_compute_union(a, b, out);
      break;
    case BENCH_OP_INTERSECTION:
      ok = PMP::corefine_and_compute_intersection(a, b, out);
      break;
    default:
      return -1.0;
  }
  if (!ok) return -1.0;
  // Exact kernel: the volume is a rational, converted only at the boundary.
  return CGAL::to_double(PMP::volume(out));
}

extern "C" double bench_cgal_subtract(const double* host_min,
                                      const double* host_max,
                                      const double* cutters, int n) {
  CgalMesh acc = cgal_box(host_min, host_max);
  for (int i = 0; i < n; ++i) {
    CgalMesh tool = cgal_hex(cutters + i * 24), out;
    if (!PMP::corefine_and_compute_difference(acc, tool, out)) return -1.0;
    acc = std::move(out);
  }
  return CGAL::to_double(PMP::volume(acc));
}

// Arbitrary mesh operands; see the Manifold equivalent above for the layout
// contract. CGAL needs the mesh assembled vertex-by-vertex through its own
// index type, and orientation matters: `corefine_and_compute_*` requires
// bounded volumes, so an inside-out operand would silently invert the answer.
static CgalMesh cgal_mesh(const double* verts, int nverts, const uint32_t* tris,
                          int ntris) {
  CgalMesh m;
  std::vector<CgalMesh::Vertex_index> handles;
  handles.reserve(nverts);
  for (int i = 0; i < nverts; ++i) {
    handles.push_back(
        m.add_vertex(CgalK::Point_3(verts[i * 3], verts[i * 3 + 1], verts[i * 3 + 2])));
  }
  for (int i = 0; i < ntris; ++i) {
    m.add_face(handles[tris[i * 3]], handles[tris[i * 3 + 1]],
               handles[tris[i * 3 + 2]]);
  }
  return m;
}

extern "C" double bench_cgal_mesh_op(const double* verts_a, int nverts_a,
                                     const uint32_t* tris_a, int ntris_a,
                                     const double* verts_b, int nverts_b,
                                     const uint32_t* tris_b, int ntris_b,
                                     int op) {
  CgalMesh a = cgal_mesh(verts_a, nverts_a, tris_a, ntris_a);
  CgalMesh b = cgal_mesh(verts_b, nverts_b, tris_b, ntris_b);
  CgalMesh out;
  bool ok = false;
  switch (op) {
    case BENCH_OP_DIFFERENCE:
      ok = PMP::corefine_and_compute_difference(a, b, out);
      break;
    case BENCH_OP_UNION:
      ok = PMP::corefine_and_compute_union(a, b, out);
      break;
    case BENCH_OP_INTERSECTION:
      ok = PMP::corefine_and_compute_intersection(a, b, out);
      break;
    default:
      return -1.0;
  }
  if (!ok) return -1.0;
  return CGAL::to_double(PMP::volume(out));
}

#endif  // HAS_CGAL
#ifdef HAS_OCCT
// OpenCascade: the B-rep incumbent. Boxes are exact solids here, not triangle
// soup, so this measures a genuinely different representation doing the same
// job. Built from source (see benchmarks/AGENTS.md): Debian's libocct-*-dev
// 7.8.1 ships Poly_ArrayOfNodes.hxx without its required
// NCollection_AliasedArray.hxx, so the packaged headers cannot compile.
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <BRepGProp.hxx>
#include <gp_Pnt.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <TopoDS.hxx>

static TopoDS_Shape occt_box(const double* mn, const double* mx) {
  return BRepPrimAPI_MakeBox(gp_Pnt(mn[0], mn[1], mn[2]),
                             gp_Pnt(mx[0], mx[1], mx[2])).Shape();
}

// A genuine B-rep solid from the 8 corners: six planar faces sewn into a
// shell. Built as exact geometry rather than a triangulation, which is the
// whole reason OpenCascade is in this comparison.
static TopoDS_Shape occt_hex(const double* c) {
  gp_Pnt p[8];
  for (int i = 0; i < 8; ++i) p[i] = gp_Pnt(c[i * 3], c[i * 3 + 1], c[i * 3 + 2]);
  static const int F[6][4] = {
      {0, 3, 2, 1},  // -Z
      {4, 5, 6, 7},  // +Z
      {0, 1, 5, 4},  // -Y
      {1, 2, 6, 5},  // +X
      {2, 3, 7, 6},  // +Y
      {3, 0, 4, 7},  // -X
  };
  BRepBuilderAPI_Sewing sew(1e-9);
  for (auto& q : F) {
    BRepBuilderAPI_MakePolygon poly(p[q[0]], p[q[1]], p[q[2]], p[q[3]], Standard_True);
    sew.Add(BRepBuilderAPI_MakeFace(poly.Wire()).Face());
  }
  sew.Perform();
  return BRepBuilderAPI_MakeSolid(TopoDS::Shell(sew.SewedShape())).Solid();
}

extern "C" double bench_occt_op(const double* host_min, const double* host_max,
                                const double* operand, int op) {
  TopoDS_Shape a = occt_box(host_min, host_max);
  TopoDS_Shape b = occt_hex(operand);
  TopoDS_Shape out;
  switch (op) {
    case BENCH_OP_DIFFERENCE: {
      BRepAlgoAPI_Cut o(a, b);
      if (!o.IsDone()) return -1.0;
      out = o.Shape();
      break;
    }
    case BENCH_OP_UNION: {
      BRepAlgoAPI_Fuse o(a, b);
      if (!o.IsDone()) return -1.0;
      out = o.Shape();
      break;
    }
    case BENCH_OP_INTERSECTION: {
      BRepAlgoAPI_Common o(a, b);
      if (!o.IsDone()) return -1.0;
      out = o.Shape();
      break;
    }
    default:
      return -1.0;
  }
  GProp_GProps props;
  BRepGProp::VolumeProperties(out, props);
  return props.Mass();
}

extern "C" double bench_occt_subtract(const double* host_min,
                                      const double* host_max,
                                      const double* cutters, int n) {
  TopoDS_Shape acc = occt_box(host_min, host_max);
  for (int i = 0; i < n; ++i) {
    BRepAlgoAPI_Cut cut(acc, occt_hex(cutters + i * 24));
    if (!cut.IsDone()) return -1.0;
    acc = cut.Shape();
  }
  GProp_GProps props;
  BRepGProp::VolumeProperties(acc, props);
  return props.Mass();
}
#endif  // HAS_OCCT
