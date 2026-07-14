---
name: code-review
description: "Review the current diff for correctness bugs, guard erosion, and spec drift, then emit an evidence-oriented findings list"
allowed-tools: Read, Grep, Glob, Bash(git status:*), Bash(git diff:*), Bash(git log:*), Bash(git show:*), Bash(git rev-parse:*), Bash(spec-spine:*), Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*)
argument-hint: "[scope] - e.g. \"branch\", \"working tree\", \"src/\""
---

# /code-review: correctness + spec drift

Reviews the current diff against two questions: does the change have
correctness or edge-case bugs, and does it still match its owning
spec's contract. Output is an evidence-oriented findings list, each
line citing `file:line`. Read-only: no files are modified unless the
user asks for a fix afterward.

## Step 0: scope the diff

```sh
git status --short && git diff --stat && git log --oneline -10
git diff origin/main...HEAD --stat   # committed delta
git diff HEAD --stat                 # uncommitted delta
```

Note which classes changed: Rust source (`src/**/*.rs`, `Cargo.toml`,
`Cargo.lock`), specs (`specs/**/spec.md`), schemas/standards
(`standards/**`), docs (`*.md`), workflows (`.github/**`), kit
(`.claude/**`).

## Step 1: corpus stays green

The change must not leave the spine red. Run the gate chain and capture
the exact outputs as evidence:

```sh
spec-spine compile
spec-spine lint --fail-on-warn       # corpus well-formedness
spec-spine index check               # staleness (exit 2 if stale)
spec-spine couple --base origin/main --head HEAD   # drift gate (exit 1 on drift)
```

Once spec 002 lands and code exists, add the cargo gates for any Rust
delta:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- A `couple` failure is the headline finding: cite the file the gate
  named and the owning spec whose declared edges fail to cover it.
- A `lint` or `index check` failure is a corpus finding: cite the
  diagnostic verbatim.
- A cargo gate failure is a build finding: cite the first error verbatim.

## Step 2: spec-contract match

For each changed source file, confirm the change is consistent with the
contract of its owning spec rather than only with the gate's mechanical
pass. Useful reads (governed, via the CLI, not ad-hoc JSON parsing):

```sh
spec-spine registry show <spec-id>           # the owning spec's declared surface
spec-spine registry relationships <spec-id>  # its typed edges
```

Flag drift where code does something the spec's narrative or owned
authority units do not describe, even if `couple` happens to pass
(e.g. the edge is over-broad). Cite the spec section and the `file:line`.

## Step 3: guards and envelopes

The product-surface guards are design, not friction (CLAUDE.md Key
Conventions). Any weakening is a critical finding:

- Stamps keep the required `--posture` flag: no default, no env
  fallback.
- Fleet remove keeps `--confirm <name>`: no `--yes`/`--force` shortcut.
- No bypass flags ever, whatever the framing.
- `--output json` envelopes are versioned API reused by the MCP face:
  flag any changed field name, type, or nesting in an existing envelope
  that its owning spec does not describe.
- The CLI never bypasses the platform: no local stamping, no direct
  kubeconfig access.

## Step 4: correctness pass

Read the changed source and look for each of the following, with a
`file:line` and a one-sentence evidence claim:

- Logic and edge-case bugs (off-by-one, unhandled `None`/`Err`, empty
  input, boundary values).
- Exit-code discipline: 0 ok, 1 operational failure, 2 usage /
  not-implemented; errors print to stderr, never stdout.
- Determinism hazards: unsorted map/set iteration, locale- or
  platform-dependent behavior, unstable ordering in emitted JSON.
- Hygiene: stray debug prints, commented-out code, dead branches.

## Step 5: findings report

```
## Review: <scope>
Base: origin/main | Head: <branch> | Files: <n> | +<a>/-<d>
Gate: compile <ok|FAIL> | lint <ok|FAIL> | index check <ok|stale> | couple <ok|drift>
Cargo: fmt <ok|FAIL|n/a> | clippy <ok|FAIL|n/a> | test <ok|FAIL|n/a>

### Findings (severity-ordered)
- [CORRECTNESS|SPEC-DRIFT|GUARD|GATE|HYGIENE] <claim> at `file:line`
  Evidence: <one sentence, cited>
  Fix: <specific recommendation>

### Clean
- <dimensions checked with nothing found>
```

If nothing is found, say so plainly and report the gate result as the
evidence. To proceed with fixes, the user names the findings to apply.
