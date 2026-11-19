//! Port of `query2/query/output/RankAndLabel.java`.

use std::cmp::Ordering;

use blaze_cmdline::Label;
use blaze_packages::target::LabelPrinter;

/// A (rank, label) pair, ordered by rank then label.
// Java: com/google/devtools/build/lib/query2/query/output/RankAndLabel.java:20 RankAndLabel
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankAndLabel {
    rank: i32,
    label: Label,
}

impl RankAndLabel {
    // Java: com/google/devtools/build/lib/query2/query/output/RankAndLabel.java:24 RankAndLabel.<init>
    pub fn new(rank: i32, label: Label) -> Self {
        RankAndLabel { rank, label }
    }

    // Java: com/google/devtools/build/lib/query2/query/output/RankAndLabel.java:29 RankAndLabel.getRank
    pub fn get_rank(&self) -> i32 {
        self.rank
    }

    // Java: com/google/devtools/build/lib/query2/query/output/RankAndLabel.java:46 RankAndLabel.toString
    pub fn to_string(&self, label_printer: &LabelPrinter) -> String {
        format!("{} {}", self.rank, label_printer.to_string(&self.label))
    }
}

impl Ord for RankAndLabel {
    // Java: com/google/devtools/build/lib/query2/query/output/RankAndLabel.java:34 RankAndLabel.compareTo
    fn cmp(&self, o: &RankAndLabel) -> Ordering {
        if self.rank != o.rank {
            return self.rank.cmp(&o.rank);
        }
        self.label.cmp(&o.label)
    }
}

impl PartialOrd for RankAndLabel {
    fn partial_cmp(&self, other: &RankAndLabel) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
