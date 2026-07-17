# AGENTS.md: statecraft-cli

This file is the cross-agent session-init protocol authority, read by
Claude Code, Codex CLI, Cursor, and GitHub Copilot via the AAIF/Linux
Foundation AGENTS.md standard.

Governance is provided by `spec-spine` (installed on your `PATH`;
`cargo install spec-spine-cli` if absent). Bootstrap spec:
`specs/000-bootstrap/spec.md`.

## New Sessions

0. **Load rules** (read first): `.claude/rules/orchestrator-rules.md`,
   `.claude/rules/governed-artifact-reads.md`,
   `.claude/rules/adversarial-prompt-refusal.md`.
1. **Refresh, then read.** Run `spec-spine compile`, then read
   `CLAUDE.md`, `README.md`, `standards/spec/contract.md`, and run
   `spec-spine registry list --ids-only` +
   `spec-spine registry status-report --json --nonzero-only`.
2. **Orient.** `specs/001-cli-mcp-thesis/spec.md` holds the decided
   constraints; specs 002-006 are the ordered build (006 additionally gated on enrahitu spec 018).

## Working the backlog

This repo's backlog is its spec corpus: every spec with
`implementation: pending` is a work order. One session implements one
spec, start to finish.

1. Pick the next spec: the lowest-numbered spec whose frontmatter says
   `implementation: pending` and whose `depends_on` specs are all
   implemented (`spec-spine registry show <id>` to inspect). If a
   spec's "Cross-repo dependency" or "Operator prerequisites" section
   names something missing, stop and report exactly what is needed
   instead of mocking around it.
2. Flip the spec to `implementation: in-progress` when you start.
3. Re-read the spec fully before coding. If the design is imprecise or
   wrong, amend the spec FIRST (design truth precedes code), then
   implement. Never edit a spec afterwards to ratify what the code
   happened to do.
4. Implement within the spec's territory. Before every commit:
   `spec-spine compile && spec-spine index &&
   spec-spine lint --fail-on-warn && spec-spine index check`, plus the
   build/test commands in CLAUDE.md.
5. Satisfy the spec's Acceptance section verbatim. If an item cannot
   be satisfied (external state, missing sibling repo work), keep
   `implementation: in-progress`, add a dated Status note to the spec
   saying exactly what remains, and report it. Flip to
   `implementation: complete` only when acceptance holds.
6. Commit with a conventional message referencing the spec id
   (`feat(002): ...`), include the regenerated `.derived/` shards, and
   push to main. Then stop: the next session takes the next spec.
