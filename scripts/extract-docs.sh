#!/usr/bin/env bash
# Pre-extract docs/*.pdf to docs/txt/*.txt (pdftotext -layout) for grepping.
# Idempotent: skips a PDF whose .txt is already newer. Run from the main
# checkout — the PDFs are git-ignored and absent from worktrees.
set -euo pipefail
shopt -s nullglob

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
docs_dir="$script_dir/../docs"
out_dir="$docs_dir/txt"

command -v pdftotext >/dev/null || { echo "pdftotext not found (brew install poppler)" >&2; exit 1; }

pdfs=("$docs_dir"/*.pdf)
if [ "${#pdfs[@]}" -eq 0 ]; then
  echo "no PDFs in $docs_dir (worktree? run from the main checkout)" >&2
  exit 1
fi

mkdir -p "$out_dir"

for pdf in "${pdfs[@]}"; do
  base="$(basename "$pdf" .pdf)"
  txt="$out_dir/$base.txt"
  if [ -e "$txt" ] && [ "$txt" -nt "$pdf" ]; then
    echo "skipped: $base.txt (up to date)"
    continue
  fi
  # Write to a temp and rename so an interrupted run never leaves a truncated .txt.
  pdftotext -layout "$pdf" "$txt.part"
  mv "$txt.part" "$txt"
  echo "extracted: $base.txt"
done
