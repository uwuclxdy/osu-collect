use crate::collector::Collection;
use crate::error::{AppError, Result};
use crate::utils::sanitize_filename;
use osu_db::collection::{Collection as DbCollection, CollectionList};
use std::path::Path;
use std::fs;
use std::io::Write;

const OSU_DB_VERSION: u32 = 20211103;

/// Create collection.db file from collection data
pub fn create_collection_db(
    collection: &Collection,
    collection_name: &str,
    output_dir: &Path,
) -> Result<()> {
    let db_path = output_dir.join("collection.db");

    let beatmap_hashes: Vec<Option<String>> = collection
        .beatmapsets
        .iter()
        .flat_map(|beatmapset| {
            beatmapset
                .beatmaps
                .iter()
                .map(|beatmap| Some(beatmap.checksum.to_string()))
        })
        .collect();

    let db_collection = DbCollection {
        name: Some(collection_name.to_string()),
        beatmap_hashes,
    };

    let collection_list = CollectionList {
        version: OSU_DB_VERSION,
        collections: vec![db_collection],
    };

    // Create a custom implementation to write the collection to a file
    let mut file = fs::File::create(&db_path).map_err(|e| AppError::other_dynamic(
        format!("Failed to create collection.db file: {}", e).into_boxed_str()
    ))?;

    // First write the version
    file.write_all(&collection_list.version.to_le_bytes()).map_err(|e| AppError::other_dynamic(
        format!("Failed to write version to collection.db: {}", e).into_boxed_str()
    ))?;

    // Write number of collections
    let num_collections = collection_list.collections.len() as u32;
    file.write_all(&num_collections.to_le_bytes()).map_err(|e| AppError::other_dynamic(
        format!("Failed to write collection count to collection.db: {}", e).into_boxed_str()
    ))?;

    // Write each collection
    for collection in &collection_list.collections {
        // Write collection name (string)
        let name = collection.name.as_ref().ok_or_else(|| AppError::other_dynamic(
            "Collection name cannot be empty"
        ))?;
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len() as u32;
        file.write_all(&name_len.to_le_bytes()).map_err(|e| AppError::other_dynamic(
            format!("Failed to write collection name length: {}", e).into_boxed_str()
        ))?;
        file.write_all(name_bytes).map_err(|e| AppError::other_dynamic(
            format!("Failed to write collection name: {}", e).into_boxed_str()
        ))?;

        // Write number of beatmaps
        let num_beatmaps = collection.beatmap_hashes.len() as u32;
        file.write_all(&num_beatmaps.to_le_bytes()).map_err(|e| AppError::other_dynamic(
            format!("Failed to write beatmap count: {}", e).into_boxed_str()
        ))?;

        // Write each beatmap hash
        for hash_opt in &collection.beatmap_hashes {
            let hash = hash_opt.as_deref().unwrap();
            let hash_bytes = hash.as_bytes();
            let hash_len = hash_bytes.len() as u32;
            file.write_all(&hash_len.to_le_bytes()).map_err(|e| AppError::other_dynamic(
                format!("Failed to write hash length: {}", e).into_boxed_str()
            ))?;
            file.write_all(hash_bytes).map_err(|e| AppError::other_dynamic(
                format!("Failed to write hash: {}", e).into_boxed_str()
            ))?;
        }
    }

    Ok(())
}

/// Generate collection folder name
#[inline]
pub fn generate_collection_folder_name(collection: &Collection) -> String {
    let sanitized_name = sanitize_filename(&collection.name);
    format!("{}-{}", sanitized_name, collection.id)
}
