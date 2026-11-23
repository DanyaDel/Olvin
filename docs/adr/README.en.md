# Architecture decisions (ADR)

<p align="center"><a href="README.md">Русский</a></p>

An ADR (Architecture Decision Record) is a short note about one decision: what was chosen, why, and what it costs. We write one when a decision is hard to undo or will be asked about again: the scripting language, a file format, a large dependency.

How the engine is built right now is in [ARCHITECTURE.en.md](../../ARCHITECTURE.en.md), the plans are in [ROADMAP.en.md](../../ROADMAP.en.md). An ADR answers a different question: why it was done this way and not another.

## Rules

- One file per decision: `NNNN-short-name.md`. Numbers go in order and are never reused. The English version sits next to it: `NNNN-short-name.en.md`.
- An accepted ADR is not rewritten. If the decision changes, a new ADR is written, and the old one gets the status "Superseded by ADR NNNN".
- Be honest: list the downsides of the chosen option and the upsides of the rejected ones. Don't write numbers nobody measured.
- Language and tone as in the rest of the docs: [STYLE.en.md](../../STYLE.en.md), section 10.

## Template

```markdown
# NNNN. Decision title

<p align="center"><a href="NNNN-name.md">Русский</a></p>

**Status:** Proposed | Accepted | Superseded by ADR NNNN — YYYY-MM-DD

## Context
The problem, and which engine constraints shape it.

## Decision
What was chosen and how it lives in the code (crate, feature, files).

## Considered options
Each option: pros, cons, why not this one.

## Consequences
What got easier, what got harder, which questions are still open.
```

## Index

| # | Decision | Status |
|---|---|---|
| [0001](0001-lua-scripting.en.md) | Scripting in Lua 5.4 (mlua) | Accepted, 2026-11-19 |
| [0002](0002-build-security.en.md) | Build, supply-chain and binary hardening | Accepted, 2026-11-24 |
| [0003](0003-retro-render-style.en.md) | Retro (PS1) render style in the main mesh shader | Accepted, 2026-11-24 |
| [0003](0003-netcode.en.md) | Netcode: own protocol on QUIC, server authority, snapshot interpolation | Accepted, 2026-11-23 |
