---
id: "000-bootstrap"
title: "Bootstrap spec system"
status: approved
created: "2026-07-14"
summary: >
  Foundational contract: authored truth lives only in markdown (+ YAML
  frontmatter); machine-consumable truth is compiler-emitted JSON only;
  every artifact is a deterministic function of (config, file contents);
  a typed authority graph governs who-owns-what. This repository is born
  governed: the spine exists before the first line of product code.
establishes:
  - "spec-spine.toml"
  - ".github/workflows/spec-spine.yml"
unamendable:
  - "markdown-truth-boundary"
  - "json-truth-boundary"
  - "determinism-requirement"
  - "typed-authority-graph"
  - "refusal-rule"
---

# 000: Bootstrap spec system

This is the spec that defines what a spec *is*. Ordinary specs live under
`specs/`. Each compilation unit links back here (or to a more specific
spec) via `[package.metadata.spec-spine].spec` in its manifest, a
`// Spec:` comment header, or a spec's ownership edge.

## 1. The authoring / derived boundary

Humans author markdown; the compiler owns the JSON. Never hand-edit a
derived artifact.

## 2. The typed authority graph

Specs declare typed edges (`establishes`, `extends`, `refines`,
`supersedes`, `amends`, `co_authority`, `constrains`, `references`) and
the units they own (file / section / symbol / directory / crate / module).
Authority is derived by walking the graph.
