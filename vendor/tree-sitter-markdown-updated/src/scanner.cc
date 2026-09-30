#include <tree_sitter/parser.h>
#include <cassert>
#include <cstdio>
#include <list>
#include <vector>

#include "./tree_sitter_markdown/token_type.h"

#ifdef TREE_SITTER_MARKDOWN_AVOID_CRASH
#define TREE_SITTER_MARKDOWN_ASSERT(condition) if (!(condition)) throw 1;
#else
#define TREE_SITTER_MARKDOWN_ASSERT(condition) assert(condition)
#endif

// tree-sitter does not support multiple files for external scanner
#include "./tree_sitter_markdown/block_context.cc"
#include "./tree_sitter_markdown/block_delimiter.cc"
#include "./tree_sitter_markdown/block_scan.cc"
#include "./tree_sitter_markdown/block_scan_util.generated.cc"
#include "./tree_sitter_markdown/inline_delimiter.cc"
#include "./tree_sitter_markdown/inline_context.cc"
#include "./tree_sitter_markdown/inline_scan.cc"
#include "./tree_sitter_markdown/inline_scan_util.generated.cc"
#include "./tree_sitter_markdown/lexer.cc"
#include "./tree_sitter_markdown/parse_table.cc"
#include "./tree_sitter_markdown/predicate.cc"
#include "./tree_sitter_markdown/predicate_util.generated.cc"
#include "./tree_sitter_markdown/serialization_budget.cc"
#include "./tree_sitter_markdown/shared_type.cc"
#include "./tree_sitter_markdown/util.cc"

namespace {

using namespace tree_sitter_markdown;

struct Scanner {
  Lexer lxr_;
  MinimizedInlineDelimiterList min_inl_dlms_;
  InlineDelimiterList inl_dlms_;
  InlineContextStack inl_ctx_stk_;
  BlockDelimiterList blk_dlms_;
  BlockContextStack blk_ctx_stk_;
  bool has_opt_wsp_ind_;

  // Lets each of the three serialized lists see what the other two are using,
  // so the 1024-byte budget is shared between them rather than partitioned.
  // Declared after them so it is constructed once they exist.
  SerializationBudget bgt_;

  Scanner(): bgt_(&min_inl_dlms_, &blk_dlms_, &blk_ctx_stk_) {
    min_inl_dlms_.set_budget(&bgt_);
    blk_dlms_.set_budget(&bgt_);
    blk_ctx_stk_.set_budget(&bgt_);
    deserialize(NULL, 0);
  }

  unsigned serialized_size() const {
    return lxr_.serialized_size()
         + min_inl_dlms_.serialized_size()
         + blk_dlms_.serialized_size()
         + blk_ctx_stk_.serialized_size()
         + 1;  // has_opt_wsp_ind_
  }

  unsigned serialize(unsigned char *buffer) {
    size_t i = 0;
#ifdef TREE_SITTER_MARKDOWN_REPORT_SIZE
    fprintf(stderr, "SRLSIZE total=%u inl=%zu blkdlm=%zu blkctx=%zu\n",
            serialized_size(), min_inl_dlms_.size(), blk_dlms_.size(), blk_ctx_stk_.size());
#endif

    // The bound is tested BEFORE the writes.  Testing it afterwards -- which is
    // what the released grammar did -- reads a buffer that has already been
    // overrun.  The container capacities make this assert unreachable; it stays
    // as the statement of the invariant they exist to keep.
    TREE_SITTER_MARKDOWN_ASSERT(serialized_size() <= TREE_SITTER_SERIALIZATION_BUFFER_SIZE);

    // Each list writes its length into a single byte, so a list longer than 255
    // would wrap the count and be read back at the wrong length -- a silently
    // wrong parse rather than a crash.  The capacities keep every list under it.
    TREE_SITTER_MARKDOWN_ASSERT(min_inl_dlms_.size() <= SRL_MAX_COUNT_PER_LIST);
    TREE_SITTER_MARKDOWN_ASSERT(blk_dlms_.size() <= SRL_MAX_COUNT_PER_LIST);
    TREE_SITTER_MARKDOWN_ASSERT(blk_ctx_stk_.size() <= SRL_MAX_COUNT_PER_LIST);

    i += lxr_.serialize(&buffer[i]);
    i += min_inl_dlms_.serialize(&buffer[i]);
    i += blk_dlms_.serialize(&buffer[i]);
    i += blk_ctx_stk_.serialize(&buffer[i]);
    buffer[i++] = has_opt_wsp_ind_;

    TREE_SITTER_MARKDOWN_ASSERT(i == serialized_size());

    return i;
  }

