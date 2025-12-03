1. create a plan to refactor the entire codebase in a way that separates concerns better and makes it easier to maintain and add features. this includes breaking down large functions into smaller ones, organizing code into modules based on functionality, and streamlining or simplifying logic and process flows where possible without changing behavior.

2. analyze the entire codebase, read all source files, and point out potential improvements, optimizations, and points for simplification. write it all down in a Markdown file for me to audit. especially focus on the flows where the program is most likely doing more work than necessary.

---

read current README then analyze the codebase and update the README to reflect the current state of the codebase, features, and usage instructions. ensure that any discrepancies between the documentation and the actual implementation are resolved. maintain the current style and formatting of the README while making these updates. keep it concise and use the same tone.

---

make BeatmapStatus reflect a state different from "Downloading" when all threads are on the same status different from "Downloading". for that, we need to implement a new thread status for rechecking maps as well. it should look like this: "Thread {number}: Rechecking #{mapset id}"

move constants from all files in the codebase into `config/constants.rs`.

---

clear distinction between old maps that cant get verified, prompt user to redownload?


add one character of inner padding to the console window on left and right side. then check and modify all strings that show in the console and have spaces in front of them.

add an option in the home tab to skip checking beatmap integrity after download. it should be configurable in settings and saved in permanent config.

official api support (api key n all that shi)

update the download pages:
1. don't show "Rate limited on _, switching to _" and instead show "Downloading from _" like it normally does.
2. hide threads that have completed their work in the tui view instead of showing them as "Done".
3. show a warning above the Overview tab (same style and position as the "Configure download directory and mirrors in the Home tab before downloading!" message in the updater tab) if not enough space in target directory before starting download.

updater should generate a collection.db and the empty file—like it normally does for collection downloads. However with an exception being that there should be all collections that were updated in one collection.db file (and not just one collection per db like there usually is when downloading a single collection). For updates there is also one more exception: collection name in `collection.db` should be the name of the collection that was updated, as it was in the local db before the update (for example, locally saved collection `> abc-1234` is named `abc-1234` on osu collector - collection.db should in this case contain `> abc-1234`).

updater should also recheck the database before refetching missing beatmaps in all cases.

combine the updater and downloder pipelines as much as possible.

**bugs:**

- dl speed is calculated wrong - multithreading issue
- updater: fetching collections and checking for updates are very slow. is there a way to speed it up?
