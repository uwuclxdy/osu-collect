#[cxx::bridge(namespace = "osu_realm")]
pub mod ffi {
    #[derive(Debug, Clone)]
    pub struct LocalBeatmap {
        pub id: u32,
        pub checksum: String,
        pub beatmapset_id: u32,
    }

    #[derive(Debug, Clone)]
    pub struct LocalBeatmapset {
        pub id: u32,
        pub beatmaps: Vec<LocalBeatmap>,
        pub folder_name: String,
    }

    #[derive(Debug, Clone)]
    pub struct LocalCollection {
        pub name: String,
        pub beatmap_checksums: Vec<String>,
    }

    unsafe extern "C++" {
        include!("realm_wrapper.hpp");

        type RealmDB;

        fn open_realm(path: &str) -> Result<UniquePtr<RealmDB>>;

        fn list_beatmapsets(self: &RealmDB) -> Vec<LocalBeatmapset>;
        fn list_collections(self: &RealmDB) -> Vec<LocalCollection>;
    }
}
