# osu-collect Refactor Plan

## Objectives
- Preserve current behaviour while making the codebase easier to extend, debug, optimize, reason about, and test.
- Separate UI, orchestration, domain logic, and infrastructure concerns instead of concentrating them in multi-purpose files such as `src/main.rs`, `src/app.rs`, `src/download_task.rs`, and `src/ui.rs`.
- Break down large functions (`run_tui`, `download_pass`, `render_home_form`, etc.) into smaller helpers with single responsibilities.
- Centralise shared concepts (events, download summaries, configuration) to reduce duplication and implicit coupling.
- Improve process flow readability (input → validation → orchestration → background tasks → rendering) without changing the observable flow.

## Target Architecture Overview
- **Presentation layer** (`tui/`): focuses solely on rendering and input mapping. Depends on immutable view models, not on downloader internals.
- **Application layer** (`app/`): owns state machines (home form, tab navigation, download pages), converts domain events into UI-facing state, and exposes commands for presentation to invoke.
- **Domain / orchestration layer** (`core/`): handles collection fetching, download lifecycle management, integrity checking, and filesystem interactions. Communicates via strongly typed events and commands.
- **Infrastructure layer** (`worker/`): HTTP clients, mirror management, file I/O, config loading, utilities, and error definitions.
- **Shared utilities** (`utils/`): common helpers, parsing, path manipulation, and error types used across layers.
- **Testing utilities** (`tests/`): integration and unit tests for all layers, ensuring behaviour parity and regression safety.
- **Documentation** (`docs/`): design documents, architecture overviews, and usage guides for developers and users.

Keeping this layering flexible will make it easier to add features such as background CLI, richer status views, or alternative frontends.

## Module Breakdown & Refactor Steps

### 1. Configuration & Startup (`src/main.rs`, `src/config.rs`)
- Extract terminal lifecycle helpers into `tui/terminal.rs` (`setup`, `cleanup`, `spawn_input_thread`) to keep `main.rs` focused on high-level flow.
- Move input loop orchestration into an `app/runtime.rs` module that ties event receivers to `App` methods.
- Promote `InputEvent`, `download_finished_id`, and download-abort helpers into a `runtime` module shared with tests.
- Keep `Config` logic in `config/` but add a `ConfigService` responsible for locating files, fallback defaults, and surfacing validation issues so `main.rs` only wires them.

### 2. Application State (`src/app.rs`)
- Split into submodules:
  - `app/home.rs`: Home page state, field navigation, string parsing helpers.
  - `app/collection_download.rs`: Download statistics, log buffers, thread status handling.
  - `app/state.rs`: Overall `App` struct with tab navigation and event dispatch.
  - `app/messages.rs`: Message / notification handling logic.
- Replace hand-written `match` blocks that mutate many booleans with enums + dedicated methods (e.g., `HomePage::toggle_focus_field()`), enabling unit tests per behaviour.
- Introduce a `DownloadCommand` enum returned by `App::handle_key_event` so presentation only forwards key presses and reacts to resulting commands (start download, cancel, toggle field, etc.).

### 3. Download Orchestration (`src/download_task.rs`)
- Turn `download_task.rs` into a module directory (`src/download/`):
  - `mod.rs`: defines public API (`spawn_download`, `DownloadRequest`, `DownloadEvent`).
  - `pipeline.rs`: equivalent of `run_download`, orchestrating stages.
  - `passes.rs`: logic around `download_pass`, queue management, and retries.
  - `integrity.rs`: `ExpectationIndex`, checksum refresh, archive inspection utilities.
  - `precheck.rs`: `verify_existing_beatmapsets` and `PrecheckReport`.
  - `events.rs`: helper constructors / conversions for emitting UI-safe events.
- Break `run_download` into clearer phases: `resolve_collection`, `prepare_output_dir`, `precheck_existing`, `execute_passes`, `finalize`. Each phase returns a struct so failure/abort handling is centralised.
- Replace ad-hoc `HashMap`/`VecDeque` juggling with dedicated types (`DownloadQueue`, `RetryBook`, `FailureRegistry`) to encapsulate invariants and allow targeted tests.
- Move tokio spawn logic for input thread and download handles into a shared `task` module with explicit shutdown semantics.

### 4. Downloader & Mirror Infrastructure (`src/downloader.rs`, `src/mirrors.rs`)
- Separate `MirrorPool` into its own module (`worker/mirror_pool.rs`) and leave schema definitions / validation inside `mirrors/mod.rs`.
- Extract streaming + integrity validation helpers into `worker/io.rs` so they can be reused for tests or future features (e.g., CLI progress bars).
- Introduce a `DownloadContext` struct bundling `client`, `output_dir`, and policy flags so functions like `process_mirror_response` take fewer parameters.
- Provide a trait-based abstraction for filesystem operations (e.g., `trait FileStore`) to enable mocking during unit tests; actual implementation wraps tokio fs calls.

