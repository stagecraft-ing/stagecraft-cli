---
id: "006-template-upgrade-verb"
title: "stagecraft template upgrade: chassis upgrades as a governed verb"
status: approved
created: "2026-07-14"
implementation: in-progress
depends_on:
  - "002-crate-scaffold"
establishes:
  - { kind: symbol, id: "stagecraft_cli::verbs::template" }
summary: >
  The upgrade half of the 2026-07-14 packaging decision: templates
  stay small because the chassis ships as versioned npm packages
  (enrahitu spec 018), and upgrading a stamped app is a verb, not a
  migration project. `stagecraft template upgrade`, run in a stamped
  app checkout, reads template.toml, bumps the chassis package pins,
  applies template-shipped codemods, runs the contract verify verb,
  and commits on a branch. The CLI orchestrates; all structure
  knowledge stays in the template and its packages. This verb is the
  boundary that keeps the CLI from ever becoming a build daemon.
---

# 006: template upgrade verb

## 1. Cross-repo dependencies (read first)

Requires enrahitu spec 018 implemented (chassis packages exist and
template.toml `[requires]` names the toolchain range). Reads only the
contract surface (spec 009 discipline: anything not in template.toml
is not the factory's or the CLI's business). If run against a
pre-018 stamped app (no chassis packages in package.json), report
"this app predates the packaged chassis" and point at the manual
re-import path; do not attempt a tree merge.

## 2. Behavior

`stagecraft template upgrade [--to <template-version>] [--dry-run]
[--no-branch]`, executed in a stamped app repo root:

1. **Preflight**: refuse on a dirty working tree; require
   template.toml present; parse `[template]`, `[contract]`,
   `[requires]`, and (when present, enrahitu spec 012)
   `[provenance]`. Resolve the target: latest published template
   version compatible with the app's contract major, or `--to`.
2. **Pin bump**: update the chassis package pins
   (@enrahitu/toolchain, @enrahitu/hiqlite-native, and any pins the
   contract's `[requires]` names) in package.json, then refresh the
   lockfile with `npm install --package-lock-only` (never a full
   platform-pruning install; same rule as enrahitu spec 014 §3).
3. **Codemods (reserved hook)**: if the target template version ships
   an `[upgrade]` table (codemod scripts, ordered, idempotent), run
   them; v1 implements the hook execution but the template may not
   ship codemods yet. Codemods are template-owned data; the CLI never
   embeds structural knowledge.
4. **Verify**: run the contract's verify verb; on failure, leave the
   branch for inspection and exit 1 with the failing step named.
5. **Commit**: on a branch `template-upgrade/<from>-<to>` (unless
   --no-branch), conventional message recording from/to versions and
   pin diffs; print the `gh pr create` suggestion rather than opening
   a PR itself in v1. `--dry-run` prints the plan and diffs, changes
   nothing.
6. JSON envelope (--output json) with {from, to, pins, codemodsRun,
   verify: pass|fail} so the platform (and later the factory's
   fleet-wide upgrade sweep) can consume the result.

## 3. Acceptance

- Mock-fixture tests: a fake stamped tree with pinned old versions
  upgrades cleanly (pins bumped, lock refreshed, verify stub run);
  dirty-tree refusal; pre-018 tree refusal with the right message;
  dry-run mutates nothing; contract-major mismatch refuses with a
  "major upgrade requires the migration path" error.
- Live check when upstream allows: upgrade a real stamped app across
  a template patch release, verify verb green, documented in the
  commit message.
- ci.yml + spine gates green.

## 4. Out of scope

- Fleet-wide upgrades across many repos (platform/factory concern;
  it will call this verb's logic through the API or a shared crate,
  designed there).
- Building, packaging, or publishing chassis packages (enrahitu spec
  018 owns the packages; this verb only consumes published versions).
- Automatic PR creation and merge (deliberate: a human or the
  platform decides; v1 prints the suggestion).

## 5. Status (2026-07-15)

Implemented: `stagecraft template upgrade` in the new
`stagecraft_cli::verbs::template` module (the symbol this spec
establishes). It is the one *local* governed verb: unlike the spec 004
verbs it never calls the control plane. It reads `template.toml`, bumps
the chassis package pins in `package.json`, refreshes the lockfile,
runs the reserved codemod hook, runs the contract's verify verb, and
commits on a `template-upgrade/<from>-<to>` branch, emitting the §2.6
`{from, to, pins, codemodsRun, verify}` result inside the shared
`{ok,data|error}` envelope. The enrahitu spec 018 gate is cleared
(chassis packages published at v0.1.0, `implementation: complete`), so
this verb is no longer parked.

v1 decisions (surfaced here for review rather than encoded silently):

- **Target ↔ pin coupling.** The template version and chassis package
  version move in lockstep (enrahitu ships root, `template.toml`, and
  all `@enrahitu/*` packages at one version), so the resolved target is
  applied as the exact pin for each discovered chassis package.
- **Compat gate = the contract's own ranges.** A `--to` (or resolved
  latest) that does not satisfy `[requires]`'s chassis range is the
  "major upgrade requires the migration path" refusal
  (`incompatible_target`). When a contract names more than one chassis
  range, the target must satisfy *all* of them, so no range can be
  crossed silently. The version policy stays in the template.
