---
id: "004-governance-verbs"
title: "Governance verbs v1: tenants, stamps, fleet"
status: approved
created: "2026-07-14"
implementation: in-progress
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
- Local certificate verification: when the signed-cert path lands
  (enrahitu spec 012 §7, born-with certVersion 2), verification is
  repo-local via pinned tenant-tail; the CLI does not wrap it.

## 5. Endpoint map and JSON envelope (2026-07-15 amendment)

§2 named the verbs but left the concrete HTTP mapping, the request
bodies, and the envelope's error arm implicit. This section inlines
them, read off stagecraft specs 004 (tenants), 005 (factory), and 006
(fleet) on 2026-07-15. §1's divergence rule still governs: if a live
plane differs, the platform spec wins and this section is re-amended,
never coded around.

### 5.1 Endpoint map (all paths under the plane's `/api/v1`)

| Verb | Method + path | Request body |
| --- | --- | --- |
| `tenants list` | GET `/tenants` | (none) |
| `tenants show <id>` | GET `/tenants/:id` | (none) |
| `tenants install-url <id>` | GET `/tenants/:id/github/install-url` | (none) |
| `stamp new` | POST `/tenants/:id/stamps` | `{appName, targetOrg, posture, frontend?}` |
| `stamp status <jobId>` | GET `/stamps/:jobId` | (none) |
| `fleet list --tenant <id>` | GET `/tenants/:id/fleet` | (none) |
| `fleet deploy` | POST `/tenants/:id/fleet` | `{name, image}` |
| `fleet update <appId>` | POST `/fleet/:appId/update` | `{image}` |
| `fleet backup <appId>` | POST `/fleet/:appId/backup` | `{}` |
| `fleet remove <appId>` | DELETE `/fleet/:appId` | `{confirm}` |

Every mapping above is verbatim from the platform specs except one:
`fleet list` -> GET `/tenants/:id/fleet`. Stagecraft spec 006 defines
the fleet collection's deploy (POST `/tenants/:id/fleet`) and the
per-app status/update/backup/remove, but names no list endpoint. The
CLI inlines the symmetric collection GET as its expectation, mirroring
the factory's `GET /tenants/:id/stamps` list; if the plane lands a
different list shape, §1 governs. `stamp new`'s `posture` is a required
field the caller must supply: the factory rejects a defaulted posture
by design (stagecraft spec 005 §3, pipeline step 3), so the CLI makes
`--posture` a required flag with no default and forwards it as
`posture`. `fleet remove`'s `confirm` carries the literal app name the
operator echoes (stagecraft spec 006 §3, destructive guard); there is
no `--force`/`--yes` shortcut.

### 5.2 JSON envelope (`--output json`)

Both faces (spec 005 reuses this) consume one stable envelope:

- Success: `{"ok": true, "data": <platform response, passed through
  verbatim>}`. The CLI does not reshape the plane's payload; it wraps
  it, so the envelope stays stable as platform fields grow.
- Failure: `{"ok": false, "error": {"kind": <k>, "message": <m>,
  "status"?: <http status>}}`, where `kind` is one of `network`,
  `unauthenticated`, `api`, `server`, `decode` (the spec 003 taxonomy).
  The envelope is emitted on stdout and the process still exits 1.

In human mode, success renders as an aligned table (ids, names, status,
age); failure prints the taxonomy message to stderr and exits 1 (the
spec 002 exit-code contract). Only the request taxonomy above is
enveloped. A pre-flight failure the CLI detects before any request is
reported plainly (stderr) in both formats, never wrapped: a missing
base URL is usage (exit 2), and a missing stored credential is
operational (exit 1) with the `run login` hint, matching `whoami`.

### 5.3 Response-shape expectations (human rendering only)

The passthrough envelope means `--output json` is faithful to whatever
the plane returns. Human tables need named fields, so the CLI reads a
tolerant subset (unknown fields ignored, absent optional fields blank):
list endpoints return a JSON array of records; `tenants show` returns
one tenant carrying an `installations` array; stamp and fleet records
carry at least `id` and `status`. These are the CLI's expectation per
§1; a shape the human renderer cannot read surfaces as a `decode`
error (exit 1), which is the intended drift signal.

### 5.4 Watch semantics (`stamp status --watch`)

Poll GET `/stamps/:jobId` starting at 2s and backing off toward a 10s
cap, emitting each observed state change (a line in human mode, an
envelope in JSON mode) rather than every poll. A terminal `green`
exits 0; a terminal `failed` exits 1 (the final state is already on
stdout, so no additional stderr line). A transport error mid-poll
aborts through the normal taxonomy after the client's GET retries are
exhausted.

## 6. Status (2026-07-15)

Implemented: the full §2/§5.1 verb surface (tenants list/show/
install-url, stamp new/status with `--watch`, fleet list/deploy/update/
backup/remove) in the new `stagecraft_cli::verbs` module over the spec
003 client, which grew GET/POST/DELETE returning a passthrough
`serde_json::Value`. Human output is aligned tables; `--output json`
emits the §5.2 `{ok, data|error}` envelope. The `--posture` guard is
required with no default and the `fleet remove` confirm-name guard is
surfaced verbatim, both per §2. A missing service's 404 surfaces as
"not enabled on this control plane" (exit 1, not a crash) per §1.

Covered by 70 tests (58 unit + 12 integration): per-verb happy path,
api-error surfacing, missing-service 404 mapping, watch-mode terminal
states, and a JSON envelope snapshot per verb family. The release
binary was additionally driven end-to-end against a mock plane: every
verb renders in both formats with the right exit codes, the
non-idempotent DELETE is not retried, and `--debug` never leaks the
token.

Outstanding: the §3 manual e2e against a **live** control plane cannot
run. Stagecraft's tenants/factory/fleet services (its specs 004/005/006)
are all `implementation: pending`, so no reachable plane serves these
endpoints; the CLI is built for exactly that case (a missing service is
a clean exit-1 404, §1). This spec stays `implementation: in-progress`
until a live plane exists; at that point the only open item is the
live e2e (login, then a governed read/write against real tenants,
factory, and fleet services), which drops in with no code change.
