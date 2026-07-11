---
id: '0198'
title: Risk normalization = within-repo percentile; one-signal-per-channel encoding
status: ACCEPTED
date: 2026-07-11
triggered_by: cxpak UI overhaul (3.1.0) — visual surface redesign
loop: planning
---

# ADR-0198: Risk normalization = within-repo percentile; one-signal-per-channel encoding

**Context.** The risk treemap renders uniformly teal. Root cause: `risk = norm_churn × norm_blast × tc_term` (`risk.rs:105`) is a product of sub-1 fractions (`norm_blast = dependents/total_files` → e.g. 60/761 = 0.079, `risk.rs:88`), so the max observed risk is ~0.04, and the ramp `d3.scaleLinear().domain([0,0.4,0.7,1.0])` (`render.rs:620`) maps everything into the first band. (The earlier "risk 6.88 out of range" was a misread of `6.886e-6` — in range; the real bug is scale collapse, not overflow. There is even an opacity kludge at `render.rs:635` compensating for the tiny range.)

**Options considered.**
1. *Rescale the ramp domain to `[0,0.02,0.04]`.* Rejected — brittle, repo-specific, still hides within-repo structure and breaks on the next repo.
2. *Color by within-repo percentile; keep the raw score as "absolute" in the tooltip.*

**Decision.** Option 2. (a) Never emit a bare mantissa. (b) Color the treemap/table by `risk_percentile = rank(risk)/N` — reusing the already-deterministic `compute_risk_ranking` (path tie-break, `risk.rs:121-126`); note `norm_churn` is *already* a percentile (`risk.rs:66-72`), so this extends an established pattern to the one non-percentile term. (c) Fix the large-repo blast penalty (`dependents/total_files` shrinks as the repo grows → use `blast/max_blast` or a percentile). (d) Data-driven quantile legend, not hardcoded 0.4/0.7. Plus the encoding discipline: one signal per channel per view (SIZE=magnitude never risk; POSITION=topology; COLOR=one quantitative signal; SHAPE/GLYPH=categorical flags), luminance-monotonic colorblind-safe ramp, and a **grayscale-survival assertion** shipped alongside `spa_a11y.rs`.

**Consequences.** The treemap becomes readable and discriminating on any repo size. Percentile coloring is determinism-safe (ranking already is). Tooltip carries both percentile and absolute, labeled — feeds the prove-it drawer (ADR-0193). Trade-off: percentile hides absolute magnitude at a glance (mitigated by the tooltip + the "absolute" label).

**Revisit if.** Users need cross-repo risk comparison (percentile is within-repo only) → add an absolute-scaled secondary view with a calibrated global ramp.