- **Chassis discovery is scope-derived.** Seeds are `package.json`
  deps whose unscoped name matches a `[requires]` key (so `node` drops
  out and `toolchain` resolves to `@enrahitu/toolchain`); the bump set
  is every exact-pinned dep sharing a seed's scope, catching companions
  like `@enrahitu/hiqlite-native` with no hardcoded scope in the CLI.
  Accepted trade-off: this assumes every exact-pinned package under a
  chassis scope releases in lockstep with the chassis (true for
  `@enrahitu/*` today); an independently-versioned same-scope package
  would need an allowlist, revisited if enrahitu ever ships one.
- **Refusal taxonomy** (all exit 1, JSON `error.kind`): `not_stamped`
  (no `template.toml`/`package.json`), `pre_chassis` (spec 006 §1: no
  chassis packages), `local_chassis` (chassis present but as `file:`
  links, i.e. a template-development tree, not a stamped app),
  `bad_target`, `incompatible_target`, `dirty_tree`. Preflight order:
  structural refusals first, then the dirty-tree gate (before target
  resolution, so a dirty tree fails fast with no wasted registry read),
  then target/compat. A verify failure is not a refusal: matching how
  `stamp status --watch` renders a terminal `failed` job, it is a
  completed run whose `{ok:true, data}` result carries `verify:"fail"`
  and whose exit code (1) carries the failure, with the branch left for
  inspection. The two-shape envelope contract holds (`ok:true` -> read
  `data`, `ok:false` -> read `error`); the verdict lives in the data.
- **Dependency added:** `semver` (pure Rust) for correct
  version/range comparison. Couples to spec 002's Cargo territory; the
  standing `Spec-Drift-Waiver` covers the crate-file edits. Note: this
  is Cargo-flavored semver, where a bare `[requires]` range (`"0.1.0"`)
  parses as caret, not an npm exact pin; contract authors must write an
  explicit operator (`^0.1`, `>=24`), as enrahitu's contract does.
- **`--to` omitted** resolves the greatest published version the range
  allows via an `npm view` registry read, behind the runner seam so
  tests stay offline.

Covered by tests (all offline; every git/npm/node effect is behind a
`Runner` trait): scope-derived discovery with companions, the
`pre_chassis`/`local_chassis`/`not_stamped`/`incompatible_target`/`dirty_tree`
refusals, the multi-range gate refusing when any seed range excludes
the target, format-preserving pin rewrite, the dry-run plan that
mutates nothing, latest-resolution when `--to` is omitted, the no-op
that still bumps a companion left behind the primary, the happy path
(branch -> lock -> verify -> commit), the verify-failure path that
leaves the branch uncommitted, and the result envelope staying
`{ok:true, data:{verify:"fail"}}` with a `Rendered` exit 1. Three
end-to-end binary checks drive the offline paths (not-stamped,
pre-018, dry-run) with the process working directory set to a fixture.

Outstanding: the §3 live check cannot run yet. It needs a real stamped
app carrying published-version chassis pins plus a newer published
template version to upgrade across; enrahitu's scaffold verb (spec 014)
is still v0 and "edits no dependencies", so no such app exists, and
there is only the v0.1.0 release to pin to. This spec stays
`implementation: in-progress` until a stamped app and a patch release
exist; at that point the only open item is the live upgrade transcript
(no code change), and the commit records its pointer.
