#!/usr/bin/env bash
# Detect Max Payne 1 localized language RAS packs.
# Convention (retail + darkje language guide): x_<language>.ras
#   english, french, german, italian, spanish, polish, portuguese, russian, …
# Non-language archives to ignore: x_data.ras, x_music.ras, x_level*.ras
#
# Usage:
#   source detect-localized-lang.sh
#   lang="$(detect_localized_lang /path/to/MaxPayne1)" || true
# Prints language stem to stdout (e.g. "italian"); exit 1 if English-only / none.

detect_localized_lang() {
  local game="${1:-}"
  [[ -n "$game" && -d "$game" ]] || return 1
  local f base
  # shellcheck disable=SC2086,SC2012
  for f in "$game"/x_*.ras "$game"/X_*.ras; do
    [[ -e "$f" ]] || continue
    [[ -f "$f" ]] || continue
    base="$(basename "$f")"
    base="${base#[Xx]_}"
    base="${base%.[Rr][Aa][Ss]}"
    base="$(printf '%s' "$base" | tr '[:upper:]' '[:lower:]')"
    case "$base" in
      english | data | music | level | level[0-9]*) continue ;;
      *)
        printf '%s\n' "$base"
        return 0
        ;;
    esac
  done
  return 1
}
