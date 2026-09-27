#!/usr/bin/env bash
# Callgrind instruction counts per small bin: baseline against current.
# Usage: performance/instructions.sh <output-directory> [bins...]
set -euo pipefail
cd "$(dirname "$0")"
output="$1"
shift
bins=("${@:-tiny small}")
mkdir -p "$output/callgrind"
printf "bin\tconfiguration\tbaseline\tcurrent\tchange_percent\n" > "$output/instructions.tsv"
for bin in ${bins[@]}; do
  for configuration in commonmark gfm mdx gfm-mdx everything reduced mdx-aware; do
    declare -A total
    for implementation in baseline current; do
      file="$output/callgrind/$bin-$configuration-$implementation.out"
      valgrind --tool=callgrind --toggle-collect='*parse_all*' --callgrind-out-file="$file" \
        ./target/release/instructions "$bin" "$configuration" "$implementation" 2>/dev/null
      total[$implementation]=$(grep -E "^(summary|totals):" "$file" | head -1 | awk '{print $2}')
    done
    awk -v b="$bin" -v c="$configuration" -v x="${total[baseline]}" -v y="${total[current]}" \
      'BEGIN { printf "%s\t%s\t%d\t%d\t%+.2f\n", b, c, x, y, (y - x) * 100 / x }' >> "$output/instructions.tsv"
  done
done
column -t "$output/instructions.tsv"