  void deserialize(const unsigned char *buffer, unsigned length) {
    lxr_.clear();
    min_inl_dlms_.clear();
    inl_dlms_.clear();
    inl_ctx_stk_.clear();
    blk_dlms_.clear();
    blk_ctx_stk_.clear();
    has_opt_wsp_ind_ = false;

    if (length > 0) {
      size_t i = 0;

      i += lxr_.deserialize(&buffer[i]);
      i += min_inl_dlms_.deserialize(&buffer[i]);
      i += blk_dlms_.deserialize(&buffer[i]);
      i += blk_ctx_stk_.deserialize(&buffer[i]);
      has_opt_wsp_ind_ = buffer[i++];

      TREE_SITTER_MARKDOWN_ASSERT(i == length);
    }
  }

  bool scan(TSLexer *lexer, const bool *valid_symbols) {
    lxr_.init(lexer);
    lxr_.mrk_end();

    if (!min_inl_dlms_.empty() && is_inl_cls_mrk_sym(min_inl_dlms_.front().sym())) {
      TREE_SITTER_MARKDOWN_ASSERT(min_inl_dlms_.front().len() == 0);
      TokenType rlt_sym = min_inl_dlms_.front().tkn_typ(lxr_.cur_chr(), lxr_.lka_chr());
      if (rlt_sym != TKN_NOT_FOUND) {
        min_inl_dlms_.pop_front();
        has_opt_wsp_ind_ = false;
        return lxr_.ret_sym(rlt_sym);
      }
    }

    if (
      !blk_dlms_.empty()
      && (blk_dlms_.front().sym() == SYM_VRT_SPC
        || blk_dlms_.front().sym() == SYM_IND_COD_BGN_PFX
        || blk_dlms_.front().sym() == SYM_IND_COD_BGN_MKR
        || blk_dlms_.front().sym() == SYM_LST_ITM_CNT_BGN_MKR)
    ) {
      BlockDelimiter &dlm = blk_dlms_.front();
      TokenType rlt_sym = dlm.tkn_typ(lxr_.lka_chr());
      if (rlt_sym != TKN_NOT_FOUND) {
        bool is_opened = true;
        if (dlm.sym() == SYM_VRT_SPC) {
          if (dlm.len() == 1) {
            blk_dlms_.pop_front();
          } else {
            dlm.set_len(dlm.len() - 1);
          }
        } else if (dlm.sym() == SYM_IND_COD_BGN_PFX) {
          lxr_.adv_len(dlm.len());
          blk_dlms_.pop_front();
        } else {
          // At the nesting ceiling the context cannot be pushed.  Emitting the
          // opening token anyway would leave a block that never closes, because
          // closing delimiters are generated from the stack -- so the delimiter
          // is consumed without opening anything and without returning a token.
          // Consuming it is what keeps the scan moving; leaving it in place
          // would return the scanner to this same decision for ever.
          is_opened = blk_ctx_stk_.push(BlockContext(dlm.sym(), dlm.len(), dlm.ind()));
          blk_dlms_.pop_front();
        }
        if (is_opened) {
          lxr_.mrk_end();
          has_opt_wsp_ind_ = false;
          return lxr_.ret_sym(rlt_sym);
        }
        has_opt_wsp_ind_ = false;
      }
    }

    bool has_wsp = lxr_.adv_rpt(is_wsp_chr);

    if (has_wsp && is_eol_chr(lxr_.lka_chr()) && valid_symbols[TKN_TXT] && valid_symbols[TKN_BLK_LBK]) {
      lxr_.mrk_end();
      return lxr_.ret_sym(TKN_TXT); // BLK_TXT
    }

    if (!blk_dlms_.empty()) {
      BlockDelimiter &dlm = blk_dlms_.front();
      TokenType rlt_sym = dlm.tkn_typ(lxr_.lka_chr());
      if (rlt_sym != TKN_NOT_FOUND && is_blk_opn_sym(dlm.sym()) && blk_ctx_stk_.is_full()) {
        // Same ceiling as above, tested before the lexer is advanced: by the
        // time the push happens below, this branch has already consumed input
        // and committed to returning a token.  Dropping the opening delimiter
        // here leaves the content to parse at the depth already reached.
        blk_dlms_.pop_front();
        has_opt_wsp_ind_ = false;
      } else if (rlt_sym != TKN_NOT_FOUND) {
        // whitespaces are not considered part of block token
        if (has_wsp && !/*exception*/(rlt_sym == TKN_LIT_LBK || rlt_sym == TKN_BNK_LBK || rlt_sym == TKN_FEN_COD_CTN_BGN_MKR)) {
          lxr_.mrk_end();
          // has_opt_wsp_ind_ is not affected
          return lxr_.ret_sym(TKN_WSP);
        }
        LexedLength spc_cnt = lxr_.cur_spc();
        if (dlm.len() || rlt_sym == TKN_PGH_BGN_MKR) {
          lxr_.adv_len(dlm.len());
          lxr_.mrk_end();
        }
        if (is_blk_opn_sym(dlm.sym())) {
          blk_ctx_stk_.push(BlockContext(dlm.sym(), dlm.len(), dlm.ind()));
          if (is_lst_itm_bgn(dlm.sym()) || dlm.sym() == SYM_BQT_BGN) {
            has_opt_wsp_ind_ = is_wsp_chr(lxr_.lka_chr());
          } else {
            has_opt_wsp_ind_ = false;
          }
        } else if (is_blk_cls_sym(dlm.sym())) {
          TREE_SITTER_MARKDOWN_ASSERT(is_paired_blk_syms(blk_ctx_stk_.back().sym(), dlm.sym()));
          blk_ctx_stk_.pop();
          has_opt_wsp_ind_ = false;
        } else {
          has_opt_wsp_ind_ = false;
        }
        blk_dlms_.pop_front();
        if (rlt_sym == TKN_LIT_LBK) {
          if (valid_symbols[TKN_HRD_LBK] && spc_cnt >= 2) rlt_sym = TKN_HRD_LBK;
          else if (valid_symbols[TKN_SFT_LBK]) rlt_sym = TKN_SFT_LBK;
          if (rlt_sym != TKN_LIT_LBK) {
            lxr_.adv_rpt(is_wsp_chr);
            lxr_.mrk_end();
          }
        }
        if (rlt_sym == TKN_LIT_LBK || rlt_sym == TKN_BNK_LBK) {
          if (valid_symbols[TKN_BLK_LBK]) rlt_sym = TKN_BLK_LBK;
        }
        return lxr_.ret_sym(rlt_sym);
      }
    }

    if (!min_inl_dlms_.empty()) {
      if (has_wsp && !is_eol_chr(lxr_.lka_chr())) {
        lxr_.mrk_end();
        return lxr_.ret_sym(valid_symbols[TKN_TXT] ? TKN_TXT : TKN_WSP);
      }

      bool has_txt = false;
      while (!min_inl_dlms_.empty() && !is_eol_chr(lxr_.lka_chr())) {
        if (is_wht_chr(lxr_.lka_chr()) && valid_symbols[TKN_WRD]) {
          TREE_SITTER_MARKDOWN_ASSERT(has_txt);
          break;
        }

        MinimizedInlineDelimiter &dlm = min_inl_dlms_.front();
        TokenType rlt_sym = dlm.tkn_typ(lxr_.cur_chr(), lxr_.lka_chr());

        if (rlt_sym == TKN_NOT_FOUND) {
          if (is_wsp_chr(lxr_.lka_chr())) {
            TREE_SITTER_MARKDOWN_ASSERT(has_txt);
            lxr_.mrk_end();
            lxr_.adv_rpt(is_wsp_chr);
          } else {
            has_txt = true;
            lxr_.adv();
          }
          continue;
        }

        if (rlt_sym == TKN_TXT || !dlm.yes()) {
          lxr_.adv_len(dlm.len());
          min_inl_dlms_.pop_front();
          has_txt = true;
          continue;
        }

        if (has_txt) break;

        lxr_.adv_len(dlm.len());
        min_inl_dlms_.pop_front();

        if (rlt_sym == TKN_HRD_LBK) {
          TREE_SITTER_MARKDOWN_ASSERT(blk_dlms_.front().sym() == SYM_LIT_LBK);
          lxr_.adv_len(blk_dlms_.front().len());
          blk_dlms_.pop_front();
        }

        lxr_.mrk_end();
        has_opt_wsp_ind_ = false;
        return lxr_.ret_sym(rlt_sym);
      }

      has_opt_wsp_ind_ = false;

      if (is_eol_chr(lxr_.lka_chr()) && !has_txt) {
        lxr_.mrk_end();
        return lxr_.ret_sym(TKN_WSP);
      }

      TREE_SITTER_MARKDOWN_ASSERT(has_txt);
      if (!is_wsp_chr(lxr_.cur_chr())) lxr_.mrk_end();
      return lxr_.ret_sym(valid_symbols[TKN_WRD] ? TKN_WRD : TKN_TXT);
    }

    TREE_SITTER_MARKDOWN_ASSERT(min_inl_dlms_.empty());
    TREE_SITTER_MARKDOWN_ASSERT(inl_dlms_.empty());
    TREE_SITTER_MARKDOWN_ASSERT(inl_ctx_stk_.empty());

    if (blk_ctx_stk_.empty() && is_eof_chr(lxr_.lka_chr())) {
      TREE_SITTER_MARKDOWN_ASSERT(blk_dlms_.empty());
      if (valid_symbols[TKN_EOF]) return lxr_.ret_sym(TKN_EOF);
      return false;
    }

    if (is_eol_chr(lxr_.lka_chr())) {
      TREE_SITTER_MARKDOWN_ASSERT(blk_dlms_.empty());
      scn_eol(lxr_, blk_dlms_, blk_ctx_stk_);
      TREE_SITTER_MARKDOWN_ASSERT(!blk_dlms_.empty());
      return lxr_.ret_sym(TKN_LKA);
    }

    if (valid_symbols[TKN_IND_COD_BGN_PFX] || valid_symbols[TKN_LST_ITM_CNT_BGN_MKR]) {
      TREE_SITTER_MARKDOWN_ASSERT(blk_dlms_.empty());
      scn_blk(lxr_, blk_dlms_, blk_ctx_stk_, lxr_.cur_ind() - has_opt_wsp_ind_);
      TREE_SITTER_MARKDOWN_ASSERT(!blk_dlms_.empty());
      return lxr_.ret_sym(TKN_LKA);
    }

    lxr_.bgn_buf();

    Symbol sym = scn_inl(lxr_, inl_dlms_, inl_ctx_stk_, blk_dlms_, blk_ctx_stk_);
    if (sym == SYM_TXT) {
      do lxr_.mrk_end();
      while (!is_wht_chr(lxr_.lka_chr()) && scn_inl(lxr_, inl_dlms_, inl_ctx_stk_, blk_dlms_, blk_ctx_stk_) == SYM_TXT);
    } else if (sym != SYM_BLK_TXT) {
      TREE_SITTER_MARKDOWN_ASSERT(!inl_dlms_.empty());
    }

    inl_dlms_.transfer_to(min_inl_dlms_);

    if (sym == SYM_TXT || sym == SYM_BLK_TXT) {
      has_opt_wsp_ind_ = false;
      return lxr_.ret_sym(TKN_TXT);
    }

    return lxr_.ret_sym(TKN_LKA);
  }
};

}

