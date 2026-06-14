# Architecture Decision Records

This directory records the architecturally significant decisions behind cxpak — what was chosen, what was considered and rejected, and the conditions under which each choice should be revisited.

## Format

Each ADR uses [`template.md`](./template.md): YAML frontmatter (`id`, `title`, `status`, `date`, `triggered_by`, `loop`) followed by **Context → Options considered → Decision → Consequences → Revisit if**. The *Options considered* and *Revisit if* sections are load-bearing: they are what make a decision auditable as it ages.

## Retroactive backfill

ADRs 0001–NNNN were reconstructed retroactively (June 2026) from the project's internal design and implementation documents (`docs/plans/`, kept local) and from the shipped code across releases v0.4.0 → v2.2.1. They carry `status: ACCEPTED` with the `date` set to when the decision actually landed, and each cites the design document or code that originated it. Where a decision was later changed or superseded, that is recorded via `SUPERSEDED by ADR-MMMM`.

From this point forward, ADRs are written at decision time, not backfilled.

## Index

See [`INDEX.md`](./INDEX.md) for the full numbered list.

## Creating a new ADR

1. `cp template.md 00XX-title.md`
2. Fill in context, options considered (with honest rejected-option reasoning), decision, consequences, and revisit-if conditions
3. Set status to `PROPOSED`
4. Submit for review; set to `ACCEPTED` when the decision is locked
