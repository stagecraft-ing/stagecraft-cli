---
id: "006-template-upgrade-verb"
title: "statecraft template upgrade: chassis upgrades as a governed verb"
status: approved
created: "2026-07-14"
implementation: complete
depends_on:
  - "002-crate-scaffold"
establishes:
  - { kind: symbol, id: "statecraft_cli::verbs::template" }
summary: >
  The upgrade half of the 2026-07-14 packaging decision: templates
  stay small because the chassis ships as versioned npm packages
  (enrahitu spec 018), and upgrading a stamped app is a verb, not a
  migration project. `statecraft template upgrade`, run in a stamped
  app checkout, reads template.toml, bumps the chassis package pins,
  applies template-shipped codemods, runs the contract verify verb,
  and commits on a branch. The CLI orchestrates; all structure
  knowledge stays in the template and its packages. This verb is the
  boundary that keeps the CLI from ever becoming a build daemon.
---

# 006: template upgrade verb

## 1. Cross-repo dependencies (read first)

Requires the packaged chassis to exist and `template.toml`
`[requires]` to name its version range. enrahitu spec 018 established
this; the packages have since moved from the retired `@enrahitu` scope
to `@statecrafting` (statecrafting spec 002 for the toolchain, spec
003 for the `hiqlite-native` addon), and they are now versioned
independently rather than in lockstep: `@statecrafting/toolchain` is
published at 0.1.0/0.2.0/0.3.0 while `@statecrafting/hiqlite-native`
and `@statecrafting/kernel-native` remain at 0.1.0. The verb reads
only the contract surface (spec 009 discipline: anything not in
template.toml is not the factory's or the CLI's business) and never
hardcodes a scope, so the rename cost it nothing; the independent
versioning is what reshaped the resolution model (§2, §5). If run
against a pre-018 stamped app (no chassis packages in package.json),
report "this app predates the packaged chassis" and point at the
manual re-import path; do not attempt a tree merge.

## 2. Behavior

`statecraft template upgrade [--to <template-version>] [--dry-run]
[--no-branch]`, executed in a stamped app repo root:

1. **Preflight**: refuse on a dirty working tree; require
   template.toml present; parse `[template]`, `[contract]`,
   `[requires]`, and (when present, enrahitu spec 012)
   `[provenance]`.
2. **Per-package target resolution and pin bump**: resolve each
   discovered chassis package to its own target independently, because
   the chassis packages no longer move in lockstep (§1). A package the
   contract names in `[requires]` (a seed, e.g.
   `@statecrafting/toolchain`) resolves the greatest published version
   its named range allows; a companion discovered by shared scope but
   unnamed (e.g. `@statecrafting/hiqlite-native`,
   `@statecrafting/kernel-native`) resolves the greatest published
   version within a caret of its current pin, so an unnamed companion
   never crosses its own major silently. `--to <version>` overrides the
   resolved target of the primary seed only (the one chassis package
   the contract's range drives); every other package still
   auto-resolves, so `--to` can never force a companion onto a version
   it never published. Write the per-package pins into package.json,
   then refresh the lockfile with `npm install --package-lock-only`
   (never a full platform-pruning install; same rule as enrahitu spec
   014 §3).
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
   fleet-wide upgrade sweep) can consume the result. Top-level
   `from`/`to` are the primary seed's headline versions; `pins[]`
   carries each package's own `from`/`to`, which can differ across
   packages under per-package resolution.

## 3. Acceptance

- Mock-fixture tests: a fake stamped tree with pinned old versions
  upgrades cleanly (pins bumped, lock refreshed, verify stub run);
  per-package resolution bumps a seed and its companion to their own
  independently-resolved targets; dirty-tree refusal; pre-018 tree
  refusal with the right message; dry-run mutates nothing;
  contract-range mismatch on a forced `--to` refuses with a "major
  upgrade requires the migration path" error.
- Live check: run the release binary against a stamped-app checkout
  that consumes the real published `@statecrafting/*` packages, hit
  the live npm registry to resolve targets, and confirm the verb
  upgrades the seed while leaving each companion at its own resolved
  version, verify verb green, documented in the commit message. (The
  packages are single-version-per-minor today, so a green in-range
  bump uses a stamped app whose `[requires]` range spans the published
  toolchain versions; forcing a `--to` outside the range still refuses
  as a migration.)
- ci.yml + spine gates green.

## 4. Out of scope

- Fleet-wide upgrades across many repos (platform/factory concern;
  it will call this verb's logic through the API or a shared crate,
  designed there).
- Building, packaging, or publishing chassis packages (enrahitu spec
  018 established the packaging; the `@statecrafting/*` packages are now
  owned by statecrafting specs 002/003; this verb only consumes
  published versions).
- Automatic PR creation and merge (deliberate: a human or the
  platform decides; v1 prints the suggestion).

## 5. Status (2026-07-23)

Implemented and live-checked: `statecraft template upgrade` in the
`statecraft_cli::verbs::template` module (the symbol this spec
establishes). It is the one *local* governed verb: unlike the spec 004
verbs it never calls the control plane. It reads `template.toml`,
resolves and bumps the chassis package pins in `package.json`,
refreshes the lockfile, runs the reserved codemod hook, runs the
contract's verify verb, and commits on a `template-upgrade/<from>-<to>`
branch, emitting the §2.6 `{from, to, pins, codemodsRun, verify}`
result inside the shared `{ok,data|error}` envelope.

