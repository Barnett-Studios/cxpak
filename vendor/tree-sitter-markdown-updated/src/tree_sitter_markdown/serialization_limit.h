#ifndef TREE_SITTER_MARKDOWN_SERIALIZATION_LIMIT_H_
#define TREE_SITTER_MARKDOWN_SERIALIZATION_LIMIT_H_

#include <tree_sitter/parser.h>
#include <stddef.h>
#include "./shared_type.h"

// tree-sitter hands the external scanner one fixed buffer of
// TREE_SITTER_SERIALIZATION_BUFFER_SIZE (1024) bytes.  Every piece of state the
// scanner carries across a token boundary has to fit inside it.
//
// The released grammar wrote the state into that buffer and tested the bound
// *afterwards*, so a document that produced more state than the budget had
// already overrun the buffer by the time the assert fired.  With
// TREE_SITTER_MARKDOWN_AVOID_CRASH that assert is `throw 1`, and since the
// scanner is `extern "C"` the exception cannot cross back into the host: the
// process dies in std::terminate.
//
// Here the bound is a property of the containers instead.  Each refuses to grow
// once the state would no longer fit, so `serialize` can never be handed more
// than it has room for.  Nothing is truncated on the way out -- a state that
// exists is a state that serializes -- which is what keeps `serialize` and
// `deserialize` in agreement.  (Truncating on the way out instead is what turns
// this crash into a non-terminating parse: the lexer position lives in the same
// state, so dropping it returns the scanner to the same offset to rebuild the
// same oversized state for ever.)
//
// The budget is SHARED across the three lists rather than split between them.
// A split is tempting and wrong: measured on this grammar, 56 levels of nested
// list need ~168 block contexts while holding two block delimiters and no
// inline delimiters, whereas a long line of link text needs ~300 inline
// delimiters at one block context.  The peaks do not coincide, so any fixed
// partition either wastes the budget one shape needs or shrinks the range of
// documents that parse correctly today.

namespace tree_sitter_markdown {

// Per-entry encoded widths, fixed by the serialize methods of each element.
#define MIN_INL_DLM_SRL_SIZE 2  // MinimizedInlineDelimiter::serialize
#define BLK_DLM_SRL_SIZE     3  // BlockDelimiter::serialize
#define BLK_CTX_SRL_SIZE     3  // BlockContext::serialize

// Fixed overhead: the Lexer's two scalars and the has_opt_wsp_ind_ flag.  The
// one count byte each list writes is counted by that list's serialized_size().
#define SRL_FIXED_OVERHEAD (sizeof(LexedColumn) + sizeof(LexedCharacter) + 1)

// What the three lists may occupy between them: 1024 - 7 = 1017 bytes.
#define SRL_ENTRY_BUDGET \
  (TREE_SITTER_SERIALIZATION_BUFFER_SIZE - SRL_FIXED_OVERHEAD)

// Each list writes its length into a single byte, so no list may hold more than
// 255 entries however many bytes remain.  Past that the count wraps and
// deserialize reads the list back at the wrong length -- a silently wrong parse
// rather than a crash, which is the worse of the two.
#define SRL_MAX_COUNT_PER_LIST 255

// Holds the three lists that make up the serialized state, so that any one of
// them can ask what the other two are already using.  Defined in
// serialization_budget.cc, once the list types are complete; the containers
// only ever hold a pointer to one and call srl_budget_used() on it.
//
// A null budget means "not part of the serialized state, so not limited".  The
// staging lists in block_scan.cc are temporaries that are transferred into the
// scanner's own lists before anything is serialized, and it is the destination
// that has to enforce the bound.
struct SerializationBudget;
unsigned srl_budget_used(const SerializationBudget *budget);

// True when `bytes` more can be afforded on top of what the three lists hold.
inline bool srl_budget_has_room(const SerializationBudget *budget, unsigned bytes) {
  if (budget == NULL_PTR) return true;
  return srl_budget_used(budget) + bytes <= SRL_ENTRY_BUDGET;
}

}

#endif // TREE_SITTER_MARKDOWN_SERIALIZATION_LIMIT_H_
