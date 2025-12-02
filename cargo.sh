TIME=$(date +%s)
START_TIME=$TIME

cargo fmt --all
NOW=$(date +%s)
echo "✅ fmt ($(( NOW - TIME ))s)"
TIME=$NOW

cargo fix --allow-dirty --release --quiet
NOW=$(date +%s)
echo "✅ fix ($(( NOW - TIME ))s)"
TIME=$NOW

cargo clippy --fix --allow-dirty --quiet --release
NOW=$(date +%s)
echo "✅ clippy fix ($(( NOW - TIME ))s)"
TIME=$NOW

cargo clippy --all-targets --all-features --release
NOW=$(date +%s)
echo "✅ clippy ($(( NOW - TIME ))s)"

echo "=== completed in $(( $(date +%s) - START_TIME ))s ==="
