//! Builds the C++ kernel shims (`cpp/shim.cpp`).
//!
//! Manifold and OCCT live outside the system prefix, so their locations are
//! env-overridable rather than hardcoded -- a clone on another host sets
//! MANIFOLD_DIR / OCCT_DIR instead of editing this file. Each kernel is
//! compiled only if its headers are actually present, and the matching
//! `has_*` cfg is emitted so the Rust side never links a kernel that is not
//! there (an absent kernel must be a missing COLUMN, never a fake number).

use std::path::Path;

fn main() {
    // No hardcoded fallbacks: a machine-specific default leaks local paths into
    // a public repo and silently builds the wrong thing on another machine.
    // Unset means "kernel absent", which the cfg gates below already handle by
    // omitting the column entirely rather than reporting a fake number.
    let manifold = std::env::var("MANIFOLD_DIR").unwrap_or_default();
    let occt = std::env::var("OCCT_DIR").unwrap_or_default();

    let has_manifold = Path::new(&format!("{manifold}/include/manifold/manifold.h")).exists();
    let has_occt = Path::new(&format!("{occt}/include/opencascade/BRepAlgoAPI_Cut.hxx")).exists();
    // CGAL is header-only for our use and ships in the system prefix.
    let has_cgal = Path::new("/usr/include/CGAL/Polygon_mesh_processing/corefinement.h").exists();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .opt_level(2)
        .file("cpp/shim.cpp");
    build.flag_if_supported("-w");

    if has_manifold {
        build
            .include(format!("{manifold}/include"))
            .define("HAS_MANIFOLD", None);
        println!("cargo:rustc-cfg=has_manifold");
        println!("cargo:rustc-link-search=native={manifold}/lib");
        println!("cargo:rustc-link-lib=dylib=manifold");
        // rpath: these libs live outside the system prefix, so the binary must
        // carry its own search path or it cannot start.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{manifold}/lib");
    }
    if has_cgal {
        build.define("HAS_CGAL", None);
        println!("cargo:rustc-cfg=has_cgal");
        println!("cargo:rustc-link-lib=dylib=gmp");
        println!("cargo:rustc-link-lib=dylib=mpfr");
    }
    if has_occt {
        build
            .include(format!("{occt}/include/opencascade"))
            .define("HAS_OCCT", None);
        println!("cargo:rustc-cfg=has_occt");
        println!("cargo:rustc-link-search=native={occt}/lib");
        // RPATH (not RUNPATH): OCCT's own libs reference each other, and only
        // RPATH is inherited when resolving those transitive deps.
        println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{occt}/lib");
        for l in [
            "TKernel",
            "TKMath",
            "TKG2d",
            "TKG3d",
            "TKGeomBase",
            "TKBRep",
            "TKGeomAlgo",
            "TKTopAlgo",
            "TKPrim",
            "TKBO",
        ] {
            println!("cargo:rustc-link-lib=dylib={l}");
        }
    }

    build.compile("kernel_shim");
    println!("cargo:rerun-if-changed=cpp/shim.cpp");
    println!("cargo:rerun-if-env-changed=MANIFOLD_DIR");
    println!("cargo:rerun-if-env-changed=OCCT_DIR");
    println!("cargo:rustc-check-cfg=cfg(has_manifold, has_cgal, has_occt)");
}
