---
id: '0095'
title: Verify emits suggestions only when deterministic, null when judgment is required
status: ACCEPTED
date: 2026-03-27
triggered_by: Verify violations should propose fixes without ever proposing a wrong one
loop: planning
---

# ADR-0095: Verify emits suggestions only when deterministic, null when judgment is required

## Context

Released in v1.1.0 (Repository DNA). When verify flags a convention deviation, some deviations have a single obvious mechanical fix — a wrong-case rename, an `unwrap()`-to-`?` rewrite, a relative-to-absolute import rewrite. For these, a deterministic rule can produce the exact corrected string. Other deviations require judgment a deterministic rule cannot supply.

The principle is that verify must never emit a confidently-wrong suggestion. The shipped `verify_changes` dispatches three checks — `check_naming`, `check_errors`, `check_imports`. Some deviation classes discussed during design (a public symbol where the trend is private, an architecture layering violation) remained aspirational: verify ships no visibility or architecture check, so those violations are never produced and there is no null-suggestion path for them in the implementation.

## Options considered

- **Option A — Suggest only for deterministic cases, null otherwise (chosen):** Generate a concrete fix string only where the correction is mechanically derivable; set `suggestion` to `null` where no mechanical mapping exists. Pros: never emits a wrong suggestion; honest about its limits. Cons: some violations carry no actionable fix string. Someone could prefer this because a wrong suggestion is worse than no suggestion.
- **Option B — Always attempt a suggestion:** A reasonable alternative would have been to heuristically guess a fix for every violation, including judgment cases. Pros: every violation gets an actionable hint. Cons: risks confidently-wrong suggestions where judgment is required, which erodes trust in the tool. Someone could prefer it if every flag must come with a next action.

## Decision

Generate a suggestion only where the fix is deterministically observable. The three shipped deterministic cases are:

- Function-naming wrong-case — emit `Rename to ` followed by the corrected snake/camel form.
- `.unwrap()` in production code — emit a replace-with-`?` / `.map_err(...)?` / `.expect(...)` mapping.
- Relative import where the convention is absolute — emit a rewrite-as-absolute suggestion.

Type-naming deviations also ship, but with `suggestion: null`, because there is no mechanical type-name mapping. The design-doc cases of a missing doc-comment stub, public-where-trend-is-private, and architecture violations were not implemented: `verify.rs` dispatches only `check_naming`, `check_errors`, and `check_imports`, so it produces no doc-comment, visibility, or architecture violations.

## Consequences

### Positive
- No confidently-wrong fix suggestions.
- Clear contract: a `null` suggestion signals "human/LLM judgment needed" (as for type-naming deviations).

### Neutral
- Visibility and architecture violations are out of verify's scope entirely; they appear only in the design-doc table, not in the shipped checks.

## Revisit if
- A reliable heuristic for judgment-class suggestions emerges.
- Visibility or architecture checks are added to verify, introducing new null-suggestion paths.

---

Source note: the v1.1.0 design doc records the principle as "Deterministic suggestions only | null when judgment needed | Never generate a wrong suggestion; honest about limits".
