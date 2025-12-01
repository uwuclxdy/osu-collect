use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/realm_wrapper.cpp");
    println!("cargo:rerun-if-changed=include/realm_wrapper.hpp");
    println!("cargo:rerun-if-changed=src/realm_bridge.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let realm_core_dir = manifest_dir.join("vendor/realm-cpp/realm-core");

    // Build realm-core only (skip cpprealm SDK which has compiler issues)
    let realm_build = cmake::Config::new(&realm_core_dir)
        .define("REALM_BUILD_LIB_ONLY", "ON")
        .define("REALM_ENABLE_SYNC", "OFF")
        .define("REALM_NO_TESTS", "ON")
        .define("BUILD_TESTING", "OFF")
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("CMAKE_CXX_STANDARD", "17")
        .cxxflag("-Wno-error")
        .build();

    let realm_lib_dir = realm_build.join("lib");
    let realm_include_dir = realm_core_dir.join("src");

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
        }
        _ => {}
    }

    // Build our C++ wrapper with cxx
    cxx_build::bridge("src/realm_bridge.rs")
        .file("src/realm_wrapper.cpp")
        .include(&realm_include_dir)
        .include(realm_build.join("include"))
        .include("include")
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-error")
        .compile("osu_realm_bridge");

    println!("cargo:rerun-if-changed=vendor/realm-cpp/realm-core");
}
