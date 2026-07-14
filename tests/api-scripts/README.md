# API test scripts

Shell scripts that exercise every HTTP endpoint the program calls, validating response shapes against the Rust models.

## dependencies

- `bash`
- `curl`
- `jq`
- `xxd`

## run

```bash
# run all tests
bash tests/api-scripts/run_all.sh

# run individual test
bash tests/api-scripts/test_osucollector.sh
```

## scripts

| script | endpoint | rust source | model validated |
|--------|----------|-------------|-----------------|
| `test_osucollector.sh` | `GET https://osucollector.com/api/collections/{id}` | `src/core/collection/api_client.rs` | `Collection { id, name, uploader: { id, username }, beatmapsets: [{ id, beatmaps: [{ id, checksum }] }] }` |
| `test_github_releases.sh` | `GET https://api.github.com/repos/uwuclxdy/osu-collect/releases/latest` | `src/auto_update.rs` | `ReleaseResponse { name, tag_name, assets: [{ name, browser_download_url }] }` |
| `test_nekoha_size.sh` | `GET https://mirror.nekoha.moe/api/beatmapset/{id}` | `osu-downloader/src/size.rs` | `BeatmapsetResponse { file_size: Option<u64> }` (handles string or number) |
| `test_mirrors_download.sh` | all anon mirror download URLs | `osu-downloader/src/mirrors/mod.rs`, `src/config/constants.rs` | ZIP magic bytes (PK\x03\x04) in first 4 bytes |
| `test_osu_official.sh` | `POST https://osu.ppy.sh/oauth/token`, `GET /api/v2/beatmapsets/{id}[/download]` | `src/auth/mod.rs`, `osu-downloader/src/mirrors/mod.rs` (`OsuApi`) | token shape `{ access_token, expires_in, token_type }`; download gated (401/403) for client_credentials |

## mirror endpoints covered

| mirror | download template | check URL (`MIRROR_CHECK_URLS`) |
|--------|-------------------|----------------------------------|
| nerinyan | `https://api.nerinyan.moe/d/{id}` | same |
| osu.direct | `https://osu.direct/d/{id}` | `https://osu.direct/api/d/{id}` |
| sayobot | `https://dl.sayobot.cn/beatmaps/download/full/{id}` | same |
| nekoha | `https://mirror.nekoha.moe/api/download/{id}` | same |
| beatconnect | `https://beatconnect.io/b/{id}/` | same (anon, 301 → CDN) |
| hinamizawa | `https://mirror.hinamizawa.ai/api/v1/hinai/d/{id}` | same (cascade) |
| osu! official | `https://osu.ppy.sh/api/v2/beatmapsets/{id}/download` | n/a — needs a `lazer`-scope user token (see `test_osu_official.sh`) |

## fixture data

- beatmapset: `705655` (THE ORAL CIGARETTES - https://osu.ppy.sh/beatmapsets/705655)
- collection: `50` (used for osucollector test; 705655 is a beatmapset ID, not a collection ID)

## notes

- mirrors that return `429` (rate limited) or `502/503/504` (server error) are skipped, not failed — these are transient upstream conditions
- sayobot intermittently returns 504; this is a known upstream issue
- beatconnect downloads anonymously via `/b/{id}/` (301 → CDN); its JSON API is patreon-gated and not used by the program
- `test_osu_official.sh` SKIPs unless `OSU_CLIENT_ID` / `OSU_CLIENT_SECRET` are exported. It asserts the download endpoint stays gated (401/403) for a `client_credentials` token — the program can only download from osu! official with a user `lazer`-scope token (interactive OAuth login)
