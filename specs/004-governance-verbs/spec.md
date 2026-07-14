---
id: "004-governance-verbs"
title: "Governance verbs v1: tenants, stamps, fleet"
status: approved
created: "2026-07-14"
implementation: pending
depends_on:
  - "003-auth-api-client"
establishes:
  - { kind: symbol, id: "stagecraft_cli::verbs" }
summary: >
  The CLI face becomes useful: the stub commands from spec 002 gain
  real implementations over the API client, mirroring the control
  plane's tenants (stagecraft spec 004), factory (spec 005), and fleet
  (spec 006) services. Every verb has a stable JSON output shape,
  because spec 005 exposes these same verbs as MCP tools and the JSON
  is the shared contract between both faces.
---

# 004: Governance verbs

## 1. Cross-repo dependency

Endpoint shapes are defined by the stagecraft repo's specs 004/005/006
and inlined below as the CLI's expectation; if the live control plane
diverges, the platform spec wins: update the shapes here via amendment
rather than coding to drift. Services that do not exist yet on the
target plane must surface as "not enabled on this control plane"
(HTTP 404 from a missing service), exit 1, not a crash.

## 2. Verb surface (all support --output json)

- `stagecraft tenants list` -> GET /api/v1/tenants
- `stagecraft tenants show <id>` -> GET /api/v1/tenants/:id (includes
  installations)
- `stagecraft tenants install-url <id>` -> GET
  /api/v1/tenants/:id/github/install-url (prints the URL; `--open`
  opens the browser)
- `stagecraft stamp new --tenant <id> --app <name> --org <org>
  [--frontend vue] --posture <none|assisted|autonomous>` -> POST
  /api/v1/tenants/:id/stamps. Posture is a REQUIRED flag with no
  default: the CLI must not invent a posture (the platform rejects
  defaulted postures by design).
- `stagecraft stamp status <jobId> [--watch]` -> GET
  /api/v1/stamps/:jobId; --watch polls (2s interval, backoff to 10s)
  until green|failed, exit 0|1 accordingly, streaming state changes as
  they happen.
- `stagecraft fleet list --tenant <id>`; `stagecraft fleet deploy
  --tenant <id> --app <fleetApp> --image <ref>`; `stagecraft fleet
  update <appId> --image <ref>`; `stagecraft fleet backup <appId>`;
  `stagecraft fleet remove <appId> --confirm <name>` (the confirm-name
  guard from stagecraft spec 006 is surfaced verbatim; no
  --force/--yes shortcut exists, deliberately).
- Human output: aligned tables with the fields a operator scans for
  (ids, names, status, age); JSON output: the platform response passed
  through with a stable envelope {ok, data|error}.

## 3. Acceptance

- Mock-server tests per verb: happy path, api-error surfacing,
  missing-service 404 mapping, watch-mode terminal states; a JSON
  snapshot test per verb locking the envelope.
- Manual e2e against a live plane for whichever services exist at
  implementation time, documented in the commit message.
- ci.yml + spine gates green.

## 4. Out of scope

- Approvals verbs (arrive when the platform's approvals surface
  exists, follow-up to stagecraft spec 008).
- MCP exposure (005).
- Any local stamping (stamping is the platform's job; the CLI only
  triggers and watches).
