pub mod counter;
pub mod degrader;

#[derive(Debug, Clone)]
pub struct BudgetAllocation {
    pub metadata: usize,
    pub directory_tree: usize,
    pub module_map: usize,
    pub dependency_graph: usize,
    pub key_files: usize,
    pub signatures: usize,
    pub git_context: usize,
}

const METADATA_FIXED: usize = 500;

impl BudgetAllocation {
    pub fn allocate(total_budget: usize) -> Self {
        let remaining = total_budget.saturating_sub(METADATA_FIXED);
        Self {
            metadata: if total_budget >= METADATA_FIXED { METADATA_FIXED } else { total_budget },
            directory_tree: (remaining as f64 * 0.05) as usize,
            module_map: (remaining as f64 * 0.20) as usize,
            dependency_graph: (remaining as f64 * 0.15) as usize,
            key_files: (remaining as f64 * 0.20) as usize,
            signatures: (remaining as f64 * 0.30) as usize,
            git_context: (remaining as f64 * 0.10) as usize,
        }
    }

    pub fn total(&self) -> usize {
        self.metadata + self.directory_tree + self.module_map + self.dependency_graph
            + self.key_files + self.signatures + self.git_context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_50k() {
        let alloc = BudgetAllocation::allocate(50000);
        assert_eq!(alloc.metadata, 500);
        assert!(alloc.total() <= 50000);
        assert!(alloc.signatures > alloc.module_map);
        assert!(alloc.signatures > alloc.key_files);
    }

    #[test]
    fn test_allocate_tiny_budget() {
        let alloc = BudgetAllocation::allocate(1000);
        assert_eq!(alloc.metadata, 500);
        assert!(alloc.total() <= 1000);
    }

    #[test]
    fn test_allocate_zero() {
        let alloc = BudgetAllocation::allocate(0);
        assert_eq!(alloc.total(), 0);
    }
}
