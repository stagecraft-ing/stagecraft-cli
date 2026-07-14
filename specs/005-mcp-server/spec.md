---
id: "005-mcp-server"
title: "The MCP face: stagecraft mcp (stdio server)"
status: approved
created: "2026-07-14"
implementation: pending
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
