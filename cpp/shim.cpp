#ifdef HAS_MANIFOLD
#include <manifold/manifold.h>
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

#endif  // HAS_CGAL
#ifdef HAS_OCCT
// OpenCascade: the B-rep incumbent. Boxes are exact solids here, not triangle
// soup, so this measures a genuinely different representation doing the same
// job. Built from source (see benchmarks/AGENTS.md): Debian's libocct-*-dev
// 7.8.1 ships Poly_ArrayOfNodes.hxx without its required
// NCollection_AliasedArray.hxx, so the packaged headers cannot compile.
#include <BRepAlgoAPI_Cut.hxx>
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
