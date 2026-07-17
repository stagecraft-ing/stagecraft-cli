---
id: "002-crate-scaffold"
title: "The statecraft binary: crate scaffold, config, CI"
status: approved
created: "2026-07-14"
implementation: complete
depends_on:
  - "001-cli-mcp-thesis"
establishes:
  - "Cargo.toml"
  - "Cargo.lock"
  - { kind: directory, path: "src/" }
  - ".github/workflows/ci.yml"
summary: >
  The Rust crate for the single binary named statecraft: clap-based
  command tree, layered configuration (flags > env > config file),
  structured output discipline (human tables on TTY, JSON with
  --output json), and a CI workflow (fmt, clippy -D warnings, test,
  release build). No network calls yet; spec 003 adds auth and the API
  client. After this spec, `cargo install --path .` yields a binary
  whose skeleton every later verb hangs off.
---

# 002: Crate scaffold

## 1. Territory

Root `Cargo.toml` (crate name `statecraft-cli`, binary `[[bin]] name =
"statecraft"`, `[package.metadata.spec-spine] spec = "002-crate-scaffold"`,
license Apache-2.0, edition 2021), `Cargo.lock` (committed; this is a
binary), `src/`, `.github/workflows/ci.yml`. Update `spec-spine.toml`
if the indexer needs the workspace declared (single root crate: the
defaults should already cover it; verify with `spec-spine index`).

## 2. Behavior

- Dependencies (keep the tree lean): clap (derive), clap_complete (the
  `completions` verb's script generator; part of the clap ecosystem, no
  async), serde + serde_json, toml, anyhow, thiserror, directories
  (config paths). Async arrives with spec 003 (tokio + reqwest); do not
  pre-add.
- Command tree v1 (stubs that print a clear "not implemented until
  spec NNN" error and exit 2, so help text is honest from day one):
  `statecraft login|whoami` (003), `tenants list|show` (004),
  `stamp new|status` (004), `fleet list|deploy|update|backup|remove`
  (004), `mcp` (005), plus working `version` and `completions <shell>`.
- Config: `~/.config/statecraft/config.toml` via the directories
  crate: `base_url`, `output` default; env prefix `STATECRAFT_`
  overrides file, flags override env. `statecraft config show` prints
  the effective, merged config with sources annotated.
- Output discipline: every command renders through one output layer:
  human-readable on TTY, `--output json` emits stable machine JSON
  (this is what the MCP face and scripts consume later; treat the JSON
  shapes as API from the start).
- Errors: process exit codes: 0 ok, 1 operational failure, 2 usage /
  not-implemented; errors print to stderr, never stdout.
- CI (`ci.yml`, SHA-pinned actions with version comments): fmt --check,
  clippy --all-targets -D warnings, cargo test, cargo build --release;
  plus the spec-spine governance gate mirroring the existing
  spec-spine.yml conventions (that workflow already exists and stays).

## 3. Acceptance

- `cargo build --release` produces `target/release/statecraft`;
  `statecraft --help` shows the full tree; stub commands exit 2 with
  the owning spec named; `config show` and `completions zsh` work.
- `cargo test` covers config layering (file/env/flag precedence) and
  the not-implemented exit code.
- ci.yml green on push; spine gates green.

## 4. Out of scope

- Any network I/O, token handling (003), real verbs (004), MCP (005).
- npm wrapper packaging and brew formula (post-M4 distribution spec).
