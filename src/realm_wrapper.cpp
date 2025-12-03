#include "osu-collect/src/realm_bridge.rs.h"
#include <realm/db.hpp>
#include <realm/table.hpp>
#include <realm/transaction.hpp>
#include <realm/list.hpp>
#include <realm/history.hpp>
#include <unordered_map>
#include <unordered_set>
#include <iostream>
#include <fstream>

namespace osu_realm {

class RealmDB::Impl {
public:
    realm::DBRef db;

    Impl(const std::string& path) {
        try {
            realm::DBOptions options;
            options.allow_file_format_upgrade = false;
            auto history = realm::make_in_realm_history();
            db = realm::DB::create(std::move(history), path, options);
        } catch (const std::exception& e) {
            std::cerr << "Failed to open Realm: " << e.what() << std::endl;
            throw;
        }
    }
};

RealmDB::RealmDB(const std::string& path)
    : impl_(std::make_unique<Impl>(path)) {}

RealmDB::~RealmDB() = default;

rust::Vec<LocalBeatmapset> RealmDB::list_beatmapsets() const {
    rust::Vec<LocalBeatmapset> results;

    std::ofstream dbg("/tmp/realm_beatmapsets_debug.txt");

    try {
        auto tr = impl_->db->start_read();

        auto beatmap_table = tr->get_table("class_Beatmap");
        if (!beatmap_table) {
            std::cerr << "Table 'class_Beatmap' not found" << std::endl;
            dbg << "Table 'class_Beatmap' not found" << std::endl;
            return results;
        }

        auto beatmapset_table = tr->get_table("class_BeatmapSet");
        if (!beatmapset_table) {
            std::cerr << "Table 'class_BeatmapSet' not found" << std::endl;
            dbg << "Table 'class_BeatmapSet' not found" << std::endl;
            return results;
        }

        dbg << "Beatmap table rows: " << beatmap_table->size() << std::endl;
        dbg << "BeatmapSet table rows: " << beatmapset_table->size() << std::endl;

        // List columns
        dbg << "\nBeatmap table columns:" << std::endl;
        for (auto ck : beatmap_table->get_column_keys()) {
            dbg << "  - " << beatmap_table->get_column_name(ck) << std::endl;
        }
        dbg << "\nBeatmapSet table columns:" << std::endl;
        for (auto ck : beatmapset_table->get_column_keys()) {
            dbg << "  - " << beatmapset_table->get_column_name(ck) << std::endl;
        }

        auto bm_online_id_col = beatmap_table->get_column_key("OnlineID");
        auto bm_md5_col = beatmap_table->get_column_key("MD5Hash");
        auto bm_set_col = beatmap_table->get_column_key("BeatmapSet");
        auto bs_online_id_col = beatmapset_table->get_column_key("OnlineID");

        std::unordered_map<uint32_t, LocalBeatmapset> sets_map;
        std::unordered_set<std::string> all_local_checksums;  // ALL checksums, including skipped ones
        size_t skipped_no_beatmap_id = 0;
        size_t skipped_no_set_link = 0;
        size_t skipped_no_beatmapset_id = 0;
        size_t total_processed = 0;

        for (auto& obj : *beatmap_table) {
            total_processed++;

            // Collect checksum regardless of OnlineID
            auto md5_val = obj.get<realm::StringData>(bm_md5_col);
            std::string md5_str(md5_val.data(), md5_val.size());
            if (!md5_str.empty()) {
                all_local_checksums.insert(md5_str);
            }

            int64_t beatmap_id_raw = obj.get<int64_t>(bm_online_id_col);
            if (beatmap_id_raw <= 0) {
                skipped_no_beatmap_id++;
                continue;
            }
            uint32_t beatmap_id = static_cast<uint32_t>(beatmap_id_raw);

            rust::String md5_hash(md5_val.data(), md5_val.size());

            auto set_link = obj.get<realm::ObjKey>(bm_set_col);
            if (!set_link) {
                skipped_no_set_link++;
                continue;
            }

            auto set_obj = beatmapset_table->get_object(set_link);
            int64_t beatmapset_id_raw = set_obj.get<int64_t>(bs_online_id_col);
            if (beatmapset_id_raw <= 0) {
                skipped_no_beatmapset_id++;
                continue;
            }
            uint32_t beatmapset_id = static_cast<uint32_t>(beatmapset_id_raw);

            LocalBeatmap beatmap;
            beatmap.id = beatmap_id;
            beatmap.checksum = std::move(md5_hash);
            beatmap.beatmapset_id = beatmapset_id;

            auto it = sets_map.find(beatmapset_id);
            if (it == sets_map.end()) {
                LocalBeatmapset new_set;
                new_set.id = beatmapset_id;
                new_set.folder_name = rust::String("");
                new_set.beatmaps.push_back(std::move(beatmap));
                sets_map.emplace(beatmapset_id, std::move(new_set));
            } else {
                it->second.beatmaps.push_back(std::move(beatmap));
            }
        }

        dbg << "\nProcessing stats:" << std::endl;
        dbg << "  Total beatmaps processed: " << total_processed << std::endl;
        dbg << "  All unique checksums (including skipped): " << all_local_checksums.size() << std::endl;
        dbg << "  Skipped (no beatmap OnlineID): " << skipped_no_beatmap_id << std::endl;
        dbg << "  Skipped (no set link): " << skipped_no_set_link << std::endl;
        dbg << "  Skipped (no beatmapset OnlineID): " << skipped_no_beatmapset_id << std::endl;
        dbg << "  Total beatmapsets with valid IDs: " << sets_map.size() << std::endl;

        // Count total beatmaps
        size_t total_beatmaps = 0;
        for (const auto& [_, set] : sets_map) {
            total_beatmaps += set.beatmaps.size();
        }
        dbg << "  Total beatmaps in result: " << total_beatmaps << std::endl;

        // Write JSON data for external analysis
        std::ofstream json_out("/tmp/realm_beatmapsets.json");
        json_out << "{\n  \"beatmapsets\": {\n";
        bool first_set = true;
        for (const auto& [set_id, set] : sets_map) {
            if (!first_set) json_out << ",\n";
            first_set = false;
            json_out << "    \"" << set_id << "\": {\n";
            json_out << "      \"id\": " << set_id << ",\n";
            json_out << "      \"beatmaps\": [\n";
            bool first_bm = true;
            for (const auto& bm : set.beatmaps) {
                if (!first_bm) json_out << ",\n";
                first_bm = false;
                json_out << "        {\"id\": " << bm.id
                         << ", \"checksum\": \"" << std::string(bm.checksum.data(), bm.checksum.size()) << "\"}";
            }
            json_out << "\n      ]\n    }";
        }
        json_out << "\n  },\n";

        // Write all checksums (only from valid beatmapsets, for comparison)
        json_out << "  \"all_checksums\": [\n";
        bool first_cs = true;
        for (const auto& [_, set] : sets_map) {
            for (const auto& bm : set.beatmaps) {
                if (!first_cs) json_out << ",\n";
                first_cs = false;
                json_out << "    \"" << std::string(bm.checksum.data(), bm.checksum.size()) << "\"";
            }
        }
        json_out << "\n  ],\n";

        // Write ALL checksums including from skipped beatmaps
        json_out << "  \"all_checksums_including_skipped\": [\n";
        first_cs = true;
        for (const auto& cs : all_local_checksums) {
            if (!first_cs) json_out << ",\n";
            first_cs = false;
            json_out << "    \"" << cs << "\"";
        }
        json_out << "\n  ]\n}\n";
        json_out.close();
        dbg << "  Wrote JSON data to /tmp/realm_beatmapsets.json" << std::endl;

        for (auto& [_, set] : sets_map) {
            results.push_back(std::move(set));
        }

    } catch (const std::exception& e) {
        std::cerr << "Error reading beatmapsets: " << e.what() << std::endl;
        dbg << "Error: " << e.what() << std::endl;
    }

    return results;
}

rust::Vec<LocalCollection> RealmDB::list_collections() const {
    rust::Vec<LocalCollection> results;

    std::ofstream dbg("/tmp/realm_debug.txt");

    try {
        auto tr = impl_->db->start_read();

        dbg << "Available tables in database:" << std::endl;
        for (auto tk : tr->get_table_keys()) {
            auto t = tr->get_table(tk);
            dbg << "  - " << t->get_name() << " (rows: " << t->size() << ")" << std::endl;
        }

        auto collection_table = tr->get_table("class_BeatmapCollection");
        if (!collection_table) {
            dbg << "Table 'class_BeatmapCollection' NOT FOUND" << std::endl;
            return results;
        }

        dbg << "Collection table found with " << collection_table->size() << " rows" << std::endl;

        // List all columns in the collection table
        dbg << "Columns in collection table:" << std::endl;
        for (auto ck : collection_table->get_column_keys()) {
            dbg << "  - " << collection_table->get_column_name(ck) << std::endl;
        }

        if (collection_table->size() == 0) {
            dbg << "Table is empty, returning" << std::endl;
            return results;
        }

        auto name_col = collection_table->get_column_key("Name");
        auto hashes_col = collection_table->get_column_key("BeatmapMD5Hashes");

        if (!name_col || !hashes_col) {
            dbg << "Missing columns: Name=" << (name_col ? "ok" : "missing")
                << ", BeatmapMD5Hashes=" << (hashes_col ? "ok" : "missing") << std::endl;
            return results;
        }

        // Check the column type
        auto hashes_col_type = collection_table->get_column_type(hashes_col);
        dbg << "BeatmapMD5Hashes column type: " << static_cast<int>(hashes_col_type) << std::endl;

        for (auto& obj : *collection_table) {
            auto name_val = obj.get<realm::StringData>(name_col);
            dbg << "Reading collection: " << std::string(name_val.data(), name_val.size()) << std::endl;

            LocalCollection collection;
            collection.name = rust::String(name_val.data(), name_val.size());

            // Try to read hashes based on column type
            try {
                auto hashes_list = obj.get_list<realm::StringData>(hashes_col);
                dbg << "  Hashes count (list<string>): " << hashes_list.size() << std::endl;
                for (size_t j = 0; j < hashes_list.size(); ++j) {
                    auto hash_val = hashes_list.get(j);
                    collection.beatmap_checksums.push_back(
                        rust::String(hash_val.data(), hash_val.size())
                    );
                }
            } catch (const std::exception& e) {
                dbg << "  Failed to read as list<string>: " << e.what() << std::endl;
                // Try linklist approach
                try {
                    auto hashes_list = obj.get_linklist(hashes_col);
                    dbg << "  Hashes count (linklist): " << hashes_list.size() << std::endl;
                    for (size_t j = 0; j < hashes_list.size(); ++j) {
                        auto hash_key = hashes_list.get(j);
                        auto hash_table = hashes_list.get_target_table();
                        auto hash_obj = hash_table->get_object(hash_key);
                        auto value_col = hash_table->get_column_key("value");
                        auto hash_val = hash_obj.get<realm::StringData>(value_col);
                        collection.beatmap_checksums.push_back(
                            rust::String(hash_val.data(), hash_val.size())
                        );
                    }
                } catch (const std::exception& e2) {
                    dbg << "  Failed to read as linklist: " << e2.what() << std::endl;
                }
            }

            results.push_back(std::move(collection));
        }

        dbg << "Total collections read: " << results.size() << std::endl;

    } catch (const std::exception& e) {
        std::cerr << "[realm-cpp] Error reading collections: " << e.what() << std::endl;
    }

    return results;
}

rust::Vec<rust::String> RealmDB::list_all_checksums() const {
    rust::Vec<rust::String> results;

    try {
        auto tr = impl_->db->start_read();

        auto beatmap_table = tr->get_table("class_Beatmap");
        if (!beatmap_table) {
            return results;
        }

        auto md5_col = beatmap_table->get_column_key("MD5Hash");

        for (auto& obj : *beatmap_table) {
            auto md5_val = obj.get<realm::StringData>(md5_col);
            if (md5_val.size() > 0) {
                results.push_back(rust::String(md5_val.data(), md5_val.size()));
            }
        }

    } catch (const std::exception& e) {
        std::cerr << "Error reading all checksums: " << e.what() << std::endl;
    }

    return results;
}

std::unique_ptr<RealmDB> open_realm(rust::Str path) {
    std::string path_str(path.data(), path.size());
    return std::make_unique<RealmDB>(path_str);
}

} // namespace osu_realm
