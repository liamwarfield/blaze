//! Ports of `lib/graph/DotOutputVisitor.java` and `lib/graph/LabelSerializer.java`.

use crate::graph::digraph::Digraph;
use crate::graph::graph_visitor::GraphVisitor;
use crate::graph::node::NodeId;

/// An interface for specifying a user-defined serialization of graph node labels as strings.
// Java: com/google/devtools/build/lib/graph/LabelSerializer.java:20 LabelSerializer
pub type LabelSerializer<'a, T> = Box<dyn Fn(&Digraph<T>, NodeId) -> String + 'a>;

/// An implementation of GraphVisitor for displaying graphs in dot format.
///
/// PORT: Java prints to a `PrintWriter`; this visitor appends the text to `out`, and the caller
/// encodes it with the writer's charset. Subclasses that override a method wrap this visitor and
/// call its methods for `super`.
// Java: com/google/devtools/build/lib/graph/DotOutputVisitor.java:24 DotOutputVisitor
pub struct DotOutputVisitor<'a, T> {
    disp: LabelSerializer<'a, T>,
    pub out: String,
    pub line_terminator: String,
}

impl<'a, T> DotOutputVisitor<'a, T> {
    /// Constructs a dot output visitor.
    ///
    /// The visitor writes to `out`, and rendering node labels as strings using the specified
    /// displayer, 'disp'.
    // Java: com/google/devtools/build/lib/graph/DotOutputVisitor.java:32 DotOutputVisitor.<init>
    pub fn new(line_terminator: &str, disp: LabelSerializer<'a, T>) -> Self {
        DotOutputVisitor {
            disp,
            out: String::new(),
            line_terminator: line_terminator.to_owned(),
        }
    }
}

impl<T> GraphVisitor<T> for DotOutputVisitor<'_, T> {
    // Java: com/google/devtools/build/lib/graph/DotOutputVisitor.java:44 DotOutputVisitor.beginVisit
    fn begin_visit(&mut self) {
        self.out
            .push_str(&format!("digraph mygraph {{{}", self.line_terminator));
    }

    /// PORT: `out.flush()` and the `closeAtEnd` close belong to the caller's writer.
    // Java: com/google/devtools/build/lib/graph/DotOutputVisitor.java:49 DotOutputVisitor.endVisit
    fn end_visit(&mut self) {
        self.out.push_str(&format!("}}{}", self.line_terminator));
    }

    // Java: com/google/devtools/build/lib/graph/DotOutputVisitor.java:58 DotOutputVisitor.visitEdge
    fn visit_edge(&mut self, graph: &Digraph<T>, lhs: NodeId, rhs: NodeId) {
        let line = format!(
            "  \"{}\" -> \"{}\"{}",
            (self.disp)(graph, lhs),
            (self.disp)(graph, rhs),
            self.line_terminator
        );
        self.out.push_str(&line);
    }

    // Java: com/google/devtools/build/lib/graph/DotOutputVisitor.java:63 DotOutputVisitor.visitNode
    fn visit_node(&mut self, graph: &Digraph<T>, node: NodeId) {
        let line = format!("  \"{}\"{}", (self.disp)(graph, node), self.line_terminator);
        self.out.push_str(&line);
    }
}
