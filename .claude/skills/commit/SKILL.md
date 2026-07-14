---
name: commit
description: Create a git commit with an impact-focused conventional commit message referencing the owning spec id.
allowed-tools: Bash
---

# Commit

Create a git commit following these steps.

## 1. Survey the changes

```
git status
git diff --cached
git diff
git log --oneline -5
```

Identify what is staged vs unstaged, the nature of each change (feature,
fix, refactor, docs, test, chore), and the user-visible impact. Match the
scoping conventions visible in recent history.

## 2. Refresh the derived shards

If the diff touches any hashed input (`specs/*/spec.md`, `spec-spine.toml`,
`Cargo.toml`, `standards/**`, `.github/workflows/**`, `.claude/**`), run:

```
spec-spine compile && spec-spine index
```

and include the regenerated `.derived/` shards in the same commit; they are
committed artifacts here. Then confirm the gates before committing:

```
spec-spine lint --fail-on-warn && spec-spine index check
```

Once spec 002 lands and code exists, also run the cargo gates for Rust
changes: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`. Halt on any failure; do not commit over a red gate.

## 3. Draft a conventional-commit message

Format: `type(scope): subject`

**Type (required):** `feat`, `fix`, `refactor`, `docs`, `test`, `chore`.
When the work belongs to a numbered spec, the spec id is the scope, per
the AGENTS.md backlog protocol: `feat(002): ...`, `fix(004): ...`.
Otherwise use an area scope when it clarifies, e.g. `docs(standards):`,
`chore(claude):`.

**Subject line:**
- 72 characters maximum (hard limit; count them).
- Lead with the impact or problem solved, not the technique used.
- No trailing period. No emojis.

**Good vs bad:**
- BAD: `refactor: extract helper for envelope rendering`
- GOOD: `fix(004): fleet remove rejects a mismatched --confirm name`
- BAD: `feat: add new subcommand handler`
- GOOD: `feat(002): clap command tree with honest not-implemented stubs`

**Body (optional):** separate from the subject with a blank line. Use
dash-prefixed bullets only for multiple distinct changes. Keep lines
under 72 characters. Explain how only when it is non-obvious; the subject
already covers what and why.

**Issue linking:** `Fixes #NNN` or `Closes #NNN` on its own line after
the body, when applicable.

## 4. Stage the relevant files

Use `git add` with specific paths. Do not use `git add -A` or `git add .`
unless every changed file belongs in this commit. Regenerated `.derived/`
shards belong with the change that dirtied them. Never stage files that
look like secrets (`.env`, credentials, tokens).

## 5. Create the commit

Pass the message via heredoc:

```
git commit -m "$(cat <<'EOF'
type(scope): subject line here

Optional body with details.
EOF
)"
```

## 6. Verify

Run `git status` to confirm the commit succeeded and the tree is in the
expected state.

## Banned content

- No `Co-Authored-By` or any AI/Claude attribution line.
- No marketing taglines, links, or promotional text.
- No emojis anywhere in the message.
- No padding about what was not changed. Be direct and factual.

$ARGUMENTS
