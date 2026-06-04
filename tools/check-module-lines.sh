#!/usr/bin/env bash
set -euo pipefail

warn_limit="${WANIX_MODULE_LINE_WARN:-250}"
hard_limit="${WANIX_MODULE_LINE_LIMIT:-350}"
baseline_file="${WANIX_MODULE_LINE_BASELINE:-tools/module-line-baseline.txt}"

if [[ ! -f "$baseline_file" ]]; then
  echo "missing module line baseline: $baseline_file" >&2
  exit 1
fi

tmp_counts="$(mktemp)"
tmp_errors="$(mktemp)"
tmp_warnings="$(mktemp)"
cleanup() {
  rm -f "$tmp_counts" "$tmp_errors" "$tmp_warnings"
}
trap cleanup EXIT

is_test_file() {
  case "$1" in
    */tests.rs|*/tests/*|*_tests.rs|*_tests/*) return 0 ;;
    *) return 1 ;;
  esac
}

count_non_test_lines() {
  awk '
    function brace_delta(text, copy, opens, closes) {
      copy = text
      opens = gsub(/\{/, "{", copy)
      copy = text
      closes = gsub(/\}/, "}", copy)
      return opens - closes
    }

    BEGIN {
      count = 0
      pending_test_cfg = 0
      skipping_test_block = 0
      test_depth = 0
    }

    {
      line = $0

      if (skipping_test_block) {
        test_depth += brace_delta(line)
        if (test_depth <= 0) {
          skipping_test_block = 0
        }
        next
      }

      if (pending_test_cfg) {
        if (index(line, "{") > 0) {
          test_depth = brace_delta(line)
          if (test_depth > 0) {
            skipping_test_block = 1
          }
          pending_test_cfg = 0
          next
        }
        if (index(line, ";") > 0) {
          pending_test_cfg = 0
        }
        next
      }

      if (line ~ /^[[:space:]]*#\[cfg\(test\)\]/) {
        pending_test_cfg = 1
        next
      }
      if (line ~ /^[[:space:]]*$/) {
        next
      }
      if (line ~ /^[[:space:]]*\/\//) {
        next
      }

      count++
    }

    END {
      print count
    }
  ' "$1"
}

find crates -path '*/src/*.rs' -o -path '*/src/**/*.rs' | while IFS= read -r file; do
  if is_test_file "$file"; then
    continue
  fi
  count="$(count_non_test_lines "$file")"
  printf "%s %s\n" "$count" "$file"
done | sort -nr > "$tmp_counts"

while read -r allowed path; do
  case "${allowed:-}" in
    ""|\#*) continue ;;
  esac
  if [[ ! -f "$path" ]]; then
    printf "stale baseline entry: %s\n" "$path" >> "$tmp_warnings"
  fi
done < "$baseline_file"

while read -r count path; do
  allowed="$(awk -v target="$path" '$1 !~ /^#/ && $2 == target { print $1; exit }' "$baseline_file")"
  if (( count > hard_limit )); then
    if [[ -z "$allowed" ]]; then
      printf "%s has %s non-test lines, over hard limit %s\n" "$path" "$count" "$hard_limit" >> "$tmp_errors"
      continue
    fi
    if (( count > allowed )); then
      printf "%s has %s non-test lines, above baseline %s\n" "$path" "$count" "$allowed" >> "$tmp_errors"
    fi
  elif (( count > warn_limit )); then
    printf "%s has %s non-test lines, above preferred limit %s\n" "$path" "$count" "$warn_limit" >> "$tmp_warnings"
  elif [[ -n "$allowed" ]]; then
    printf "%s is now under hard limit; remove baseline entry\n" "$path" >> "$tmp_warnings"
  fi
done < "$tmp_counts"

if [[ -s "$tmp_warnings" ]]; then
  sed 's/^/module-lines warning: /' "$tmp_warnings"
fi

if [[ -s "$tmp_errors" ]]; then
  sed 's/^/module-lines error: /' "$tmp_errors" >&2
  exit 1
fi

echo "module-lines ok: production Rust modules fit current baseline and hard limit ${hard_limit}"
