# Original User Request

## Initial Request — 2026-07-28T16:01:16Z

Ship the first GitHub Release for the `acc` C compiler project and provide convenient installation methods for end users. The project is a production Rust binary (`acc`) with an existing but never-tested release workflow.

Working directory: /Users/iml1s/Documents/mine/acc
Integrity mode: development

## Context & Current State

- **Repository**: `ImL1s/acc` on GitHub (public, MIT licensed)
- **Cargo.toml version**: `0.1.0`
- **CI status**: Green (13/13 steps pass on `main`, commit `b16d3de`)
- **Existing `release.yml`**: Triggers on `v*` tags, builds 4 platform targets (x86_64-linux, aarch64-linux, x86_64-macos, aarch64-macos), creates GitHub Release via `softprops/action-gh-release@v1`. **Never been run — likely has bugs.**
- **Known issues in `release.yml`**:
  1. AArch64 Linux cross-compilation on `ubuntu-latest` is missing `gcc-aarch64-linux-gnu` / cross-compilation linker setup
  2. `actions/download-artifact@v4` default directory structure differs from v3 — the `files:` glob paths in `softprops/action-gh-release` are likely wrong
  3. No `timeout-minutes` safety (should match the 15-minute guard already in `ci.yml`)
- **README**: Has no "Installation" section — only developer build instructions (`cargo build --release`)
- **No install script exists** — users cannot `curl | sh` to install

## Requirements

### R1. Fix and validate the release workflow

The existing `.github/workflows/release.yml` must produce working GitHub Releases with downloadable pre-built binaries for all 4 platform targets when a `v*` tag is pushed. Fix the known cross-compilation and artifact path issues. Ensure the workflow has a `timeout-minutes` guard.

### R2. Add a one-line installer shell script

Provide a shell script (hosted in the repo or as a GitHub raw URL) that detects the user's OS and architecture, downloads the correct binary from the latest GitHub Release, and installs it to a standard location (e.g., `~/.local/bin` or `/usr/local/bin`). The script should work on macOS (Intel + Apple Silicon) and Linux (x86_64 + AArch64).

### R3. Add an Installation section to README.md

Add a clear "Installation" section to `README.md` (after the badges, before or near "Quick Start") that documents all available installation methods:
- One-line installer (`curl | sh`)
- Download pre-built binary from GitHub Releases
- Build from source (`cargo install --git`)

### R4. Create and push the first release tag `v0.1.0`

After the release workflow is fixed and verified, tag and push `v0.1.0` to trigger the actual release. Verify the GitHub Release is created successfully with all 4 binary artifacts.

## Acceptance Criteria

### Release Workflow
- [ ] `git tag v0.1.0 && git push origin v0.1.0` triggers the release workflow
- [ ] The release workflow completes successfully (no failed jobs)
- [ ] A GitHub Release named `v0.1.0` exists at `https://github.com/ImL1s/acc/releases`
- [ ] The release contains 4 downloadable `.tar.gz` artifacts (x86_64-linux, aarch64-linux, x86_64-macos, aarch64-macos)
- [ ] Each `.tar.gz` contains a working `acc` binary (verify at least one by extracting and running `./acc --help`)

### Install Script
- [ ] A shell script exists in the repository that can be invoked via `curl -fsSL <raw-url> | sh`
- [ ] The script correctly detects OS (`Linux` vs `Darwin`) and architecture (`x86_64` vs `aarch64`/`arm64`)
- [ ] The script downloads the correct binary from the latest GitHub Release and places it in PATH
- [ ] The script prints clear success/failure messages
- [ ] The script is safe: uses `set -euo pipefail`, validates checksums or HTTP status, and does not silently fail

### README
- [ ] README.md contains an "Installation" section with at least 3 methods (curl installer, GitHub Release download, cargo install from git)
- [ ] All URLs and commands in the Installation section are correct and point to actual resources

### Verification Script
- [ ] A verification script `scripts/verify_release.sh` exists that:
  1. Checks the GitHub Release exists via `gh release view v0.1.0`
  2. Downloads and extracts at least one artifact
  3. Runs `./acc --help` on the extracted binary and confirms exit code 0
  4. Verifies the install script is syntactically valid (`bash -n`)
