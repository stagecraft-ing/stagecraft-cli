# statecraft-cli

One binary, two faces. `statecraft` exposes the
[Statecraft](https://github.com/statecrafting/statecraft) control
plane's governance verbs as CLI subcommands for humans and as an MCP
server for agents, so a coding agent (Claude Code first) can request
approvals, check spec-code coupling, and trigger factory stages natively
under governance instead of shelling out around it.

Successor to OPC, the Open Agentic Platform's desktop cockpit; the
desktop app is retired, the governance verbs live on here.

## Status

Milestone M4 in the Statecraft ladder, implemented: the crate scaffold
(002), auth + API client (003), the governance verbs (004), the MCP
stdio server (005), and the template upgrade verb (006). The thesis and
decided constraints (binary name, Rust, stdio MCP, Apache-2.0, no TUI)
live in `specs/001-cli-mcp-thesis/spec.md`.

## Install

Prebuilt binaries for macOS and Linux (spec 007):

```sh
curl -fsSL https://raw.githubusercontent.com/statecrafting/statecraft-cli/main/install.sh | sh
```

Windows: download the `.zip` from the
[Releases](https://github.com/statecrafting/statecraft-cli/releases)
page. Every release archive ships a `.sha256` sidecar, a CycloneDX
SBOM, and a SLSA build-provenance attestation; the installer verifies
the checksum and (best-effort) the attestation. From source:
`cargo install --git https://github.com/statecrafting/statecraft-cli`
(rustls only, no OpenSSL).

## Governance

Governed by [spec-spine](https://github.com/statecrafting/spec-spine)
(`cargo install spec-spine-cli`): `spec-spine compile | index | lint`,
coupling gate at PR time, derived shards under `.derived/` are
compiler-owned.

## License

Apache-2.0 (see [LICENSE](LICENSE)).
