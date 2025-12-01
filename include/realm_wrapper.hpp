#pragma once

#include <memory>
#include <string>
#include <vector>
#include <cstdint>
#include "rust/cxx.h"

namespace osu_realm {

// These structs are defined by CXX bridge, we only forward-declare here
struct LocalBeatmap;
struct LocalBeatmapset;
struct LocalCollection;

class RealmDB {
public:
    RealmDB(const std::string& path);
    ~RealmDB();

    RealmDB(const RealmDB&) = delete;
    RealmDB& operator=(const RealmDB&) = delete;

    rust::Vec<LocalBeatmapset> list_beatmapsets() const;
    rust::Vec<LocalCollection> list_collections() const;

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

std::unique_ptr<RealmDB> open_realm(rust::Str path);

} // namespace osu_realm
