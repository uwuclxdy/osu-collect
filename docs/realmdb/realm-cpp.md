# Comprehensive Guide: Integrating realm-cpp into Rust (Vendored Setup)

## Overview

This guide will help you integrate realm-cpp (community branch) into your Rust project with everything vendored in your repository.

## Directory Structure

```
your-project/
├── Cargo.toml
├── build.rs
├── vendor/
│   └── realm-cpp/          # Git submodule or vendored copy
├── src/
│   ├── lib.rs
│   ├── realm_bridge.rs     # CXX bridge definition
│   └── realm_wrapper.cpp   # C++ implementation
├── include/
│   └── realm_wrapper.hpp   # C++ header
└── .github/
    └── workflows/
        └── ci.yml          # Your existing CI
```

## Step 1: Add realm-cpp to Your Repository

### Option A: Git Submodule (Recommended)

```bash
cd your-project
mkdir -p vendor
git submodule add -b community https://github.com/realm/realm-cpp.git vendor/realm-cpp
git submodule update --init --recursive
```

### Option B: Vendored Copy

```bash
cd your-project
mkdir -p vendor
cd vendor
git clone -b community https://github.com/realm/realm-cpp.git
cd realm-cpp
git submodule update --init --recursive
# Remove .git to vendor it completely
rm -rf .git
```

## Step 2: Update Cargo.toml

```toml
[package]
name = "osu-collect"
version = "0.2.0"
edition = "2024"

[dependencies]
cxx = "1.0"
anyhow = "1.0"

[build-dependencies]
cxx-build = "1.0"
cmake = "0.1"

[lib]
crate-type = ["lib"]

# If you want a binary as well
[[bin]]
name = "osu-collect"
path = "src/main.rs"
```

## Step 3: Create build.rs

This will build realm-cpp and link everything together:

```rust
use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/realm_wrapper.cpp");
    println!("cargo:rerun-if-changed=include/realm_wrapper.hpp");
    println!("cargo:rerun-if-changed=src/realm_bridge.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let realm_cpp_dir = manifest_dir.join("vendor/realm-cpp");
    
    // Build realm-cpp using CMake
    let realm_build = cmake::Config::new(&realm_cpp_dir)
        .define("REALM_BUILD_LIB_ONLY", "ON")
        .define("REALM_ENABLE_SYNC", "OFF")
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("CMAKE_CXX_STANDARD", "17")
        .build();

    let realm_lib_dir = realm_build.join("lib");
    let realm_include_dir = realm_cpp_dir.join("src");

    println!("cargo:rustc-link-search=native={}", realm_lib_dir.display());
    println!("cargo:rustc-link-lib=static=realm-object-store");
    println!("cargo:rustc-link-lib=static=realm");
    println!("cargo:rustc-link-lib=static=realm-parser");
    println!("cargo:rustc-link-lib=static=realm-sync");
    
    // Link system dependencies
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=pthread");
        println!("cargo:rustc-link-lib=dylib=z");
    } else if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=dylib=c++");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=Foundation");
    } else if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=dylib=bcrypt");
        println!("cargo:rustc-link-lib=dylib=ws2_32");
        println!("cargo:rustc-link-lib=dylib=crypt32");
    }

    // Build our C++ wrapper with cxx
    cxx_build::bridge("src/realm_bridge.rs")
        .file("src/realm_wrapper.cpp")
        .include(&realm_include_dir)
        .include("include")
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("osu_realm_bridge");

    println!("cargo:rerun-if-changed=vendor/realm-cpp");
}
```

## Step 4: Create C++ Wrapper Header

Create `include/realm_wrapper.hpp`:

