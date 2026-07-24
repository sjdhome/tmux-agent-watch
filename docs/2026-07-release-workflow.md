# Tag-based release workflow

## Why

The project had no CI/CD. The user asked for a GitHub Actions workflow that
publishes a release when a version tag is pushed, with a guard that the tag
version must match the `version` field in `Cargo.toml` so tags and crate
metadata can never drift apart.

## When

2026-07-24.

## How

`.github/workflows/release.yml` triggers on tags matching `v*` and runs four
jobs:

1. **verify-version** — extracts the crate version via
   `cargo metadata --no-deps | jq '.packages[0].version'` and compares it with
   the tag name stripped of its `v` prefix. A mismatch fails the run with an
   error telling the operator to bump `Cargo.toml` or re-tag.
2. **check** — runs `make check` (rustfmt check + clippy with `-D warnings` +
   tests), reusing the project's own validation target.
3. **build** — matrix build gated on the two jobs above:
   `x86_64-unknown-linux-musl` (ubuntu-latest),
   `aarch64-unknown-linux-musl` (ubuntu-24.04-arm),
   `x86_64-apple-darwin` and `aarch64-apple-darwin` (macos-latest, the x86_64
   one cross-compiled). Linux builds use musl for static, glibc-independent
   binaries. Each job packages `tmux-agent-watch-vX.Y.Z-<target>.tar.gz` and
   uploads it as an artifact.
4. **release** — downloads all archives, generates `SHA256SUMS`, and publishes
   a GitHub release with `gh release create --generate-notes` using the
   built-in `GITHUB_TOKEN` (workflow has `contents: write`).

Releasing a new version is therefore:

```sh
# bump version in Cargo.toml, commit, then:
git tag v0.2.0
git push origin v0.2.0
```

## Open issues

- The workflow has not run yet; it can only be validated end to end after the
  first tag is pushed to GitHub. Local checks covered YAML syntax and the
  version-extraction command only.
- No Windows target is built (tmux does not run natively on Windows), and no
  crates.io publish step exists. Add `cargo publish` to the release job if the
  crate should ever be published to the registry.
- `ubuntu-24.04-arm` runners are free only for public repositories. If the
  repository is ever made private, that matrix entry needs a different runner
  or cross-compilation via `cross`.
