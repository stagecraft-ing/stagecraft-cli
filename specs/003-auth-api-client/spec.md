---
id: "003-auth-api-client"
title: "Auth + control-plane API client"
status: approved
created: "2026-07-14"
implementation: pending
depends_on:
  - "002-crate-scaffold"
establishes:
  - { kind: symbol, id: "stagecraft_cli::api" }
  - { kind: symbol, id: "stagecraft_cli::auth" }
summary: >
  The binary learns to authenticate against a Stagecraft control plane
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

A reachable control plane (stagecraft repo, specs 002+004) is needed
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
  1. `stagecraft login [--base-url URL]` must work on a headless
     machine with a browser elsewhere: print a URL + one-time code, or
     open a localhost callback when a browser is local. The rauthy
     OIDC authorization-code flow with a loopback redirect
     (RFC 8252) is the preferred shape; a control-plane-issued
     personal access token pasted at a prompt is an acceptable v1
     fallback if the loopback flow needs control-plane changes that
     do not exist yet (report which).
  2. Credentials at `~/.config/stagecraft/credentials.toml`, mode
     0600, keyed by base_url (multiple planes allowed). Refresh
     transparently when the mechanism supports it; on 401, error with
     "run stagecraft login".
- `stagecraft whoami`: GET the control plane's auth identity endpoint
  (`/api/v1/auth/me` in the chassis) and render id/email; exit 1 when
  unauthenticated.
- **api module**: base_url + credentials -> a client with: typed
  request/response helpers, uniform error taxonomy (network, auth,
  api-4xx with server message, api-5xx), retry with jitter for
  idempotent GETs only, and a `--debug` flag dumping request/response
  metadata (never credential material) to stderr.
- JSON output shapes for whoami defined and tested (the MCP face
  reuses them).

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
