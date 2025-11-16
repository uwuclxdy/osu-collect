1. create a plan to refactor the entire codebase in a way that separates concerns better and makes it easier to maintain and add features. this includes breaking down large functions into smaller ones, organizing code into modules based on functionality, and streamlining or simplifying logic and process flows where possible without changing behavior.

2. analyze the entire codebase, read all source files, and point out potential improvements, optimizations, and points for simplification. write it all down in a markdown file for me to audit. especially focus on the flows where the program is most likely doing more work than necessary.




add accumulative download speed (sum from all threads) in the overview block

show how much gb is downloaded out of total (x/yGB)

make BeatmapStatus reflect a state different than "Downloading" when all threads are on the same status different than "Downloading". for that, we need to implement a new thread status for rechecking maps as well. it should look like this: "Thread {number}: Rechecking #{mapset id}"

read current README then analyze the codebase and update the README to reflect the current state of the codebase, features, and usage instructions. ensure that any discrepancies between the documentation and the actual implementation are resolved. maintain the current style and formatting of the README while making these updates. keep it concise and use the same tone.

implement auto update. on startup, check `https://github.com/uwuclxdy/osu-collect/releases/latest` and download the binary for the current platform if the version is newer than the running one. replace the current binary and write a status notification "Application updated to {release name}, please restart".

