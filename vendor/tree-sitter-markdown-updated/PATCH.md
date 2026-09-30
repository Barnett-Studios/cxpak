# Local changes to `tree-sitter-markdown-updated` 0.1.0

Vendored from crates.io (`tree-sitter-markdown-updated 0.1.0`, sha256
`650c95d396001cd3f81129ee0cea805a593eafcf016e177f221baab7bc7e7616`) and patched.
Upstream is <https://github.com/c-gamble/tree-sitter-markdown>, a fork of
`ikatyang/tree-sitter-markdown` whose `master` carries the identical
`scanner.cc`. **Issues are disabled on that repository and 0.1.0 is its only
crates.io release**, so there is nowhere to send this.

Wired in through `[patch.crates-io]` in the workspace `Cargo.toml`.

## The fault

`cxpak serve` exited at startup with

```
terminate called after throwing an instance of 'int'
```

and exit code 139. One markdown file was enough, and nothing named it. Under
`Restart=on-failure` the unit restarted 6,429 times before anyone looked.

`Scanner::serialize` wrote the parse state into tree-sitter's serialization
buffer and tested that it fit **afterwards**:

```c
i += lxr_.serialize(&buffer[i]);
i += min_inl_dlms_.serialize(&buffer[i]);
i += blk_dlms_.serialize(&buffer[i]);
i += blk_ctx_stk_.serialize(&buffer[i]);
buffer[i++] = has_opt_wsp_ind_;

TREE_SITTER_MARKDOWN_ASSERT(i <= TREE_SITTER_SERIALIZATION_BUFFER_SIZE);  // too late
```

That buffer is 1024 bytes and three of the four components grow with the
document, so the check ran on a buffer that had already been overrun. **This is
a buffer overflow, and the input controls how far it goes.**

`bindings/rust/build.rs` defines `TREE_SITTER_MARKDOWN_AVOID_CRASH`, which makes
`TREE_SITTER_MARKDOWN_ASSERT` throw. The scanner is `extern "C"`, so the
exception could not reach the Rust host: `std::terminate`. The guard that
catches it existed, but only on `scan` — `serialize` and `deserialize` had none.

A second fault sat in the same functions: each list writes its own length into
**one byte**, so a list above 255 entries wrapped and `deserialize` read it back
at the wrong length — a silently wrong parse rather than a crash.

Measured on this grammar, with the state sizes the scanner actually reaches:

| input | `min_inl_dlms_` | `blk_dlms_` | `blk_ctx_stk_` | serialized |
|---|---|---|---|---|
| 235 real `.md` files in this repo (peak) | 44 | 8 | 7 | 101 B |
| 56 levels of nested list | 0 | 2 | 168 | 1015 B |
| 57 levels of nested list | 0 | 171 | 171 | **1036 B** |
| 150 lines of link text on one line | **305** | 149 | 1 | **1070 B** |

## The repair

**The bound is enforced where the state is built, not where it is written.**
Each container refuses to grow once the state would no longer serialize, so
`serialize` can never be handed more than it has room for. Nothing is truncated
on the way out — a state that exists is a state that serializes — which is what
keeps `serialize` and `deserialize` in agreement.

Truncating on the way out instead *looks* like the obvious fix and does not
work: the lexer position lives in the same state, so returning a short or empty
buffer sends the scanner back to the same offset to rebuild the same oversized
state for ever. A crash becomes a non-terminating parse, which for a daemon
under `Restart=on-failure` is worse — it never fails, so it never restarts.

**The budget is shared between the three lists, not split between them.** A
split is the tempting version and it is wrong: the peaks above do not coincide,
so any fixed partition either wastes the budget one document shape needs or
shrinks the range of documents that parse correctly today. A fixed split was
tried first here and regressed 56-level nesting, which the released grammar
parses correctly.

### Files

- `src/tree_sitter_markdown/serialization_limit.h` *(new)* — the budget, the
  per-entry widths, the 255-entry count ceiling, and `srl_budget_has_room()`.
- `src/tree_sitter_markdown/serialization_budget.cc` *(new)* — `SerializationBudget`,
  which holds the three lists so any one of them can ask what the other two are
  using. It reads their live `serialized_size()` (each O(1)) rather than keeping
  a running total that every pop, erase, clear and resize would have to adjust.
- `block_context.{h,cc}`, `block_delimiter.{h,cc}`, `inline_delimiter.{h,cc}` —
  `serialized_size()`, `size()`, `is_full()`, and a budget-aware `push_back` /
  `push` / `insert`. `BlockContextStack::push` now returns whether it pushed.
- `lexer.{h,cc}` — `serialized_size()`.
- `scanner.cc` — the bounds test moved ahead of the writes; the count ceilings
  asserted; the two block-opening sites made to refuse coherently; `serialize`
  and `deserialize` guarded like `scan` already was.
- `inline_scan.cc` — see below.
- `bindings/rust/build.rs` — watches every file under
  `src/tree_sitter_markdown/` (`scanner.cc` `#include`s them all, and watching
  only `scanner.cc` left edits to the rest silently uncompiled), and honours
  `TREE_SITTER_MARKDOWN_REPORT_SIZE=1` to print the state size per token, which
  is how the table above was measured.

### The one place a refusal was not safe

`inline_scan.cc` assumed `scn_eol` always appends:

```c
} else if (blk_dlms.back().sym() == SYM_LIT_LBK) {
  lxr.jmp_pos(blk_dlms.back().end_pos());
```

With nothing appended, `back()` is a delimiter from an **earlier row**, and
jumping to its end position moves the lexer backwards — the same line break for
ever. The guard is now that the list actually grew; otherwise the inline span is
abandoned and read as literal text, the escape this branch already takes for a
line break it cannot use. (As written it was also an unchecked `back()` on a
possibly empty list.)

## What changes for a document

Nothing, until a document needs more than 1024 bytes of scanner state. All 235
markdown files in this repository produce **byte-identical parse trees**, peaking
at 101 bytes of state — an order of magnitude below the budget.

Past the budget, structure that cannot be represented is dropped rather than
recorded: nesting beyond the ceiling parses at the depth already reached, and
inline delimiters beyond it are read as literal text. The parse terminates and
the tree is well-formed.

The documents that used to abort now parse:

| input | before | after |
|---|---|---|
| 56 levels of nesting | ok | ok, identical tree |
| 57, 58 levels | **abort (exit 139)** | ok |
| 100 – 2,000 levels | **abort** | ok, degraded, 16 ms |
| 100 lines of link text | ok | ok, identical tree |
| 500 – 5,000 lines of link text | **abort** | ok, 23 – 113 ms |
| 35 KB mixed document | ok | ok, 91 ms |
