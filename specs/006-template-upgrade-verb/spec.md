---
id: "006-template-upgrade-verb"
title: "stagecraft template upgrade: chassis upgrades as a governed verb"
status: approved
created: "2026-07-14"
implementation: pending
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