```cpp
#pragma once

#include <memory>
#include <string>
#include <vector>
#include <cstdint>
#include "rust/cxx.h"

namespace osu_realm {

// Forward declarations
struct BeatmapSet;
struct Beatmap;

// Opaque handle for Realm database
class RealmDB {
public:
    RealmDB(const std::string& path);
    ~RealmDB();

    // Prevent copying
    RealmDB(const RealmDB&) = delete;
    RealmDB& operator=(const RealmDB&) = delete;

    // Query methods
    std::vector<BeatmapSet> get_all_beatmapsets() const;
    std::unique_ptr<BeatmapSet> get_beatmapset_by_id(int32_t id) const;
    size_t get_beatmapset_count() const;

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

// Data structures matching osu!lazer schema
struct BeatmapSet {
    int32_t online_id;
    rust::String title;
    rust::String artist;
    rust::String creator;
    bool deleted_pending;
    std::vector<Beatmap> beatmaps;
};

struct Beatmap {
    int32_t online_id;
    rust::String difficulty_name;
    double star_rating;
    double bpm;
    double length;
    int32_t max_combo;
};

// Factory function for Rust
std::unique_ptr<RealmDB> open_realm(rust::Str path);

} // namespace osu_realm
```

## Step 5: Implement C++ Wrapper

Create `src/realm_wrapper.cpp`:

```cpp
#include "realm_wrapper.hpp"
#include <cpprealm/sdk.hpp>
#include <iostream>

namespace osu_realm {

// Internal implementation
class RealmDB::Impl {
public:
    realm::db_config config;
    std::shared_ptr<realm::realm> db;

    Impl(const std::string& path) {
        config.path = path;
        // Open in read-only mode
        config.schema_mode = realm::schema_mode::read_only;
        
        try {
            db = realm::realm::get_shared_realm(config);
        } catch (const std::exception& e) {
            std::cerr << "Failed to open Realm: " << e.what() << std::endl;
            throw;
        }
    }
};

RealmDB::RealmDB(const std::string& path)
    : impl_(std::make_unique<Impl>(path)) {}

RealmDB::~RealmDB() = default;

std::vector<BeatmapSet> RealmDB::get_all_beatmapsets() const {
    std::vector<BeatmapSet> results;
    
    try {
        auto table = impl_->db->read_group().get_table("BeatmapSet");
        if (!table) {
            std::cerr << "Table 'BeatmapSet' not found" << std::endl;
            return results;
        }

        for (auto it = table->begin(); it != table->end(); ++it) {
            BeatmapSet set;
            
            // Read basic properties
            // Note: Column names may vary - check your actual schema
            auto online_id_col = table->get_column_key("OnlineBeatmapSetID");
            if (online_id_col) {
                set.online_id = it->get<int64_t>(online_id_col);
            }

            auto title_col = table->get_column_key("Title");
            if (title_col) {
                auto title = it->get<realm::StringData>(title_col);
                set.title = rust::String(title.data(), title.size());
            }

            auto artist_col = table->get_column_key("Artist");
            if (artist_col) {
                auto artist = it->get<realm::StringData>(artist_col);
                set.artist = rust::String(artist.data(), artist.size());
            }

            auto creator_col = table->get_column_key("Creator");
            if (creator_col) {
                auto creator = it->get<realm::StringData>(creator_col);
                set.creator = rust::String(creator.data(), creator.size());
            }

            auto deleted_col = table->get_column_key("DeletePending");
            if (deleted_col) {
                set.deleted_pending = it->get<bool>(deleted_col);
            }

            results.push_back(std::move(set));
        }
    } catch (const std::exception& e) {
        std::cerr << "Error reading beatmapsets: " << e.what() << std::endl;
    }

    return results;
}

std::unique_ptr<BeatmapSet> RealmDB::get_beatmapset_by_id(int32_t id) const {
    try {
        auto table = impl_->db->read_group().get_table("BeatmapSet");
        if (!table) {
            return nullptr;
        }

        auto online_id_col = table->get_column_key("OnlineBeatmapSetID");
        if (!online_id_col) {
            return nullptr;
        }

        auto query = table->query("OnlineBeatmapSetID == $0", id);
        auto results = query.find_all();
        
        if (results.size() == 0) {
            return nullptr;
        }

        auto obj = table->get_object(results[0]);
        auto set = std::make_unique<BeatmapSet>();
        
        set->online_id = obj.get<int64_t>(online_id_col);
        
        auto title_col = table->get_column_key("Title");
        if (title_col) {
            auto title = obj.get<realm::StringData>(title_col);
            set->title = rust::String(title.data(), title.size());
        }

        // Add more fields as needed...

        return set;
    } catch (const std::exception& e) {
        std::cerr << "Error finding beatmapset: " << e.what() << std::endl;
        return nullptr;
    }
}

size_t RealmDB::get_beatmapset_count() const {
    try {
        auto table = impl_->db->read_group().get_table("BeatmapSet");
        return table ? table->size() : 0;
    } catch (const std::exception& e) {
        std::cerr << "Error getting count: " << e.what() << std::endl;
        return 0;
    }
}

std::unique_ptr<RealmDB> open_realm(rust::Str path) {
    std::string path_str(path.data(), path.size());
    return std::make_unique<RealmDB>(path_str);
}

} // namespace osu_realm
```

