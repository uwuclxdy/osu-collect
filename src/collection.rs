use crate::collector::Collection;
use crate::error::{AppError, Result};
use crate::utils::sanitize_filename;
use osu_db::collection::{Collection as DbCollection, CollectionList};
use std::path::Path;

const OSU_DB_VERSION: u32 = 20150203;

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

    collection_list.to_file(&db_path).map_err(|e| {
        AppError::other_dynamic(
            format!("Failed to write collection.db: {}", e).into_boxed_str()
        )
    })?;

    Ok(())
}

/// Generate collection folder name
#[inline]
pub fn generate_collection_folder_name(collection: &Collection) -> String {
    let sanitized_name = sanitize_filename(&collection.name);
    format!("{}-{}", sanitized_name, collection.id)
}
