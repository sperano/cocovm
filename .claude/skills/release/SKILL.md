---
name: release
description: Cut a release of cocovm — collect unreleased changes, bump the workspace version, write the CHANGELOG entry, commit, tag, push, and create the GitHub release. Use when the user says "cut a release", "release 0.6", "tag a new version", or asks what's unreleased.
---

# Release cocovm

Single source of truth for the version: `[workspace.package] version` in the
root `Cargo.toml`. All crates inherit it (`version.workspace = true`) and are
released in lockstep. Feature PRs never bump the version or edit the
changelog — only this release flow does.

## Arguments

- No args → **preview mode**: report what's unreleased and recommend a bump
  level, then stop. Do not modify anything.
- `patch` | `minor` | `major` → bump that component of the current version.
- An explicit version like `0.6.0` → use it verbatim (must be greater than
  the current version).

## Preview mode (also the first step of a real release)

1. Find the last release: `git tag --sort=-v:refname | head -1`. If there are
   no tags yet, this is the **first release** — see below.
2. List what's unreleased:
   - `git log <last-tag>..HEAD --oneline --first-parent`
   - For merged PRs, get real titles/bodies: `gh pr list --state merged
     --limit 30 --json number,title,mergedAt` and keep those merged after the
     last tag's date.
3. Summarize the changes grouped as Added / Changed / Fixed and recommend a
   bump: new features → `minor`; only fixes → `patch`. (Pre-1.0, breaking
   changes also go under `minor` — flag them in the entry instead.)
4. In preview mode, print this and stop.

## Release flow

Run these steps in order. Git commands sequentially, never in parallel.

### 1. Preflight

- `git status` must be clean (no staged or unstaged changes — untracked
  git-ignored files are fine). If dirty, stop and tell the user what's there;
  never stash or discard for them.
- Must be on `main`, and `git fetch origin && git status` must show it up to
  date with `origin/main`. Releases are cut from main only, never from a
  worktree feature branch.
- The proposed version must be strictly greater than both the current
  `Cargo.toml` version and the latest tag.
- `Cargo.toml`'s version must equal the latest tag (minus the `v`), and the
  newest `## [X.Y.Z]` section in `CHANGELOG.md` must be that same version.
  If either is newer than the tag, a previous release was started but never
  committed and tagged (its bump got swept into an unrelated commit). Stop
  and report it; the user decides whether to fold those entries into this
  release's notes. Never retro-tag such commits.

### 2. Bump the version

- Edit `version` under `[workspace.package]` in the root `Cargo.toml` — the
  only place the version lives. Do not add `version =` lines to member crates.
- Run `cargo check --workspace` so `Cargo.lock` picks up the new crate
  versions. The lockfile change is part of the release commit.

### 3. Changelog

`CHANGELOG.md` at the repo root, [Keep a Changelog](https://keepachangelog.com)
format. Prepend a new section:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Added
- ...

### Changed
- ...

### Fixed
- ...
```

- Write entries for **users of the emulator**, not for developers: name the
  hardware/feature ("Orchestra-90 CC cartridge with stereo audio"), not the
  refactor. Omit empty subsections. Fold internal-only changes into a short
  line or drop them.
- Source material: the PR titles/bodies and commits gathered in preview.
  Draft the entry and show it to the user. Wording tweaks happen via
  `git commit --amend` after step 5 — do not leave the bump and changelog
  sitting uncommitted while waiting for a reply.
- **First release only** (no CHANGELOG.md yet): create the file with the
  standard Keep-a-Changelog header and backfill one section per era of the
  full git history rather than pretending everything is new. Ask the user
  whether to backfill in detail or start with a single summary section.

### 4. Verify

Run the test suite before committing (use the `quick-check` agent or
`cargo build --workspace && cargo test --workspace`). A release is never cut
on a red tree. **Never run `cargo fmt` in this repo.**

### 5. Commit, tag, push

```
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release: vX.Y.Z"
git tag -a vX.Y.Z -m "vX.Y.Z"
git push --follow-tags
```

- Annotated tag (`-a`), name prefixed with `v`.
- No co-author trailers on the commit message.
- Commit and tag immediately after verification passes, without waiting for
  the user: an uncommitted bump gets swept into the next unrelated commit
  and the version is silently never tagged. Only the push waits for the
  user's confirmation (the push publishes the tag; everything before it is
  local and reversible — `git tag -d` and `git commit --amend` fix wording).

### 6. GitHub release

```
gh release create vX.Y.Z --title "vX.Y.Z" --notes-file <notes>
```

where `<notes>` is a temp file (scratchpad) containing just this version's
changelog section (without the `## [X.Y.Z]` heading line). If a CI workflow
exists that attaches binaries on `v*` tags, note that it will run; otherwise
the release ships notes-only.

### 7. Report

End by stating: new version, tag name, release URL, and a one-line summary of
what shipped.

## Failure handling

- If anything fails after the tag exists locally but before push: fix, then
  `git tag -d vX.Y.Z` and re-tag on the corrected commit. Never retag a
  version that has already been pushed — bump again instead.
- If the push succeeded but `gh release create` failed, just re-run the
  `gh release create` step; the tag is already up.
