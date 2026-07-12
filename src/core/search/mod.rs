pub mod api_client;

use std::sync::LazyLock;

pub use api_client::{HttpSearchService, SearchService};

pub use osu_downloader::search::{
    BeatmapRow, BeatmapSetMeta, MAX_BATCH_IDS, SearchClient, SearchMode, SearchQuery,
    SearchResults, SearchStatus, SortField, SortOrder,
};

/// The one session-wide search service. The search task and the enrichment pager
/// both resolve tokens through it, so the cached guest `client_credentials` token
/// is minted once and shared instead of one cache per caller.
pub fn shared_service() -> &'static HttpSearchService {
    static SERVICE: LazyLock<HttpSearchService> =
        LazyLock::new(|| HttpSearchService::new(SearchClient::new()));
    &SERVICE
}
