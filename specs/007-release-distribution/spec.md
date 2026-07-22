---
id: "007-release-distribution"
title: "Release distribution: tag-gated prebuilt binaries and installer"
status: approved
created: "2026-07-22"
implementation: complete
depends_on:
  - "002-crate-scaffold"
establishes:
  - ".github/workflows/release.yml"
  - "install.sh"
summary: >
  Closes the gap between "builds from source" and "installable
  product": pushing a v<semver> tag builds a per-triple archive for
  the five supported targets, attaches checksums, a CycloneDX SBOM,
  and a SLSA build-provenance attestation to a GitHub Release, and
  install.sh (curl | sh) consumes the archives. Mirrors the
  spec-spine release pipeline (the family precedent this repo's own
  CI already consumes) plus the fail-fast version guard OPC's
  release workflow learned the hard way. No registry publishing:
  the crate is a binary product, not a library.
---

# 007: Release distribution

## 1. Purpose

Specs 002-006 produce a binary that only a Rust toolchain can obtain.
The CLI's consumers (operators without cargo; agents installing the
MCP server) need prebuilt binaries with integrity and authenticity
evidence. The family already has a proven shape: spec-spine's
tag-gated matrix release and installer, which this repo's
spec-spine.yml consumes on every PR. This spec adopts that shape for
`statecraft`, trimmed to what a single-binary product needs.

## 2. Territory

`.github/workflows/release.yml` (the pipeline) and `install.sh` (the
consumer). `Cargo.toml` stays spec 002's territory: a release is cut
against the committed version, never by editing the manifest from the
workflow.

## 3. Behavior: the pipeline

Trigger: pushing a tag matching `v*`. This is a single-product repo,
so the bare `v<semver>` grammar is enough (OPC needed product-prefixed
tags; we do not).

1. **Version guard (fail-fast, zero build minutes).** A standalone
   first job compares the tag against the committed Cargo.toml
   version (via `cargo metadata`) and dies on mismatch, so a release
   can never ship assets whose `--version` output disagrees with its
   envelope. Lesson imported from OPC spec 193.
2. **Build matrix (five triples).** x86_64/aarch64 linux-gnu,
   x86_64/aarch64 apple-darwin, x86_64 windows-msvc. Four build
   natively on a matching-arch runner; x86_64-apple-darwin
   cross-compiles on the Apple Silicon runner (Xcode ships the x86_64
   SDK; the Intel macos-13 runner is deprecated and queues badly).
   Builds are `--locked`: the committed Cargo.lock is authoritative.
3. **Archives.** `statecraft-<tag>-<triple>.tar.gz` (`.zip` on
   Windows) containing the binary, LICENSE, and README.md, each with
   a `.sha256` sidecar.
4. **Supply-chain evidence.** A per-target CycloneDX SBOM
   (fail-closed if it catalogs zero components) and a SLSA
   build-provenance attestation whose subject is the archive; verify
   with `gh attestation verify <archive> --repo
   statecrafting/statecraft-cli`.
5. **Publish.** One job attaches every archive, sidecar, and SBOM to
   the GitHub Release with generated notes; an unmatched file pattern
   fails the job rather than shipping a partial asset set. Re-running
   a tag updates the same release (idempotent).
6. **Pinning.** Every action is pinned to a full commit SHA with a
   version comment (same rule as ci.yml), so an upstream tag rewrite
   cannot alter a release build.

## 4. Behavior: install.sh

`curl -fsSL .../install.sh | sh`: detect platform and arch, download
the matching archive and its `.sha256` sidecar from GitHub Releases,
verify the checksum (hard requirement), verify the provenance
attestation best-effort via `gh` when available, and install to
`~/.local/bin` (or `/usr/local/bin` when writable and already on
PATH). Environment overrides: `STATECRAFT_VERSION` (release tag,
default latest), `STATECRAFT_BIN_DIR`,
`STATECRAFT_REQUIRE_ATTESTATION=1` (hard-fail without verified
provenance), `STATECRAFT_SKIP_ATTESTATION=1` (checksum still
enforced). musl-based Linux is refused up front with a pointer to
`cargo install --git` (the prebuilt Linux binaries are glibc-only).
Windows users take the `.zip` from the Releases page; the script
targets macOS/Linux.

The installer env prefix overlaps the CLI's own `STATECRAFT_*` config
prefix by design; the CLI reads only `STATECRAFT_BASE_URL` and
`STATECRAFT_OUTPUT`, so no installer variable collides with a config
variable. Any future config key must keep clear of the four installer
names above.

## 5. Release procedure

1. Bump `version` in Cargo.toml (Cargo.lock follows) on a branch;
   merge through the normal gates.
2. Tag the merge commit `v<version>` and push the tag; the pipeline
   does the rest.
3. A failed run is safe to re-run from the tag: every step is
   idempotent against the same release.

## 6. Acceptance

- The first tag produces a GitHub Release carrying five archives,
  five `.sha256` sidecars, five SBOMs, and attestations that
  `gh attestation verify` accepts.
- Live check: install.sh installs that release on a dev machine and
  `statecraft --version` reports the tag's version; the transcript is
  recorded in this spec's status section and flips
  `implementation` to complete.
- ci.yml + spine gates green.

## 7. Out of scope

- Registry publishing (crates.io, npm, PyPI, Homebrew): the crate is
  a binary product, not a library. spec-spine's npm/pypi lanes exist
  for toolchain embedding that this CLI does not need; revisit only
  with a concrete consumer.
- macOS notarization and Windows code signing (unsigned archives plus
  provenance attestations for now; the attestation is the
  authenticity story).
- Self-update (re-run install.sh or use the Releases page; no
  in-binary updater, matching the no-bypass posture of the verbs).

## 8. Status (2026-07-22)

Implemented and live-verified; `implementation: complete`.

The first release, v0.1.0 (signed tag on merge commit 4f0cc73,
workflow run 29964581207), went green on the first run: the version
guard passed, all five matrix legs built, and the publish job attached
all fifteen assets (five archives, five `.sha256` sidecars, five
CycloneDX SBOMs) to a published release with generated notes.

Live check (§6 acceptance), Apple Silicon dev machine, hard mode:
`curl -fsSL .../install.sh | sh` with
`STATECRAFT_REQUIRE_ATTESTATION=1` resolved the latest tag to v0.1.0,
verified the checksum, verified the provenance attestation, installed
the aarch64-apple-darwin binary, and the installed `statecraft
--version` printed `statecraft 0.1.0` (`version --output json`:
`{"name":"statecraft","version":"0.1.0"}`).

One wart found by the live check and fixed with this status edit: the
installer's latest-tag resolution streamed the GitHub API response
into `grep -m1`, which closes the pipe as soon as the tag line is
found and makes curl print a spurious "(56) Failure writing output"
line on the happy path; the response is now buffered before parsing.

Known cosmetic follow-up, deliberately deferred: the pinned
actions/checkout (v4.3.1, shared with ci.yml) emits a Node 20
deprecation notice on current runners. Bump both workflows together
when refreshing action pins; not worth a lone-file drift now.
