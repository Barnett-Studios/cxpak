use tiktoken_rs::o200k_base;

pub struct TokenCounter {
    bpe: tiktoken_rs::CoreBPE,
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenCounter {
    pub fn new() -> Self {
        Self {
            bpe: o200k_base().expect("failed to load o200k_base tokenizer"),
        }
    }

    /// Count tokens in a string.
    ///
    /// Uses `encode_ordinary` so that special tokens such as `<|endoftext|>` are
    /// treated as literal text rather than single synthetic tokens, giving accurate
    /// counts for user-supplied content.
    pub fn count(&self, text: &str) -> usize {
        self.bpe.encode_ordinary(text).len()
    }

    /// Count tokens in a string, returning 0 for empty input
    pub fn count_or_zero(&self, text: &str) -> usize {
        if text.is_empty() {
            0
        } else {
            self.count(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_empty_string() {
        let counter = TokenCounter::new();
        assert_eq!(counter.count(""), 0);
    }

    #[test]
    fn test_count_simple_text() {
        let counter = TokenCounter::new();
        let count = counter.count("hello world");
        assert!(count > 0);
        assert!(count < 10);
    }

    #[test]
    fn test_count_code() {
        let counter = TokenCounter::new();
        let code = "fn main() {\n    println!(\"Hello, world!\");\n}";
        let count = counter.count(code);
        assert!(count > 0);
        assert!(count < 30);
    }

    #[test]
    fn test_count_or_zero_empty() {
        let counter = TokenCounter::new();
        assert_eq!(counter.count_or_zero(""), 0);
    }

    #[test]
    fn test_default_impl() {
        let counter = TokenCounter::default();
        assert!(counter.count("hello") > 0);
    }

    #[test]
    fn test_count_or_zero_nonempty() {
        let counter = TokenCounter::new();
        let count = counter.count_or_zero("hello world");
        assert!(count > 0);
    }

    #[test]
    fn test_count_special_token_treated_as_literal() {
        let counter = TokenCounter::new();
        // With encode_ordinary, <|endoftext|> is treated as multiple literal tokens,
        // not a single special token.  The count must be > 1.
        let count = counter.count("<|endoftext|>");
        assert!(
            count > 1,
            "special token must be counted as literal text (>1 token), got {count}"
        );
    }
}
