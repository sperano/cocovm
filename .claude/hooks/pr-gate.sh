#!/usr/bin/env bash
# PreToolUse gate for `gh pr create`: runs with the user's shell permissions,
# so it can read the diff itself, then runs a headless Claude review for a
# final verdict. Fails closed only on concrete findings or on being
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

# The hook's cwd is the session's, not the command's: honor a leading `cd <dir> &&`.
if [[ "$cmd" =~ ^[[:space:]]*cd[[:space:]]+[\"\']?([^[:space:]\"\'\&\;]+)[\"\']?[[:space:]]*\&\& ]]; then
  dir="${BASH_REMATCH[1]}"; dir="${dir/#\~/$HOME}"
  cd "$dir" || deny "pr-gate: cannot cd to $dir"
fi

# Diff the --head branch when one is given.
head_ref=HEAD
if [[ "$cmd" =~ --head[[:space:]=]+[\"\']?([^[:space:]\"\']+) ]]; then head_ref="${BASH_REMATCH[1]##*:}"; fi

base="${PR_GATE_BASE:-}"
if [ -z "$base" ]; then
  if git rev-parse --verify -q origin/main >/dev/null; then base=origin/main; else base=main; fi
fi

stat=$(git diff "$base...$head_ref" --stat) || deny "pr-gate: git diff $base...$head_ref failed"
[ -n "$stat" ] || deny "pr-gate: no commits ahead of $base — nothing to open a PR for"

stray=$(git diff "$base...$head_ref" --name-only | grep -E '(^|/)(target/|\.DS_Store$|.*\.(orig|rej|swp)$)' || true)
[ -z "$stray" ] && stray=""
if [ -n "$stray" ]; then deny "pr-gate: stray/generated files in the diff:"$'\n'"$stray"; fi

diff=$(git diff "$base...$head_ref" | head -c "$MAX_DIFF_BYTES")
truncated=""
[ "$(git diff "$base...$head_ref" | wc -c)" -gt "$MAX_DIFF_BYTES" ] && truncated=" (diff truncated to ${MAX_DIFF_BYTES} bytes)"

prompt="You are the final gate before PR creation in the cocovm Rust workspace. Pre-PR reviews (idiomatic-rust + pr-challenger) have already run on this branch, so this is a fast last-line check, NOT a re-review. Look ONLY for: obviously broken logic or invariants in changed code; brand-new behavior with no test at all. Do NOT flag style or anything the compiler/clippy/tests already guarantee; never request tests that assert a removed feature stays absent. PASS is a first-class outcome — do not manufacture findings.

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
