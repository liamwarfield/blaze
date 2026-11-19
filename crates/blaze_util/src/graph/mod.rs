//! Port of `lib/graph`: a generic directed graph (`Digraph`), its nodes, depth-first search and
//! visitors (including GraphViz `dot` output).

pub mod collecting_visitor;
mod concurrent_collection_wrapper;
pub mod dfs;
pub mod digraph;
pub mod dot_output_visitor;
pub mod graph_visitor;
mod java_priority_queue;
pub mod node;

pub use collecting_visitor::CollectingVisitor;
pub use dfs::{Dfs, DfsOrder};
pub use digraph::{Comparator, Digraph, NodeSet, get_path_to_tree_node};
pub use dot_output_visitor::{DotOutputVisitor, LabelSerializer};
pub use graph_visitor::{AbstractGraphVisitor, GraphVisitor};
pub use node::{Node, NodeId};