**Model correction (2026-07-23).** The original v1 (2026-07-15) coupled
every chassis package to one target: the template and all `@enrahitu/*`
packages were assumed to ship at one version in lockstep, so the
resolved target was pinned to each discovered package. That assumption
is now falsified. The chassis packages moved from the retired
`@enrahitu` scope to `@statecrafting` (statecrafting spec 002 for the
toolchain, spec 003 for `hiqlite-native`) and are versioned
independently: `@statecrafting/toolchain` is published at
0.1.0/0.2.0/0.3.0 while `@statecrafting/hiqlite-native` and
`@statecrafting/kernel-native` are still 0.1.0. Under the old model a
run would have pinned the companions to the toolchain's 0.3.0, a
version they never published, and the lockfile refresh would have
failed. The verb's own §5 trade-off had flagged exactly this ("an
independently-versioned same-scope package would need [revisiting] if
enrahitu ever ships one"). Per the coherence guard the spec was amended
first (§1/§2/§3), then the code, rather than the reverse.

Current decisions (the per-package model):

- **Per-package resolution.** Each discovered chassis package resolves
  its own target. A seed (a package the contract names in `[requires]`,
  e.g. `@statecrafting/toolchain`) resolves the greatest published
  version its named range allows; a companion discovered by shared
  scope but unnamed (e.g. `@statecrafting/hiqlite-native`,
  `@statecrafting/kernel-native`) resolves the greatest published
  version within a caret of its current pin, so an unnamed companion
  never crosses its own major silently and is never pinned to a version
  it did not publish. `pins[]` carries each package's own `from`/`to`;
  the top-level `from`/`to` are the primary seed's headline versions.
- **`--to` overrides the primary seed only.** The one chassis package
  the contract's range drives. Every other package (secondary seeds and
  companions) still auto-resolves, so `--to` can never force a companion
  onto a nonexistent version. Documented limitation: with more than one
  named seed, `--to` targets the primary and the others auto-resolve.
- **Compat gate = the primary seed's `[requires]` range.** A forced
  `--to` outside that range is the "major upgrade requires the migration
  path" refusal (`incompatible_target`). Auto-resolved targets satisfy
  their own range by construction, so they need no gate.
- **Chassis discovery is scope-derived** (unchanged). Seeds are
  `package.json` deps whose unscoped name matches a `[requires]` key (so
  `node` drops out and `toolchain` resolves to `@statecrafting/toolchain`);
  the bump set is every exact-pinned dep sharing a seed's scope, with no
  hardcoded scope in the CLI, which is why the `@enrahitu`->`@statecrafting`
  rename cost the logic nothing (only doc comments and fixtures moved).
- **Refusal taxonomy** (all exit 1, JSON `error.kind`): `not_stamped`
  (no `template.toml`/`package.json`), `pre_chassis` (§1: no chassis
  packages), `local_chassis` (chassis present but as `file:` links, a
  template-development tree, not a stamped app), `bad_target`,
  `incompatible_target`, `dirty_tree`. Preflight order: structural
  refusals first, then the dirty-tree gate (before target resolution, so
  a dirty tree fails fast with no wasted registry read), then
  per-package resolution/compat. A verify failure is not a refusal: it
  is a completed run whose `{ok:true, data}` carries `verify:"fail"` and
  whose exit code (1) carries the failure, with the branch left for
  inspection, matching how `stamp status --watch` renders a terminal
  `failed` job.
- **Registry-shape robustness.** `npm view <pkg> versions --json`
  returns a JSON array, but for a package with exactly one published
  version some npm builds return a bare JSON string; the runner accepts
  both, so single-version companions resolve.
- **Dependency:** `semver` (pure Rust). Couples to spec 002's Cargo
  territory; the standing `Spec-Drift-Waiver` covers the crate-file
  edits. Cargo-flavored semver: a bare `[requires]` range (`"0.1.0"`)
  parses as caret, so contract authors write an explicit operator
  (`^0.3`, `>=24`), as enrahitu's contract does.

Covered by tests (all offline; every git/npm/node effect is behind a
`Runner` trait, which resolves per-package versions from a configured
published set via the same `select_latest` the real runner uses):
scope-derived discovery marking seed vs companion; the
`pre_chassis`/`local_chassis`/`not_stamped`/`incompatible_target`/`dirty_tree`
refusals; per-package pin rewrite to distinct targets; a companion
bumped on its own track independently of the forced primary; the
regression itself (a companion that never published the seed's target
stays on its own latest and is not rewritten to a nonexistent version);
the dry-run plan that mutates nothing; latest-resolution when `--to` is
omitted; the no-op; the happy path (branch -> lock -> verify -> commit);
the verify-failure path that leaves the branch uncommitted; and the
result envelope staying `{ok:true, data:{verify:"fail"}}` with a
`Rendered` exit 1. Four end-to-end binary checks drive the offline
paths (not-stamped, pre-018, dry-run) with the process working
directory set to a fixture.

**Live check (2026-07-23).** The release binary was run against a
stamped-app checkout that consumes the real published `@statecrafting/*`
packages (a git repo pinning toolchain/hiqlite-native/kernel-native at
0.1.0, contract `toolchain = ">=0.1, <0.4"` so an in-range target
exists, verify `true`). Against the live npm registry the verb resolved
per-package and upgraded cleanly: `@statecrafting/toolchain`
0.1.0 -> 0.3.0 while both companions stayed at their own 0.1.0; the real
`npm install --package-lock-only` resolved all three
(`toolchain@0.3.0`, `hiqlite-native@0.1.0`, `kernel-native@0.1.0`),
which the old lockstep model could not have produced; verify passed,
the bump committed on `template-upgrade/0.1.0-0.3.0`, exit 0. The
mirror case (a `^0.1` contract, `--to 0.3.0`) refused with
`incompatible_target`, exit 1. This satisfies §3; the verb is complete.
