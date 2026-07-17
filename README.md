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

Born governed, pre-code. This is milestone M4 in the Statecraft ladder;
the thesis and decided constraints (binary name, Rust, stdio MCP,
Apache-2.0, no TUI) live in `specs/001-cli-mcp-thesis/spec.md`. The crate
lands when M4 starts.

## Governance

Governed by [spec-spine](https://github.com/statecrafting/spec-spine)
(`cargo install spec-spine-cli`): `spec-spine compile | index | lint`,
coupling gate at PR time, derived shards under `.derived/` are
compiler-owned.

## License

Apache-2.0 (see [LICENSE](LICENSE)).
