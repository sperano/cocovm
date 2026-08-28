#!/usr/bin/env bash
# Pre-extract docs/*.pdf to docs/txt/*.txt (pdftotext -layout) for grepping.
# Idempotent: skips a PDF whose .txt is already newer.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
docs_dir="$script_dir/../docs"
out_dir="$docs_dir/txt"

mkdir -p "$out_dir"

for pdf in "$docs_dir"/*.pdf; do
  [ -e "$pdf" ] || continue
  base="$(basename "$pdf" .pdf)"
  txt="$out_dir/$base.txt"
  if [ -e "$txt" ] && [ "$txt" -nt "$pdf" ]; then
    echo "skipped: $base.txt (up to date)"
    continue
  fi
  pdftotext -layout "$pdf" "$txt"
  echo "extracted: $base.txt"
done