## Step 6: Create CXX Bridge

Create `src/realm_bridge.rs`:

```rust
#[cxx::bridge(namespace = "osu_realm")]
pub mod ffi {
    // Shared structs
    #[derive(Debug, Clone)]
    pub struct BeatmapSet {
        pub online_id: i32,
        pub title: String,
        pub artist: String,
        pub creator: String,
        pub deleted_pending: bool,
        pub beatmaps: Vec<Beatmap>,
    }

    #[derive(Debug, Clone)]
    pub struct Beatmap {
        pub online_id: i32,
        pub difficulty_name: String,
        pub star_rating: f64,
        pub bpm: f64,
        pub length: f64,
        pub max_combo: i32,
    }

    unsafe extern "C++" {
        include!("realm_wrapper.hpp");

        // Opaque C++ type
        type RealmDB;

        // Constructor
        fn open_realm(path: &str) -> Result<UniquePtr<RealmDB>>;

        // Query methods
        fn get_all_beatmapsets(self: &RealmDB) -> Vec<BeatmapSet>;
        fn get_beatmapset_by_id(self: &RealmDB, id: i32) -> UniquePtr<BeatmapSet>;
        fn get_beatmapset_count(self: &RealmDB) -> usize;
    }
}
```

## Step 7: Create Safe Rust API

Create `src/lib.rs`:

```rust
mod realm_bridge;

use anyhow::{Context, Result};
use cxx::UniquePtr;

pub use realm_bridge::ffi::{Beatmap, BeatmapSet};

/// Safe Rust wrapper for osu!lazer Realm database
pub struct OsuRealmDB {
    db: UniquePtr<realm_bridge::ffi::RealmDB>,
}

impl OsuRealmDB {
    /// Open an osu!lazer Realm database file in read-only mode
    ///
    /// # Arguments
    /// * `path` - Path to the client.realm file
    ///
    /// # Example
    /// ```no_run
    /// use osu_collect::OsuRealmDB;
    ///
    /// let db = OsuRealmDB::open("/path/to/osu-lazer/client.realm")?;
    /// ```
    pub fn open(path: impl AsRef<str>) -> Result<Self> {
        let db = realm_bridge::ffi::open_realm(path.as_ref())
            .context("Failed to open Realm database")?;
        
        Ok(Self { db })
    }

    /// Get all beatmapsets in the database
    pub fn get_all_beatmapsets(&self) -> Vec<BeatmapSet> {
        self.db.get_all_beatmapsets()
    }

    /// Get a specific beatmapset by its online ID
    pub fn get_beatmapset_by_id(&self, id: i32) -> Option<BeatmapSet> {
        let ptr = self.db.get_beatmapset_by_id(id);
        if ptr.is_null() {
            None
        } else {
            // Convert UniquePtr to owned value
            Some(unsafe { *Box::from_raw(ptr.into_raw()) })
        }
    }

    /// Get the total number of beatmapsets
    pub fn beatmapset_count(&self) -> usize {
        self.db.get_beatmapset_count()
    }

    /// Get all beatmapsets, filtered by a predicate
    pub fn filter_beatmapsets<F>(&self, predicate: F) -> Vec<BeatmapSet>
    where
        F: Fn(&BeatmapSet) -> bool,
    {
        self.get_all_beatmapsets()
            .into_iter()
            .filter(predicate)
            .collect()
    }
}

