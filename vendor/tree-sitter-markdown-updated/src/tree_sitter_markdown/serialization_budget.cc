namespace tree_sitter_markdown {

// The three lists that make up the serialized state.  Kept as pointers to the
// scanner's own members so that `used()` always reflects what is actually
// there, rather than a running total that every pop, erase, clear and resize
// would have to remember to adjust.  Each serialized_size() is O(1).
struct SerializationBudget {
  const MinimizedInlineDelimiterList *min_inl_dlms;
  const BlockDelimiterList *blk_dlms;
  const BlockContextStack *blk_ctx_stk;

  SerializationBudget(
    const MinimizedInlineDelimiterList *minimized_inline_delimiters,
    const BlockDelimiterList *block_delimiters,
    const BlockContextStack *block_context_stack
  ):
    min_inl_dlms(minimized_inline_delimiters),
    blk_dlms(block_delimiters),
    blk_ctx_stk(block_context_stack) {}
};

unsigned srl_budget_used(const SerializationBudget *bgt) {
  if (bgt == NULL_PTR) return 0;
  return bgt->min_inl_dlms->serialized_size()
       + bgt->blk_dlms->serialized_size()
       + bgt->blk_ctx_stk->serialized_size();
}

}
