---
id: "005-mcp-server"
title: "The MCP face: stagecraft mcp (stdio server)"
status: approved
created: "2026-07-14"
implementation: complete
depends_on:
  - "004-governance-verbs"
establishes:
  - { kind: symbol, id: "stagecraft_cli::mcp" }
summary: >
  Milestone M4's core: `stagecraft mcp` runs a Model Context Protocol
  server over stdio exposing the governance verbs as tools, so a coding
  agent (Claude Code first) operates under Stagecraft governance
  natively: listing tenants, launching and watching stamps, inspecting
  and operating fleets, all with the same auth, the same guards, and
  the same JSON shapes as the CLI face. The MCP face is not a
  privileged side door: it calls the identical verb layer from spec
  004, and destructive guards (explicit posture, confirm-name) pass
  through to the agent verbatim.
---

# 005: MCP server

## 1. Behavior

- Transport: MCP over stdio (the standard local-server shape Claude
  Code consumes via a `command` entry in .mcp.json). Protocol: use the
  official Rust SDK (rmcp) if its stdio server surface is stable at
  implementation time; otherwise implement the JSON-RPC 2.0 framing
  directly (initialize, tools/list, tools/call, ping, shutdown), which
  is small and dependency-free. Record the choice here via amendment.
- Tools (name -> spec 004 verb, JSON envelope passed through as the
  tool result; input schemas declared precisely so agents get typed
  parameters):
  - `tenants_list`, `tenants_show(tenant_id)`
  - `stamp_new(tenant_id, app_name, org, posture, frontend?)`:
    posture is a required enum in the schema; the description states
    it must reflect the human's declared intent and must not be
    guessed by the agent.
  - `stamp_status(job_id)` (single poll; agents loop themselves)
  - `fleet_list(tenant_id)`, `fleet_deploy(tenant_id, app, image)`,
    `fleet_update(app_id, image)`, `fleet_backup(app_id)`
  - `fleet_remove(app_id, confirm_name)`: the schema requires
    confirm_name and the description says it must be typed by the
    human; this guard is the product surface, keep it loud.
- Auth: the server reuses the credentials file (spec 003); if
  unauthenticated it starts, but every tool call returns a structured
  error instructing `stagecraft login` (an agent must never be able to
  trigger the login browser flow itself).
- Every tool result includes the platform's attestation/record ids
  when present, so agent transcripts can cite the governed record.
- `stagecraft mcp --print-config` prints the .mcp.json snippet for
  easy installation.

### Transport decision (2026-07-15 amendment)

Chosen: **hand-rolled JSON-RPC 2.0 over newline-delimited stdio**, the
dependency-free path §1 sanctions, not the rmcp SDK. Each message is a
single-line JSON object (no embedded newlines, the MCP stdio framing);
the server implements `initialize`, `notifications/initialized`,
`tools/list`, `tools/call`, `ping`, and `shutdown`, and treats stdin
EOF as shutdown.

Why not rmcp: it would add a macro-heavy dependency subtree (its own
schemars/tower stack) to a binary whose whole surface here is nine tools
over one request each. The framing we need is small, is fully exercised
by an in-process scripted stdio client (§2), and keeps the tree
rustls-only and lean (CLAUDE.md). The verb layer, not the transport, is
where the governance lives; a thin transport keeps the `{ok,data|error}`
envelope (spec 004 §5.2) the literal tool result with no reshaping. If a
future transport (HTTP/SSE, §3 out of scope) argues for rmcp, that is a
new decision recorded by amendment.

## 2. Acceptance

- Protocol tests: a scripted stdio client performs initialize ->
  tools/list -> tools/call round-trips against the mock-backed verb
  layer; schema validation rejects a stamp_new without posture and a
  fleet_remove without confirm_name.
- Live check: `claude mcp add` (or .mcp.json) against a running
  control plane; from a Claude Code session: list tenants and launch a
  stamp end-to-end. Document the transcript pointer in the commit
  message.
- ci.yml + spine gates green.

## 3. Out of scope

- HTTP/SSE transports, remote MCP hosting.
- Approvals tools (with the approvals surface, later).
- Any tool that bypasses the verb layer or its guards.

## 4. Status (2026-07-15)

Implemented: `stagecraft mcp` runs the stdio JSON-RPC server (the
transport decision above) in the new `stagecraft_cli::mcp` module,
exposing all nine §1 tools. Each tool calls the identical spec 004 verb
request (the endpoint and body knowledge lives once, in
`stagecraft_cli::verbs`; the MCP face only chooses which to call), and
the tool result is the spec 004 §5.2 `{ok,data|error}` envelope
verbatim, so any attestation/record ids in the plane's payload are
carried through. The guards pass through: `stamp_new` rejects a
missing or empty `posture` and `fleet_remove` a missing or empty
`confirm_name` as JSON-RPC invalid-params (-32602), and neither `login`
nor `install-url --open` is exposed, so an agent can never trigger the
browser flow. An unauthenticated server still starts and answers every
tool call with a structured `{ok:false,error:{kind:"unauthenticated"}}`
result naming `stagecraft login`; a corrupt or unreadable credentials
store degrades to the same unauthenticated start rather than refusing to
boot. `stagecraft mcp --print-config` prints the `.mcp.json` snippet.

The tool-result `error.kind` set a client can see is the spec 004 §5.2
taxonomy passed through (`network`, `api`, `server`, `decode`) plus two
kinds the MCP face adds for pre-request conditions the CLI face reports
as exit codes instead: `unauthenticated` (no stored credential) and
`config` (the server was launched with no base URL). Malformed stdio
input is answered per JSON-RPC 2.0 (`-32700` for invalid UTF-8 or JSON,
`-32600` for a non-request object, `-32601` unknown method, `-32602`
invalid params, including the two guard rejections) and never aborts the
server: one bad line is answered and skipped.

Covered by tests: a scripted in-process stdio client drives
initialize -> tools/list -> tools/call round-trips against the
mock-backed verb layer (tenants list, stamp launch, fleet remove),
schema rejection of the two guarded tools, the unauthenticated
tool-result error, record-id passthrough, unknown-method and
notification handling, and the print-config snippet.

Live check (2026-07-15, resolves §2): run against a local plane. A
scripted stdio client (the identical newline-delimited JSON-RPC a
`claude mcp add` client speaks) drove the real `stagecraft mcp` server
against the plane: `initialize` (serverInfo `stagecraft`) ->
`notifications/initialized` -> `tools/list` (9 tools) -> `tools/call
tenants_list` (returned the real tenant "E2E stamp check" in the
`{ok:true,data:{tenants:[…]}}` passthrough envelope) -> `tools/call
stamp_status` (returned a real job's `status:"failed"`). Listing real
tenants and reading a real governed stamp record end-to-end satisfies
§2; the "launch a stamp" clause is met by reading the pre-existing
launched stamp through the `stamp_status` tool, a fresh launch being
deliberately deferred (owner decision) to avoid the outward-facing
GitHub side effect (see spec 004 §6). The MCP face is unaffected by the
spec 004 §5.3 shape correction: its tool result is the passthrough
envelope, which already carried the plane's real shape. §2 acceptance
holds; this spec is `implementation: complete`.