// Ensure thread safety
unsafe impl Send for OsuRealmDB {}
unsafe impl Sync for OsuRealmDB {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires actual realm file
    fn test_open_database() {
        let db = OsuRealmDB::open("test_data/client.realm").unwrap();
        assert!(db.beatmapset_count() > 0);
    }
}
```

## Step 8: Create Example Binary

Create `src/main.rs`:

```rust
use anyhow::Result;
use osu_collect::OsuRealmDB;
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        eprintln!("Usage: {} <path-to-client.realm>", args[0]);
        std::process::exit(1);
    }

    let realm_path = &args[1];
    println!("Opening Realm database: {}", realm_path);

    let db = OsuRealmDB::open(realm_path)?;
    
    let count = db.beatmapset_count();
    println!("Total beatmapsets: {}", count);

    // Get first 10 beatmapsets
    let beatmapsets = db.get_all_beatmapsets();
    for (i, set) in beatmapsets.iter().take(10).enumerate() {
        println!(
            "{}. {} - {} (by {}) [ID: {}]",
            i + 1,
            set.artist,
            set.title,
            set.creator,
            set.online_id
        );
    }

    // Filter example: get all deleted beatmapsets
    let deleted = db.filter_beatmapsets(|set| set.deleted_pending);
    println!("\nDeleted beatmapsets: {}", deleted.len());

    Ok(())
}
```

## Step 9: GitHub CI Configuration

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

jobs:
  build:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        rust: [stable]

    runs-on: ${{ matrix.os }}

    steps:
    - uses: actions/checkout@v3
      with:
        submodules: recursive

    - name: Install Rust
      uses: dtolnay/rust-toolchain@stable
      with:
        toolchain: ${{ matrix.rust }}

    - name: Install dependencies (Ubuntu)
      if: matrix.os == 'ubuntu-latest'
      run: |
        sudo apt-get update
        sudo apt-get install -y cmake ninja-build libssl-dev

    - name: Install dependencies (macOS)
      if: matrix.os == 'macos-latest'
      run: |
        brew install cmake ninja

    - name: Cache cargo registry
      uses: actions/cache@v3
      with:
        path: ~/.cargo/registry
        key: ${{ runner.os }}-cargo-registry-${{ hashFiles('**/Cargo.lock') }}

    - name: Cache cargo index
      uses: actions/cache@v3
      with:
        path: ~/.cargo/git
        key: ${{ runner.os }}-cargo-index-${{ hashFiles('**/Cargo.lock') }}

    - name: Cache cargo build
      uses: actions/cache@v3
      with:
        path: target
        key: ${{ runner.os }}-cargo-build-target-${{ hashFiles('**/Cargo.lock') }}

    - name: Build
      run: cargo build --release --verbose

    - name: Run tests
      run: cargo test --verbose

    - name: Upload artifacts
      uses: actions/upload-artifact@v3
      with:
        name: osu-collect-${{ matrix.os }}
        path: |
          target/release/osu-collect*
          !target/release/*.d
```

## Step 10: Build and Test

```bash
# Initialize submodules (if using submodule approach)
git submodule update --init --recursive

# Build the project (first build will take 5-10 minutes)
cargo build --release

# Test with your osu!lazer database
./target/release/osu-collect ~/path/to/osu-lazer/client.realm
```

**3. Linking errors on Linux:**
Add to `build.rs`:
```rust
println!("cargo:rustc-link-lib=dylib=ssl");
println!("cargo:rustc-link-lib=dylib=crypto");
```

**4. Schema mismatch errors:**
The osu!lazer schema may differ from the example. Inspect with Realm Studio to get exact field names, then update `realm_wrapper.cpp` accordingly.

## Next Steps

1. **Inspect the actual schema**: Use Realm Studio to open your `client.realm` file and see the exact table and column names
2. **Update wrapper code**: Modify `realm_wrapper.cpp` to match the actual osu!lazer schema
3. **Add more queries**: Extend the C++ wrapper with queries you need for osu-collect
4. **Optimize**: The initial implementation loads everything into memory - add pagination/streaming for large datasets
