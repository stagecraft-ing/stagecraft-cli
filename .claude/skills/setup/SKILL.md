---
name: setup
description: One-time contributor setup. Install spec-spine, verify the Rust toolchain, and check the governed loop (compile, index check, lint, couple) so /init can report lifecycle and structural counts.
allowed-tools: Bash, Read
---

# Setup

Get a fresh clone operational. After this completes, `/init` can report
lifecycle and structural counts through the `spec-spine` binary, never by
ad-hoc parsing of `.derived/**/*.json` (see
`.claude/rules/governed-artifact-reads.md`).

## Process

### 1. Verify the Rust toolchain

This repo builds one Rust binary (`statecraft`, spec 002). Check the
toolchain the gates need:

```bash
cargo --version
rustup component list --installed | grep -E 'rustfmt|clippy'
```

If cargo is absent, install it via rustup (https://rustup.rs) before
continuing. `rustfmt` and `clippy` back the cargo gates once spec 002
lands: `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings`.

### 2. Install spec-spine

```bash
cargo install spec-spine-cli
```

Verify with `spec-spine --version`. Halt on a non-zero exit and surface the
failing step verbatim.

### 3. Compile a fresh registry

```bash
spec-spine compile
```

`.derived/` is committed in this repo and `compile` is deterministic: a
no-op on a clean tree. Run it before any read so the registry reflects the
working tree, and commit the regenerated shards whenever `specs/*/spec.md`
changes.

### 4. Verify the governed loop

Smoke-test the gates `/init` and CI depend on. Passing here means the loop works
on this clone:

```bash
spec-spine index check           # codebase index staleness gate
spec-spine lint --fail-on-warn   # corpus conformance
spec-spine couple --base origin/main --head HEAD   # PR-time coupling gate
```

If `index check` exits non-zero the committed index is stale against current
inputs. Run `spec-spine index`, re-commit the regenerated shards, then
re-check. Do not parse `.derived/**/*.json` directly to "verify" success.

### 5. Emit summary

Report exactly:

```
## setup: statecraft-cli

**Toolchain:** {cargo <version> / missing}
**Install:** {ok / failed at <step>}
**Governed loop:**
  - compile: {fresh registry / failed}
  - index check: {fresh / stale}
  - lint: {clean / N diagnostics}
  - couple: {clean / drift surfaced}
**Lifecycle:** {N specs across <statuses>}  (from registry status-report)

Next: run `/init` to load full session context.
```

Do not invent counts. Only report values that came back from a `spec-spine`
subcommand.

## Rules

- The loop runs through the installed `spec-spine` binary on your `PATH`.
- Halt on first failure. Do not silently continue past a missing prerequisite
  or a failing gate.
- Never parse `.derived/**/*.json` directly in any verification step. Use the
  `spec-spine` subcommands.
- Idempotent: safe to re-run.
