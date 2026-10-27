#!/usr/bin/env bash
# Build each commit, run the pipeline benchmark (`rapidraw bench`) with each build, and compare
# the results in commit order with bench/compare-bench.mjs. Use it to see how each commit, for
# example each merge of a feature branch, changed the speed and the output of the pipeline.
#
# Usage: bench/bench-commits.sh <commit>... -- <image>... [bench options]
#   For example, every merge on the first-parent history of a branch, starting from its base:
#   bench/bench-commits.sh $(git rev-list --first-parent --reverse base^..main) -- photos/*.ARW
#
# Builds go to $BENCH_WORK (default bench/out/commits): a worktree, a shared cargo target dir,
# one copy of the binary per commit and the JSON results. All builds are made before any
# benchmark runs, so compilation does not heat up the CPU during a measurement.
set -euo pipefail

commits=()
while [ $# -gt 0 ] && [ "$1" != "--" ]; do
  commits+=("$1")
  shift
done
if [ $# -eq 0 ] || [ ${#commits[@]} -eq 0 ]; then
  echo "Usage: $0 <commit>... -- <image>... [bench options]" >&2
  exit 2
fi
shift
bench_args=("$@")

repo=$(git rev-parse --show-toplevel)
work=$(realpath -m "${BENCH_WORK:-$repo/bench/out/commits}")
mkdir -p "$work/bin" "$work/results"
tree="$work/src"
if [ ! -e "$tree/.git" ]; then
  git -C "$repo" worktree add --detach "$tree" "${commits[0]}" > /dev/null
fi

labels=()
for commit in "${commits[@]}"; do
  sha=$(git -C "$repo" rev-parse --short "$commit^{commit}")
  labels+=("$sha")
  bin="$work/bin/$sha"
  if [ -x "$bin/RapidRAW" ]; then
    echo "== $sha already built"
    continue
  fi
  echo "== Building $sha: $(git -C "$repo" log -1 --format=%s "$sha")"
  git -C "$tree" checkout --quiet --detach "$sha"
  # Headless runs don't load the frontend, so it is not built (no tauri/custom-protocol).
  CARGO_TARGET_DIR="$work/target" cargo build --release --quiet --manifest-path "$tree/src-tauri/Cargo.toml"
  # Without custom-protocol, Tauri looks for resources next to the executable.
  rm -rf "$bin.tmp"
  mkdir -p "$bin.tmp"
  cp "$work/target/release/RapidRAW" "$bin.tmp/"
  for resource in resources lensfun_db; do
    if [ -e "$work/target/release/$resource" ]; then cp -rL "$work/target/release/$resource" "$bin.tmp/"; fi
  done
  mv "$bin.tmp" "$bin"
done

results=()
for i in "${!labels[@]}"; do
  sha=${labels[$i]}
  echo "== Benchmarking $sha ($((i + 1))/${#labels[@]})"
  "$work/bin/$sha/RapidRAW" bench "${bench_args[@]}" --json "$work/results/$sha.json" > "$work/results/$sha.txt" 2>&1 || {
    echo "Benchmark of $sha failed, see $work/results/$sha.txt" >&2
    exit 1
  }
  results+=("$sha=$work/results/$sha.json")
done

echo
for sha in "${labels[@]}"; do
  echo "$sha  $(git -C "$repo" log -1 --format=%s "$sha")"
done
echo
node "$repo/bench/compare-bench.mjs" "${results[@]}"
