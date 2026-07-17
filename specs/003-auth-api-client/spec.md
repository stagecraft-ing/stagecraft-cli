---
id: "003-auth-api-client"
title: "Auth + control-plane API client"
status: approved
created: "2026-07-14"
implementation: complete
depends_on:
  - "002-crate-scaffold"
establishes:
  - { kind: symbol, id: "statecraft_cli::api" }
  - { kind: symbol, id: "statecraft_cli::auth" }
summary: >
  The binary learns to authenticate against a Statecraft control plane
  and speak its API. Auth v1 is a browser-assisted session-cookie
  handoff (the control plane's chassis auth is cookie based, and the
  embedded rauthy exposes OIDC; the exact mechanism is
  DECIDE-AT-IMPLEMENTATION between OAuth device-flow-style polling and
  a localhost callback, constrained below). Tokens/cookies are stored
  in a 0600 credentials file, never in the config file. An api module
  gives every later verb a typed, authenticated request path with
  consistent error mapping.
---

# 003: Auth + API client

## 1. Cross-repo dependency

A reachable control plane (statecraft repo, specs 002+004) is needed
for live verification. Everything unit-testable must be tested against
a local mock HTTP server (httpmock or wiremock crate); do not gate
`cargo test` on a live platform. If no control plane is reachable for
the manual e2e, implement, test against the mock, and report the live
check as pending.

## 2. Behavior

- New deps: tokio (rt-multi-thread), reqwest (json, cookies,
  rustls-tls; never native-tls), httpmock (dev).
- **Auth mechanism constraints** (implementer picks the simplest that
  the control plane's auth surface supports, and records the choice
  here via amendment):
  1. `statecraft login [--base-url URL]` must work on a headless
     machine with a browser elsewhere: print a URL + one-time code, or
     open a localhost callback when a browser is local. The rauthy
     OIDC authorization-code flow with a loopback redirect
     (RFC 8252) is the preferred shape; a control-plane-issued
     personal access token pasted at a prompt is an acceptable v1
     fallback if the loopback flow needs control-plane changes that
     do not exist yet (report which).
  2. Credentials at `~/.config/statecraft/credentials.toml`, mode
     0600, keyed by base_url (multiple planes allowed). Refresh
     transparently when the mechanism supports it; on 401, error with
     "run statecraft login".
- `statecraft whoami`: GET the control plane's auth identity endpoint
  (`/api/v1/auth/me` in the chassis) and render id/email; exit 1 when
  unauthenticated.
- **api module**: base_url + credentials -> a client with: typed
  request/response helpers, uniform error taxonomy (network, auth,
  api-4xx with server message, api-5xx), retry with jitter for
  idempotent GETs only, and a `--debug` flag dumping request/response
  metadata (never credential material) to stderr.
- JSON output shapes for whoami defined and tested (the MCP face
  reuses them).

### Auth mechanism decision (2026-07-14 amendment)

Chosen v1 mechanism: **browser-assisted bearer-token handoff (paste)**,
the fallback sanctioned by constraint 1 above. `statecraft login
[--base-url URL]` guides the operator to sign in through a browser at
the control plane, then reads the resulting chassis session token
(`access_token`) from stdin (a piped value or an interactive prompt, so
a headless machine with a browser elsewhere is supported), validates it
with `GET /api/v1/auth/me`, and writes it to the credentials file. The
api module replays the stored token as `Authorization: Bearer` on every
request.

Why not the preferred RFC 8252 loopback OIDC flow: it needs two
control-plane additions that do not exist today. Verified against the
enrahitu chassis that the control plane imports (chassis specs
004-auth-core, 005-rauthy-same-origin, both complete):

1. No public rauthy OIDC client with a `127.0.0.1:<port>` loopback
   redirect URI. The sole bootstrapped client (`enrahitu`) is
   confidential, its redirect is fixed to `localhost:4000`, and dynamic
   client registration is off. The `device_code` grant (RFC 8628) is
   not enabled either, so device flow is equally blocked.
2. No endpoint that returns the app-minted token to a local listener.
   The gateway verifies an app-issued RS256 JWT (issuer `enrahitu`,
   deposited only as an httpOnly cookie), not a rauthy-issued token, so
   even a perfect loopback exchange against rauthy would yield a token
   the API rejects. There is also no personal-access-token or API-key
   issuance surface.

Forward compatibility: the credentials store and the Bearer replay in
the api module are independent of how the token was acquired. When the
control plane adds the loopback client plus a token-return endpoint,
`login` gains that acquisition path with no change to the credentials
contract or the api module.

Refresh: the paste mechanism hands the CLI no refresh token, so v1 has
no transparent refresh; on HTTP 401 the CLI errors with "run statecraft
login". Longer-lived or refreshable CLI tokens are control-plane work
to add alongside the loopback flow. (The chassis `access_token` is a
15-minute JWT, a control-plane limitation reported here, not a CLI
defect.)

## 3. Acceptance

- Unit: credentials file round-trip with permission assertion; 401 ->
  login-hint error; retry only on GET; mock-server tests for whoami
  happy/sad paths.
- Manual e2e vs a locally running control plane: login, whoami, and a
  raw authenticated GET documented in the commit message.
- ci.yml + spine gates green.

## 4. Out of scope

- Governance verbs (004) and MCP (005).
- OS keychain integration (file + 0600 is v1; a later spec may add
  keychain backends).
- Multi-user/profile switching UX beyond per-base-url entries.

## 5. Status (2026-07-14)

Implementation landed: the `auth` and `api` modules, the credentials
store (0600, per-base-url), `login` (paste handoff) and `whoami`, the
uniform error taxonomy, GET-only retry with jitter, and `--debug`, all
covered by unit and mock-server (httpmock) tests.

Live e2e (2026-07-15, resolves §3): run against a local plane. The
chassis mock auth driver mints a session token (`GET
/api/v1/auth/mock/login`); `login` took it over the bearer handoff and
validated it against `/api/v1/auth/me`, `whoami` rendered the identity
(`admin@example.com`) in both formats, and a raw authenticated `GET
/api/v1/tenants` returned 200. `--debug` was confirmed to leak the
token zero times. The gateway accepts `Authorization: Bearer` (verified
in the enrahitu chassis `backend/auth/handler.ts`), so the bearer-replay
model holds against the real plane. §3 acceptance holds; this spec is
`implementation: complete`.
