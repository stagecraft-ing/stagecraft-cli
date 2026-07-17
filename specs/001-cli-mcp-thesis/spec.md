---
id: "001-cli-mcp-thesis"
title: "statecraft-cli: one binary, two faces (CLI verbs + MCP server)"
status: approved
created: "2026-07-14"
implementation: pending
depends_on:
  - "000-bootstrap"
establishes:
  - "README.md"
summary: >
  The successor to OPC (the retired Tauri desktop cockpit): a single
  binary named statecraft that exposes the platform's governance verbs
  twice, as CLI subcommands for humans and as an MCP server for agents.
  The MCP face is the product's genuinely unique surface: any agent
  (Claude Code first) operates natively under Statecraft governance,
  requesting approvals, checking spec-code coupling, and triggering
  factory stages. This is milestone M4 in the Statecraft ladder; the spec
  records the thesis and the decided constraints so the repo is born
  governed ahead of its build.
---

# 001: statecraft-cli thesis

## 1. Purpose

OPC (the Open Agentic Platform's Tauri + React desktop cockpit) had thin
value over the platform API as a windowed app; its real asset was where it
ran: on the developer's machine, next to the code and the agents. This
repo rebuilds that asset in its natural shape: one binary, two faces.

- **CLI face** (humans): governance verbs as subcommands: authenticate
  against a Statecraft control plane, inspect tenants and fleets, request
  and grant approvals, check coupling status, trigger and watch factory
  stages.
- **MCP face** (agents): the same verbs exposed as MCP tools over stdio,
  so a coding agent operates under Statecraft governance natively instead
  of shelling out around it. Claude Code is the first target client.

Both faces call the same control-plane API with the same identity model;
the MCP face is not a privileged side door.

## 2. Decided constraints

- **Binary name `statecraft`**; npm distribution as `statecraft-cli` (the
  bare npm name is taken; brew/cargo naming is uncontested). The OPC name
  retires with the desktop app.
- **Implementation language: Rust** (single static binary, no runtime
  dependency on the customer machine; the MCP server speaks stdio).
- **License Apache-2.0.** The CLI is the funnel and the agent bridge;
  AGPL binaries in corporate dev loops cost adoption for zero protective
  benefit. The AGPL boundary stays at the control plane.
- **No TUI** until demand proves it; the web governance UI absorbs
  graphical needs.

## 3. Territory

`README.md` now; the crate and its manifests when the build starts (M4 in
the Statecraft milestone ladder; see statecraft spec 001 §6). Verb
inventory and API surface graduate into their own numbered specs at that
point, informed by the OPC governance-verb inventory
(knowledge://open-agentic-platform/product/apps/opc).

## 4. Out of scope

- Control-plane behavior (statecraft repo).
- Template mechanics (enrahitu repo).
- A desktop or graphical client of any kind.
