#!/usr/bin/env bash
# Throughput: run once in a plain Ghostty window and once inside shroud, then compare.
# Usage: scripts/bench.sh [size-in-MB]   (default 200)
set -euo pipefail
mb="${1:-200}"
file="${TMPDIR:-/tmp}/shroud-bench-${mb}mb.txt"
if [[ ! -f "$file" ]]; then
  # Mixed ASCII, colour and wide characters, closer to real output than /dev/urandom.
  line=$'plain text 0123456789 \e[31mred\e[0m \e[1;38;2;10;200;90mtruecolor\e[0m 世界 emoji ✨ tabs\tend'
  yes "$line" | head -c "$((mb * 1024 * 1024))" > "$file"
fi
where="plain terminal"
[[ -n "${SHROUD:-}" ]] && where="inside shroud"
echo "cat ${mb} MB ($where):"
time cat "$file"
