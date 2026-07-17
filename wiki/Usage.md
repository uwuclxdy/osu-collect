# Usage

Run `osu-collect` in a terminal. The app opens on the **get maps** tab with the collection field focused: paste an osu!collector link, pick a folder, activate the download button. Press <kbd>?</kbd> anywhere for the key reference ([Keybindings](Keybindings)).

## Sources on the get maps tab

The source row at the top switches between three ways to get maps. Cycle it with <kbd>space</kbd> / <kbd>↵</kbd>, or jump straight to one with <kbd>1</kbd>-<kbd>3</kbd>.

| Source | What it does |
|---|---|
| `collection` | Downloads a whole osu!collector collection by URL or ID |
| `find` | Searches maps by criteria, no collection needed → [Finding maps](Finding-Maps) |
| `update` | Re-checks downloaded collections and fetches only what's missing → [Updating collections](Updating-Collections) |

Every source shares the same download section (mirrors, directory, threads, overwrite, video); run settings persist across a source switch.

## Form fields

| Field | What it does |
|---|---|
| **Collection URL or ID** | Accepts `https://osucollector.com/collections/{id}` or a bare ID. Resolves as you type and remembers recent collections. *Required for the collection source.* |
| **Download directory** | Defaults to the last used folder. <kbd>tab</kbd> completes filesystem paths while editing. |
| **Threads** | Parallel downloads. Defaults to your CPU core count; 20 or fewer avoids rate limiting. Adjust with <kbd>+</kbd> / <kbd>-</kbd>. |
| **Overwrite existing** | Off (default) verifies and skips maps already on disk; on skips the recheck and redownloads every map fresh. |
| **Video** | Includes beatmap videos (on by default); off downloads video-free where the mirror supports it. |

Custom mirror URLs and the built-in mirror toggles live on the config tab → [Mirrors](Mirrors).

Before a run starts, maps already in your osu! library (stable `osu!.db` or the lazer client database) are skipped instead of re-fetched; they still land in the generated `collection.db`. Turn this off with `download.skip_already_imported` ([Configuration](Configuration#download)).

## Downloads tab

Every run lives on the downloads tab: active runs first, then finished ones, then past runs restored from disk. History survives restarts, cancelled runs included. A new download switches to this tab by default; turn off `display.jump_to_downloads` to stay where you are.

Delete a finished run or a past history entry from the list with <kbd>d</kbd> (an active run can't be deleted; cancel it with <kbd>q</kbd> first). It asks to confirm; tick "don't ask again" with <kbd>space</kbd> in the prompt to skip the confirmation from then on (re-enable it by setting `display.confirm_delete_history = true` in the config file).

Open a run with <kbd>↵</kbd> to see live per-map progress, download speed, ETA, rate-limit countdowns per map, plus a failure summary with reasons.

| Key | Action (inside an open run) |
|---|---|
| <kbd>r</kbd> | Retry all failed maps |
| <kbd>s</kbd> | Defer maps stuck on a rate-limit cooldown so they retry later |
| <kbd>S</kbd> | Drop maps stuck on a rate-limit cooldown for the rest of the run |
| <kbd>q</kbd> | Cancel the run while it's downloading |
| <kbd>esc</kbd> / <kbd>←</kbd> | Step back to the run list without cancelling |

## Failed maps

Failures persist per collection between runs. Three ways to retry them:

- press <kbd>r</kbd> inside the open run,
- accept the prompt on your next download of that collection (`download.retry_failed_on_download` = `ask` / `yes` / `no`),
- let rate-limited maps re-queue themselves: a map that stays throttled past the auto-defer delay (60s of actual waiting by default, `download.rate_limit_skip_secs`) goes back in the queue on its own.
