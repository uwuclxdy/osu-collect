1. create a plan to refactor the entire codebase in a way that separates concerns better and makes it easier to maintain and add features. this includes breaking down large functions into smaller ones, organizing code into modules based on functionality, and streamlining or simplifying logic and process flows where possible without changing behavior.

2. analyze the entire codebase, read all source files, and point out potential improvements, optimizations, and points for simplification. write it all down in a Markdown file for me to audit. especially focus on the flows where the program is most likely doing more work than necessary.

---

read current README then analyze the codebase and update the README to reflect the current state of the codebase, features, and usage instructions. ensure that any discrepancies between the documentation and the actual implementation are resolved. maintain the current style and formatting of the README while making these updates. keep it concise and use the same tone.

---

make BeatmapStatus reflect a state different from "Downloading" when all threads are on the same status different from "Downloading". for that, we need to implement a new thread status for rechecking maps as well. it should look like this: "Thread {number}: Rechecking #{mapset id}"

move constants from all files into one central file named `constants.rs` in the root directory—next to main.rs.

---

clear distinction between old maps that cant get verified, prompt user to redownload?

show warning if not enough space in target directory before starting download
