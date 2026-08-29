#!/usr/bin/env bash
# Build EPUB and PDF editions of the book from book/*.md.
#
# Requires: pandoc, tectonic (brew install pandoc tectonic).
# Usage: tools/build-book.sh [output-dir]   (default: build/)
set -euo pipefail
cd "$(dirname "$0")/.."

OUTDIR=${1:-build}
mkdir -p "$OUTDIR"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The syllabus (README) leads, then chapters in order, then appendices.
# STYLE.md is a contributor document and stays out of the built book.
sed '1s/^# .*/# Syllabus/' book/README.md > "$WORK/00-syllabus.md"
ORDERED=("$WORK/00-syllabus.md")
for f in book/ch*.md book/appendices.md; do
  cp "$f" "$WORK/$(basename "$f")"
  ORDERED+=("$WORK/$(basename "$f")")
done

# The syllabus links chapters by filename. In the merged book, those links must
# become internal anchors. Use pandoc to compute each chapter's H1 anchor ID so
# the rewrite remains consistent with pandoc's identifier algorithm.
for f in book/ch*.md book/appendices.md; do
  base=$(basename "$f")
  h1=$(grep -m1 '^# ' "$f")
  id=$(printf '%s\n' "$h1" | pandoc -f gfm-tex_math_dollars -t html5 |
    sed -n 's/.*id="\([^"]*\)".*/\1/p' | head -1)
  sed -i '' -e "s|]($base#|](#|g" -e "s|]($base)|](#$id)|g" \
    "$WORK/00-syllabus.md"
done

# The subtitle is the real emulator version, read from the workspace
# Cargo.toml so it remains consistent with the code.
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

META=(
  --metadata title="Writing a CoCo Emulator"
  --metadata subtitle="Version $VERSION"
  --metadata author="Éric Spérano"
  --metadata lang=en
  --metadata date="$(date +%Y-%m-%d)"
)
# gfm-tex_math_dollars: bare $FF98-style addresses in prose must stay text,
# not open TeX math spans (pandoc's gfm enables $-math by default).
COMMON=(-f gfm-tex_math_dollars --toc --toc-depth=2 --top-level-division=chapter)

pandoc "${COMMON[@]}" "${META[@]}" \
  -o "$OUTDIR/cocovm-book.epub" "${ORDERED[@]}"
echo "built: $OUTDIR/cocovm-book.epub"

# PDF: wrap long code lines instead of letting them overflow the margin.
cat > "$WORK/header.tex" <<'EOF'
\usepackage{fvextra}
\DefineVerbatimEnvironment{Highlighting}{Verbatim}{breaklines,breakanywhere,commandchars=\\\{\}}
EOF

pandoc "${COMMON[@]}" "${META[@]}" \
  --pdf-engine=tectonic \
  -V documentclass=report -V geometry:margin=2.5cm -V fontsize=10pt \
  -V mainfont="Palatino" -V monofont="Menlo" -V monofontoptions="Scale=0.82" \
  -V colorlinks=true -V linkcolor=blue -V urlcolor=blue \
  -H "$WORK/header.tex" \
  -o "$OUTDIR/cocovm-book.pdf" "${ORDERED[@]}"
echo "built: $OUTDIR/cocovm-book.pdf"
