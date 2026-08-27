#!/usr/bin/env bash
# PreToolUse gate for `gh pr create`: runs with the user's shell permissions,
# so it can read the diff itself, then asks a headless Claude for a fast
# last-line verdict. Fails closed only on concrete findings or on being
# unable to run the check at all.
set -euo pipefail

MAX_DIFF_BYTES=200000
MODEL="${PR_GATE_MODEL:-claude-sonnet-5}"

input=$(cat)
# The settings `if` matcher errs on the side of matching for commands it
# can't parse (heredocs, brace groups); only gate a real `gh pr create`.
cmd=$(printf '%s' "$input" | jq -r '.tool_input.command // empty')
case "$cmd" in *"gh pr create"*) ;; *) exit 0 ;; esac
cwd=$(printf '%s' "$input" | jq -r '.cwd // empty')
[ -n "$cwd" ] && cd "$cwd"

deny() {
  jq -cn --arg r "$1" '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

base="${PR_GATE_BASE:-}"
if [ -z "$base" ]; then
  if git rev-parse --verify -q origin/main >/dev/null; then base=origin/main; else base=main; fi
fi

stat=$(git diff "$base"...HEAD --stat) || deny "pr-gate: git diff $base...HEAD failed"
[ -n "$stat" ] || deny "pr-gate: no commits ahead of $base — nothing to open a PR for"

stray=$(git diff "$base"...HEAD --name-only | grep -E '(^|/)(target/|\.DS_Store$|.*\.(orig|rej|swp)$)' || true)
[ -z "$stray" ] && stray=""
if [ -n "$stray" ]; then deny "pr-gate: stray/generated files in the diff:"$'\n'"$stray"; fi

diff=$(git diff "$base"...HEAD | head -c "$MAX_DIFF_BYTES")
truncated=""
[ "$(git diff "$base"...HEAD | wc -c)" -gt "$MAX_DIFF_BYTES" ] && truncated=" (diff truncated to ${MAX_DIFF_BYTES} bytes)"

prompt="You are the final gate before PR creation in the cocovm Rust workspace. Pre-PR reviews (idiomatic-rust + pr-challenger) have already run on this branch, so this is a fast last-line check, NOT a re-review. Look ONLY for: obviously broken logic or invariants in changed code; brand-new behavior with no test at all. Do NOT flag style, book/ being out of date, or anything the compiler/clippy/tests already guarantee; never request tests that assert a removed feature stays absent. PASS is a first-class outcome — do not manufacture findings.

Reply with exactly one of these on the FIRST line: PASS or FAIL. After FAIL, list each concrete must-fix problem as file:line plus one sentence.

--- git diff --stat${truncated} ---
$stat

--- git diff ---
$diff"

verdict=$(printf '%s' "$prompt" | claude -p --model "$MODEL" --tools "" 2>&1) || deny "pr-gate: headless review failed to run:"$'\n'"$(printf '%s' "$verdict" | tail -5)"
first=$(printf '%s' "$verdict" | grep -m1 -E '^(PASS|FAIL)' || true)
case "$first" in
  PASS*) exit 0 ;;
  FAIL*) deny "pr-gate FAIL:"$'\n'"$verdict" ;;
  *) deny "pr-gate: no PASS/FAIL verdict from the review:"$'\n'"$(printf '%s' "$verdict" | head -20)" ;;
esac