extern "C" {

void *tree_sitter_markdown_external_scanner_create() {
  return new Scanner();
}

void tree_sitter_markdown_external_scanner_destroy(void *payload) {
  Scanner *scanner = static_cast<Scanner *>(payload);
  delete scanner;
}

unsigned tree_sitter_markdown_external_scanner_serialize(void *payload, char *buffer) {
  Scanner *scanner = static_cast<Scanner *>(payload);
#ifdef TREE_SITTER_MARKDOWN_AVOID_CRASH
  // This function is `extern "C"`.  An exception thrown through it cannot reach
  // the host and ends the process in std::terminate, which is how one markdown
  // file used to kill the whole program.  `scan` was guarded; these two were
  // not.  This is defence only -- by the time an assert in serialize fires the
  // buffer has already been written -- so the capacities, not this catch, are
  // what prevent the overrun.
  try {
    return scanner->serialize((unsigned char *)buffer);
  } catch (...) {
#ifdef TREE_SITTER_MARKDOWN_REPORT_SIZE
    fprintf(stderr, "SRLTHROW in serialize, size=%u\n", scanner->serialized_size());
#endif
    return 0;
  }
#else
  return scanner->serialize((unsigned char *)buffer);
#endif
}

void tree_sitter_markdown_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  Scanner *scanner = static_cast<Scanner *>(payload);
#ifdef TREE_SITTER_MARKDOWN_AVOID_CRASH
  try {
    scanner->deserialize((unsigned char *)buffer, length);
  } catch (...) {
#ifdef TREE_SITTER_MARKDOWN_REPORT_SIZE
    fprintf(stderr, "SRLTHROW in deserialize, length=%u\n", length);
#endif
    // A state that cannot be read back leaves the scanner cleared rather than
    // half-populated, which is the one state deserialize is always allowed to
    // produce (it is what a zero-length buffer means).
    scanner->deserialize(NULL, 0);
  }
#else
  scanner->deserialize((unsigned char *)buffer, length);
#endif
}

bool tree_sitter_markdown_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  Scanner *scanner = static_cast<Scanner *>(payload);
#ifdef TREE_SITTER_MARKDOWN_AVOID_CRASH
  try {
    return scanner->scan(lexer, valid_symbols);
  } catch (...) {
    return false;
  }
#else
  return scanner->scan(lexer, valid_symbols);
#endif
}

}
