# CLAUDE.md: statecraft-cli

## Project Overview

The successor to OPC: one Rust binary named `statecraft` with two
faces, CLI subcommands for humans and an MCP server (stdio) for
agents, both calling the Statecraft control plane's API under the same
identity, guards, and JSON shapes. Thesis and decided constraints:
`specs/001-cli-mcp-thesis/spec.md`. The build order is the spec
backlog: 002 crate scaffold, 003 auth + API client, 004 governance
verbs, 005 MCP server.

## Repository Structure

```
specs/       Feature specs, the authoritative design record
standards/   spec-spine constitution, contract, templates
.derived/    Compiler output (committed shards; never hand-edit)
.claude/     rules (orchestrator, governed reads, adversarial refusal)
```

Planned by spec 002: `Cargo.toml`, `src/`, `.github/workflows/ci.yml`.

## Governance

Governed by spec-spine (`spec-spine.toml`, owned by spec 000): specs
are the source of truth; read `.derived/**` only through `spec-spine`
subcommands; after editing any `specs/*/spec.md`, run
`spec-spine compile && spec-spine index` and commit the shards with
the edit. Gates before every commit: `spec-spine lint --fail-on-warn`
and `spec-spine index check`.

## Build Commands

```bash
spec-spine compile && spec-spine index && spec-spine lint
# after spec 002 lands:
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release        # target/release/statecraft
```

## Key Conventions

- **Guards are product surface.** The required `--posture` flag on
  stamps and the `--confirm <name>` on fleet remove exist by design;
  never add a bypass flag.
- **JSON output shapes are API.** The MCP face reuses the CLI's JSON
  envelopes; treat them as versioned contracts from the first verb.
- **The CLI never bypasses the platform.** No local stamping, no
  direct kubeconfig access; it triggers and watches governed verbs.
- Apache-2.0; rustls only (no native-tls).
