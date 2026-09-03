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

extern "C" double bench_manifold_subtract(const double* host_min,
                                          const double* host_max,
                                          const double* cutters, int n) {
  Manifold acc = box_of(host_min, host_max);
  for (int i = 0; i < n; ++i) {
    const double* c = cutters + i * 6;
    acc = acc.Boolean(box_of(c, c + 3), OpType::Subtract);
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

extern "C" double bench_cgal_subtract(const double* host_min,
                                      const double* host_max,
                                      const double* cutters, int n) {
  CgalMesh acc = cgal_box(host_min, host_max);
  for (int i = 0; i < n; ++i) {
    const double* c = cutters + i * 6;
    CgalMesh tool = cgal_box(c, c + 3), out;
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

static TopoDS_Shape occt_box(const double* mn, const double* mx) {
  return BRepPrimAPI_MakeBox(gp_Pnt(mn[0], mn[1], mn[2]),
                             gp_Pnt(mx[0], mx[1], mx[2])).Shape();
}

extern "C" double bench_occt_subtract(const double* host_min,
                                      const double* host_max,
                                      const double* cutters, int n) {
  TopoDS_Shape acc = occt_box(host_min, host_max);
  for (int i = 0; i < n; ++i) {
    const double* c = cutters + i * 6;
    BRepAlgoAPI_Cut cut(acc, occt_box(c, c + 3));
    if (!cut.IsDone()) return -1.0;
    acc = cut.Shape();
  }
  GProp_GProps props;
  BRepGProp::VolumeProperties(acc, props);
  return props.Mass();
}
#endif  // HAS_OCCT
