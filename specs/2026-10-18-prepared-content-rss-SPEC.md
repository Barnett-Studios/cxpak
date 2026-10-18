---
title: cxpak 3.5.2 — scenario C max RSS after the in-place republish (prepared-content lever)
status: DRAFT
created: 2026-10-18
updated: 2026-10-18
target: cxpak 3.5.2 (PATCH — no public API change)
adrs: 0215
spec: specs/2026-10-18-peak-rss-regression-SPEC.md
---

# Prepared-content RSS — Spec

## 1. Problem (measured)

`benches/BASELINES.md` "3.5.1 peak RSS, round 2" (`/usr/bin/time -l`, scriptorium, n=5 interleaved):

| Scenario | Binary | peak memory footprint | maximum resident set size |
|---|---|---|---|
| U | 3.4.0 / 3.5.1 | 422.8 / 372.8 | 395.3 / 384.9 |
| C | 3.4.0 / 3.5.1 | 692.1 / 647.7 | 458.4 / **569.5** |

Footprint meets the gate (<= 3.4.0 x 1.10). Max RSS in scenario C does not: 569.5 MB against 504.2 MB
(+24% over 3.4.0). Scenario U meets it. C minus U is about 185 MB, so the excess is not the build
(shared by U) but what C adds: HEAD moved, the derived cache hits on content, conventions are
`StalePending` and are re-mined in the background (ADR-0212/0213).

## 2. Where file content is held (evidence)

1. `src/commands/serve.rs:483-484` — `settled.into_parts()` yields the file `String`; sha256 computed.
2. `serve.rs:543` — moved (not copied) into `Prepared.file.content` (`struct Prepared`, `serve.rs:370`;
   `ArtifactFile.content`, `src/index/mod.rs:303`). Every file's text is resident in the
   `Vec<Option<Prepared>>` collected at `serve.rs:~548` until the fold at `serve.rs:563-579`.
3. `src/index/mod.rs:692` — `from_artifacts_with_call_sites` moves it again into
   `IndexedFile.content`. From here it is owned by the published index for the process lifetime.
4. `IndexedFile.content` is a `pub` field of a `pub` struct and is read after the build by tools
   (`serve.rs:2101-2106`, `4333`, `4416-4421`), `auto_context/mod.rs:163-192`, `lsp/backend.rs:383`,
   `plugin/manifest.rs:64`, `commands/hook.rs:305`, and by the convention miners
   (`conventions/imports.rs`, `conventions/testing.rs`) plus `intelligence/*`.

Consequence: content is **moved, never duplicated**, between steps 1-3. Removing it from `Prepared`
cannot lower the peak: the same bytes would simply be owned by `ArtifactFile` or `IndexedFile`
earlier. Dropping it from the *index* is a public field/behaviour change and is out of scope. So the
ADR-0213 "drop `content` from `Prepared`" fallback, as worded, is expected to be a no-op. The
duplicated, transient state during build is elsewhere: `Prepared.fresh` (`ArtifactEntry` cloning
`parse_result`, term frequencies and call sites, `serve.rs:~452-475`) and the `changed` map that
`PendingStore` carries to the store write (`serve.rs:564-573`), alongside the same data in
`FileArtifacts`.

## 3. Unknown to resolve first

Which phase sets C's max RSS is not attributed. U and C share the build; C adds the background mining
and republish. Phase-tagged RSS sampling is the first deliverable; levers are chosen from that.

## 4. Acceptance gate

- Scenario C max RSS **median <= 504 MB** (n=5, interleaved, vs released 3.5.1; 3.4.0 if available).
- Peak memory footprint not worse than 3.5.1 (C 647.7, U 372.8; medians).
- `index_ready_s` not worse than +10% vs 3.5.1.
- Tool outputs byte-identical; existing determinism tests green.
- No change to `pub mod cache`, `pub mod core_graph`, `pub mod index` public items.
- If no lever reaches 504 MB, report the measured best and the unmet gate; do not claim it.