### 5. UI Rendering (`src/ui.rs`)
- Restructure into `ui/home.rs`, `ui/download.rs`, `ui/widgets.rs`, mirroring the layout of `App` submodules.
- Create lightweight view models (plain structs copied from `App` state) so rendering code no longer accesses interior mutability. This will simplify future changes like asynchronous UI refresh or portability to other backends.
- Break monolithic functions like `render_download` into small helpers (`render_overview`, `render_progress_gauge`, `render_failed_map_list`).
- Introduce a `Keymap` widget description module so footer hints are generated from a single source of truth.

### 6. Collection & Collector (`src/collection.rs`, `src/collector.rs`)
- Group under `core/collection/` with `api_client.rs` (HTTP + retry policies), `model.rs` (data types), and `db_writer.rs` (SQLite export). This isolates HTTP concerns from filesystem writing.
- Define interfaces for `CollectionService` so tests can inject fake collections without touching the network.

### 7. Shared Utilities & Errors (`src/utils.rs`, `src/error.rs`)
- Move `sanitize_filename`, `parse_collection_id`, and other helpers into `utils/` submodules (e.g., `utils/path.rs`, `utils/parsing.rs`).
- Expand `AppError` variants or wrap with `thiserror` derive per layer (e.g., `DomainError`, `UiError`) to avoid overloading a single enum.
- Provide conversion impls (`From<DomainError> for DownloadEvent`) where necessary to keep error handling localised.

### Structured Logging Plan (in progress)
- Adopt the `tracing` ecosystem (`tracing`, `tracing-subscriber`, `tracing-appender`) for lightweight, structured spans that can bridge future async work.
- Add a `logging` table in `config.toml` with `enabled`, `level`, `format`, and `file_dir` keys; default to disabled so current behaviour is unchanged.
- When enabled, initialise a layered subscriber: compact JSON to disk (rolling daily files in the configured directory) plus human-readable stderr output gated by the configured level.
- Expose a helper in `utils::logging` (planned) so both CLI and future headless modes register the same subscriber wiring without duplicating setup code.
- Ensure download/runtime modules accept a `LoggerHandle` (or share `tracing` spans) so diagnostics can be emitted without leaking implementation details into the UI layer.

## Process & Sequencing
1. **Scaffolding**: Create the new module directories (`app/`, `download/`, `tui/`, `core/`, `worker/`, `utils/`). Move existing files gradually while keeping `mod` declarations compiling.
2. **Runtime & App split**: Refactor `main.rs` + `app.rs` to use the new runtime module before touching download logic, ensuring TUI boot still works.
3. **Download pipeline extraction**: Move `DownloadEvent`/`DownloadRequest` definitions first, then peel off helper structs (e.g., `ExpectationIndex`) into dedicated files. Add unit tests for integrity checking as soon as they are isolated.
4. **UI modularisation**: After state layer is stable, reorganise `ui.rs` into submodules and adapt `draw` to consume view models from `App`.
5. **Infrastructure cleanup**: Extract mirror pool + downloader utilities, add abstractions for filesystem/network operations, and update `download` modules to use them.
6. **Follow-up polish**: Add targeted tests for each module, tighten visibility (`pub(crate)`), and introduce linting (clippy) gates once structure settles.

At every step, run `cargo fmt`, `cargo clippy`, and the existing integration flow to guard against regressions.

## Clarifications Needed
1. **Module naming preferences**: Do you have a preferred naming scheme for new directories (e.g., `core/` vs `domain/`, `tui/` vs `ui/`)? the ones proposed are okay.
2. **Testing expectations**: Should we add new automated tests (unit/integration) as we refactor, or keep the current test footprint until after behaviour parity is confirmed? we should write tests as the very last step in `tests/` directory.
3. **CLI vs TUI**: Are there plans for a non-TUI interface? yes, we should keep that in mind. in the future i want to support a headless mode. not to be implemented now tho. 
4. **Config compatibility**: Can we reorganise config loading (e.g., TOML schema) if the resulting behaviour is equivalent, or must file layout and defaults remain exactly as today? we can reorganise config loading.
5. **Logging strategy**: Is it acceptable to introduce a structured logging crate (env_logger/tracing) during the refactor, or should we keep the current println!/tx-based messaging only? we can introduce structured logging which can be enabled via config as well as custom log directory. disabled by default.
6. **Platform scope**: Should Windows-specific terminal setup stay inline, or can we hide it behind a platform abstraction module for cleanliness? we can hide it behind a platform abstraction module. however, ensure complete support for windows remains.
