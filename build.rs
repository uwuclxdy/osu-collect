use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/realm_wrapper.cpp");
    println!("cargo:rerun-if-changed=include/realm_wrapper.hpp");
    println!("cargo:rerun-if-changed=src/realm_bridge.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let realm_core_dir = manifest_dir.join("vendor/realm-cpp/realm-core");

    let target = env::var("TARGET").unwrap_or_default();
    let is_msvc = target.contains("msvc");

    // Build realm-core only (skip cpprealm SDK which has compiler issues)
    let mut cmake_config = cmake::Config::new(&realm_core_dir);
    cmake_config
        .define("REALM_BUILD_LIB_ONLY", "ON")
        .define("REALM_ENABLE_SYNC", "OFF")
        .define("REALM_NO_TESTS", "ON")
        .define("BUILD_TESTING", "OFF")
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("CMAKE_CXX_STANDARD", "17");

    if is_msvc {
        // Use dynamic CRT (/MD) to match Rust's default runtime linkage
        // Without this, realm-core builds with static CRT (/MT) causing LNK2038 mismatch
        cmake_config.define("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreadedDLL");
    } else {
        cmake_config.cxxflag("-Wno-error");
    }

    let realm_build = cmake_config.build();

    let realm_lib_dir = realm_build.join("lib");
    let realm_include_dir = realm_core_dir.join("src");

    // On Windows, realm-core downloads zlib to its build directory
    let zlib_dir = realm_build.join("build/zlib/lib");
    if zlib_dir.exists() {
        println!("cargo:rustc-link-search=native={}", zlib_dir.display());
    }

    println!("cargo:rustc-link-search=native={}", realm_lib_dir.display());
    println!("cargo:rustc-link-lib=static=realm");
    println!("cargo:rustc-link-lib=static=realm-parser");

    // Link system dependencies based on target OS
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    match target_os.as_str() {
        "linux" => {
            println!("cargo:rustc-link-lib=dylib=stdc++");
            println!("cargo:rustc-link-lib=dylib=pthread");
            println!("cargo:rustc-link-lib=dylib=z");
        }
        "macos" => {
            println!("cargo:rustc-link-lib=dylib=c++");
            println!("cargo:rustc-link-lib=framework=Security");
            println!("cargo:rustc-link-lib=framework=Foundation");
        }
        "windows" => {
            println!("cargo:rustc-link-lib=dylib=bcrypt");
            println!("cargo:rustc-link-lib=dylib=ws2_32");
            println!("cargo:rustc-link-lib=dylib=crypt32");
            println!("cargo:rustc-link-lib=dylib=advapi32");
            println!("cargo:rustc-link-lib=dylib=version");
            // Link zlib (downloaded by realm-core on Windows)
            println!("cargo:rustc-link-lib=static=zlib");
        }
        _ => {}
    }

    // Build our C++ wrapper with cxx
    let mut cxx_builder = cxx_build::bridge("src/realm_bridge.rs");
    cxx_builder
        .file("src/realm_wrapper.cpp")
        .include(&realm_include_dir)
        .include(realm_build.join("include"))
        .include("include");

    if is_msvc {
        cxx_builder
            .flag("/std:c++17")
            .flag("/EHsc") // Enable C++ exception handling
            .define("WIN32_LEAN_AND_MEAN", None) // Prevent winsock.h inclusion (use winsock2.h)
            .define("NOMINMAX", None); // Prevent min/max macro conflicts
    } else {
        cxx_builder
            .flag_if_supported("-std=c++17")
            .flag_if_supported("-Wno-unused-parameter")
            .flag_if_supported("-Wno-error");
    }

    cxx_builder.compile("osu_realm_bridge");

    println!("cargo:rerun-if-changed=vendor/realm-cpp/realm-core");
}
