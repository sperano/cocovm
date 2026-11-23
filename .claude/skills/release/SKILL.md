---
name: release
description: Cut a release of cocovm — collect unreleased changes, write the release notes, open the release PR, then draft the GitHub release once it is merged; the owner approves by pushing the tag. Use when the user says "cut a release", "release 0.6", "tag a new version", or asks what's unreleased.
---

# Release cocovm

Single source of truth for the version: `[workspace.package] version` in the
root `Cargo.toml`. All crates inherit it (`version.workspace = true`) and are
released in lockstep. Feature PRs never bump the version or edit the
changelog — only this release flow does.

Releases go through hollingsworth's shared `/release` script
(`release_cli.py`, catalogue entry `cocovm`, kind `binaries`), in three
steps: a release PR, a draft GitHub release, then the owner's approval.
Nothing is ever pushed straight to `main`, and an agent never pushes a `v*`
tag or publishes a release.

How to run the script:

- In an agent pod: `python3 /opt/kelos-tools/release_cli.py cocovm X.Y.Z ...`
  from the repository root.
- On the laptop, from the hollingsworth checkout:
  `just release cocovm X.Y.Z --cwd ~/code/cocovm ...` (paths absolute).

## Arguments

- No args → **preview mode**: report what's unreleased and recommend a bump
  level, then stop. Do not modify anything.
- `patch` | `minor` | `major` → bump that component of the current version.
- An explicit version like `0.6.0` → use it verbatim. It must be greater
  than the current version and every `v*` tag and published release; the
  script refuses otherwise.

## Preview mode (also the first step of a real release)

1. Refresh the remote state first: `git fetch --quiet --tags origin`.
   Without it, `origin/main` and the tags can be stale.
2. Find the last release: `git tag --sort=-v:refname | head -1`.
3. List what's unreleased:
   - `git log <last-tag>..origin/main --oneline --first-parent`
   - For merged PRs, get real titles/bodies: `gh pr list --state merged
     --limit 30 --json number,title,mergedAt` and keep those merged after the
     last tag's date.
4. Summarize the changes grouped as Added / Changed / Fixed and recommend a
   bump: new features → `minor`; only fixes → `patch`. (Pre-1.0, breaking
   changes also go under `minor` — flag them in the entry instead.)
5. In preview mode, print this and stop.

## Release flow

Git commands sequentially, never in parallel.

### 1. Write the release notes

Write this version's changelog section body (no heading line) to a scratch
file outside the repository, in [Keep a Changelog](https://keepachangelog.com)
form:

```markdown
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
  Show the draft to the user, but don't wait for a reply: wording tweaks
  can be pushed to the release PR before it merges.

### 2. Open the release PR

```
release_cli.py cocovm X.Y.Z --notes-file <notes>
```

`origin/main` does not carry `X.Y.Z` yet, so this is **prepare mode**: on a
`release/cocovm/X.Y.Z` branch the script bumps
`[workspace.package] version`, every workspace crate in `Cargo.lock`, and
inserts `## [X.Y.Z] - YYYY-MM-DD` with the notes below an empty
`## [Unreleased]`, then opens (or reuses) the PR "Release cocovm X.Y.Z".
Report its link. It merges through `cubic-gate` like any PR, and merging it
publishes nothing. Rerunning the command rebuilds the branch from
`origin/main` and updates the same PR.

If `## [Unreleased]` already has entries, run without `--notes-file`: they
become the release's notes. Both, or neither, is refused.

### 3. Draft the GitHub release (after the PR is merged)

```
release_cli.py cocovm X.Y.Z
```

`origin/main` now carries `X.Y.Z`, so this is **draft mode**. The script
checks that `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` at that exact
commit say `X.Y.Z`, that no `vX.Y.Z` tag exists elsewhere, then runs
`cargo test --workspace --locked` at the commit (blocking: this repo has no
test CI; it can take a while, so give the call the longest timeout). Only
then does it create or update the one draft release for `vX.Y.Z`, pinned
to the full commit SHA, with the changelog section as its notes. Report the
draft's link. Rerunning reuses the same draft.

**Never publish the draft.** The repository uses immutable releases: a
published release accepts no further assets, and GitHub never lets a tag
name an immutable release used be published again. Publishing early burns
the version with no binaries.

### 4. Approval (the owner, on the laptop)

Tell the user the release is ready for approval:

```
just release-approve cocovm X.Y.Z --cwd ~/code/cocovm
```

It re-runs the validation (tests included) at the draft's SHA, refuses if
the draft no longer matches that commit's version or changelog, then pushes
the annotated tag `vX.Y.Z` there. The tag push runs `Release binaries`
(`.github/workflows/release.yml`): it builds the five targets, attaches
each archive to the draft, and its `publish` job publishes it. A tag
ruleset on `refs/tags/v*` lets only the owner push release tags.

### 5. Report

End by stating: the version, the PR or draft link (whichever step you
reached), and what the user does next.

## Failure handling

- The script exits 1 with the reason for a blocking failure (version not
  above the latest tag, a tag already elsewhere, failing tests, a changelog
  or lockfile that does not say the version): fix it on `main` through a
  PR, then rerun the step.
- Draft mode after a later commit on `main` changed the notes: rerun step 3;
  the draft follows the new SHA and notes while it is still a draft.
- A build target failed after approval: the draft stays unpublished. The
  owner fixes the cause and re-runs the workflow against the existing tag:
  `gh workflow run release.yml -f tag=vX.Y.Z` (agents have no Actions
  write). It reuses the draft and publishes once all targets succeed.
- A published release cannot be repaired, and its version is burned:
  release the next patch instead.
