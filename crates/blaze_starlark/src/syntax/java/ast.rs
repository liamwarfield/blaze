//! The syntax tree of Bazel's Starlark parser, as far as the Resolver and the error texts need it.
//!
//! Java: net/starlark/java/syntax/{Expression,Statement,Argument,Parameter,...}.java. Offsets are
//! char offsets into the file (Java `Node.getStartOffset`).

use super::lexer::TokenKind;

#[derive(Debug, Clone)]
pub struct Ident {
    pub name: String,
    pub start: usize,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Ident(Ident),
    Int {
        start: usize,
        value: String,
    },
    Float {
        start: usize,
        value: f64,
    },
    Str {
        start: usize,
        value: String,
    },
    /// `start` is None for an unparenthesized tuple (Java lbracketOffset -1).
    List {
        is_tuple: bool,
        start: Option<usize>,
        elems: Vec<Expr>,
    },
    Dict {
        start: usize,
        entries: Vec<(Expr, Expr)>,
    },
    Comprehension {
        is_dict: bool,
        start: usize,
        body: Box<CompBody>,
        clauses: Vec<Clause>,
    },
    Call {
        func: Box<Expr>,
        args: Vec<Argument>,
    },
    Dot {
        obj: Box<Expr>,
        field: Ident,
    },
    Index {
        obj: Box<Expr>,
        key: Box<Expr>,
    },
    Slice {
        obj: Box<Expr>,
        start: Option<Box<Expr>>,
        stop: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
    Binary {
        x: Box<Expr>,
        op: TokenKind,
        op_offset: usize,
        y: Box<Expr>,
    },
    Unary {
        op: TokenKind,
        offset: usize,
        x: Box<Expr>,
    },
    Conditional {
        then: Box<Expr>,
        cond: Box<Expr>,
        els: Box<Expr>,
    },
    Lambda {
        start: usize,
        params: Vec<Parameter>,
        body: Box<Expr>,
    },
    Cast {
        start: usize,
        ty: Box<Expr>,
        value: Box<Expr>,
    },
    IsInstance {
        start: usize,
        value: Box<Expr>,
        ty: Box<Expr>,
    },
    Ellipsis {
        start: usize,
    },
    TypeApplication {
        constructor: Ident,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone)]
pub enum CompBody {
    Expr(Expr),
    Entry(Expr, Expr),
}

#[derive(Debug, Clone)]
pub enum Clause {
    For { vars: Expr, iterable: Expr },
    If { cond: Expr },
}

#[derive(Debug, Clone)]
pub enum Argument {
    Positional(Expr),
    Keyword(Ident, Expr),
    Star(usize, Expr),
    StarStar(usize, Expr),
}

#[derive(Debug, Clone)]
pub enum Parameter {
    Mandatory(Ident, Option<Expr>),
    Optional(Ident, Option<Expr>, Expr),
    Star(usize, Option<Ident>, Option<Expr>),
    StarStar(usize, Ident, Option<Expr>),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Assign {
        lhs: Expr,
        ty: Option<Expr>,
        op: Option<TokenKind>,
        op_offset: usize,
        rhs: Expr,
    },
    Expr(Expr),
    If {
        start: usize,
        cond: Expr,
        then: Vec<Stmt>,
        els: Option<Vec<Stmt>>,
    },
    For {
        start: usize,
        vars: Expr,
        collection: Expr,
        body: Vec<Stmt>,
    },
    Def {
        start: usize,
        name: Ident,
        type_params: Vec<Ident>,
        params: Vec<Parameter>,
        ret: Option<Expr>,
        body: Vec<Stmt>,
    },
    Return {
        start: usize,
        result: Option<Expr>,
    },
    Load {
        start: usize,
        bindings: Vec<(Ident, Ident)>,
    },
    Flow {
        kind: TokenKind,
        start: usize,
    },
    TypeAlias {
        start: usize,
        name: Ident,
        params: Vec<Ident>,
        def: Expr,
    },
    Var {
        name: Ident,
        ty: Expr,
    },
}

impl Expr {
    /// Java `Node.getStartOffset`.
    pub fn start(&self) -> usize {
        match self {
            Expr::Ident(i) => i.start,
            Expr::Int { start, .. }
            | Expr::Float { start, .. }
            | Expr::Str { start, .. }
            | Expr::Dict { start, .. }
            | Expr::Comprehension { start, .. }
            | Expr::Lambda { start, .. }
            | Expr::Cast { start, .. }
            | Expr::IsInstance { start, .. }
            | Expr::Ellipsis { start } => *start,
            Expr::List { start, elems, .. } => match start {
                Some(s) => *s,
                None => elems.first().map_or(0, Expr::start),
            },
            Expr::Call { func, .. } => func.start(),
            Expr::Dot { obj, .. } | Expr::Index { obj, .. } | Expr::Slice { obj, .. } => {
                obj.start()
            }
            Expr::Binary { x, .. } => x.start(),
            Expr::Unary { offset, .. } => *offset,
            Expr::Conditional { then, .. } => then.start(),
            Expr::TypeApplication { constructor, .. } => constructor.start,
        }
    }
}

impl Argument {
    pub fn start(&self) -> usize {
        match self {
            Argument::Positional(e) => e.start(),
            Argument::Keyword(id, _) => id.start,
            Argument::Star(o, _) | Argument::StarStar(o, _) => *o,
        }
    }

    pub fn value(&self) -> &Expr {
        match self {
            Argument::Positional(e)
            | Argument::Keyword(_, e)
            | Argument::Star(_, e)
            | Argument::StarStar(_, e) => e,
        }
    }
}

impl Parameter {
    pub fn start(&self) -> usize {
        match self {
            Parameter::Mandatory(id, _) | Parameter::Optional(id, _, _) => id.start,
            Parameter::Star(o, _, _) | Parameter::StarStar(o, _, _) => *o,
        }
    }

    pub fn ident(&self) -> Option<&Ident> {
        match self {
            Parameter::Mandatory(id, _) | Parameter::Optional(id, _, _) => Some(id),
            Parameter::Star(_, id, _) => id.as_ref(),
            Parameter::StarStar(_, id, _) => Some(id),
        }
    }

    pub fn ty(&self) -> Option<&Expr> {
        match self {
            Parameter::Mandatory(_, t)
            | Parameter::Optional(_, t, _)
            | Parameter::Star(_, _, t)
            | Parameter::StarStar(_, _, t) => t.as_ref(),
        }
    }
}

impl Stmt {
    /// Java `Node.getStartOffset`.
    pub fn start(&self) -> usize {
        match self {
            Stmt::Assign { lhs, .. } => lhs.start(),
            Stmt::Expr(e) => e.start(),
            Stmt::If { start, .. }
            | Stmt::For { start, .. }
            | Stmt::Def { start, .. }
            | Stmt::Return { start, .. }
            | Stmt::Load { start, .. }
            | Stmt::Flow { start, .. }
            | Stmt::TypeAlias { start, .. } => *start,
            Stmt::Var { name, .. } => name.start,
        }
    }
}

/// Java `Identifier.isValid`.
// Java: net/starlark/java/syntax/Identifier.java:82 Identifier.isValid
pub fn is_valid_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()) || c == '_')
}

/// Java `Double.toString`.
pub fn java_double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".to_owned();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0.0" } else { "0.0" }.to_owned();
    }
    let e = format!("{:e}", d.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if d < 0.0 { "-" } else { "" };
    if (1e-3..1e7).contains(&d.abs()) {
        let s = if exp >= 0 {
            let int_len = exp as usize + 1;
            let mut ds = digits.clone();
            while ds.len() < int_len {
                ds.push('0');
            }
            let (i, f) = ds.split_at(int_len);
            format!("{i}.{}", if f.is_empty() { "0" } else { f })
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        };
        format!("{sign}{s}")
    } else {
        let (i, f) = digits.split_at(1);
        format!("{sign}{i}.{}E{exp}", if f.is_empty() { "0" } else { f })
    }
}

/// Java `Expression.toString` (Node.prettyPrint, overridden by several node classes).
// Java: net/starlark/java/syntax/NodePrinter.java:313 NodePrinter.printExpr
pub fn expr_to_string(e: &Expr) -> String {
    match e {
        Expr::Ellipsis { .. } => "...".to_owned(),
        // Java: UnaryOperatorExpression.java:51 toString
        Expr::Unary { op, x, .. } => format!(
            "{}{}",
            if *op == TokenKind::Not {
                "not ".to_owned()
            } else {
                op.to_string()
            },
            expr_to_string(x)
        ),
        // Java: BinaryOperatorExpression.java:86 toString
        Expr::Binary { x, op, y, .. } => {
            format!("{} {} {}", expr_to_string(x), op, expr_to_string(y))
        }
        // Java: CallExpression.java:86 toString
        Expr::Call { func, args } => {
            let mut buf = expr_to_string(func);
            buf.push('(');
            let items: Vec<String> = args.iter().map(argument_to_string).collect();
            append_nodes(&mut buf, &items);
            buf.push(')');
            buf
        }
        // Java: ListExpression.java:71 toString
        Expr::List {
            is_tuple, elems, ..
        } => {
            let mut buf = String::from(if *is_tuple { "(" } else { "[" });
            let items: Vec<String> = elems.iter().map(expr_to_string).collect();
            append_nodes(&mut buf, &items);
            if *is_tuple && elems.len() == 1 {
                buf.push(',');
            }
            buf.push(if *is_tuple { ')' } else { ']' });
            buf
        }
        _ => {
            let mut buf = String::new();
            print_expr(&mut buf, e, false);
            buf
        }
    }
}

// Java: net/starlark/java/syntax/ListExpression.java:84 ListExpression.appendNodes
fn append_nodes(buf: &mut String, items: &[String]) {
    let n = items.len();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            buf.push_str(", ");
        }
        let mark = buf.len();
        buf.push_str(item);
        if buf.chars().count() >= 32 || (i == 4 && i + 1 < n) {
            buf.truncate(mark);
            buf.push_str(&format!("+{} more", n - i));
            break;
        }
    }
}

fn argument_to_string(a: &Argument) -> String {
    let mut buf = String::new();
    print_argument(&mut buf, a);
    buf
}

fn print_argument(buf: &mut String, a: &Argument) {
    match a {
        Argument::Positional(_) => {}
        Argument::Keyword(id, _) => {
            buf.push_str(&id.name);
            buf.push_str(" = ");
        }
        Argument::Star(..) => buf.push('*'),
        Argument::StarStar(..) => buf.push_str("**"),
    }
    print_expr(buf, a.value(), true);
}

fn print_parameter(buf: &mut String, p: &Parameter) {
    match p {
        Parameter::Mandatory(id, _) => buf.push_str(&id.name),
        Parameter::Optional(id, _, d) => {
            buf.push_str(&id.name);
            buf.push('=');
            print_expr(buf, d, false);
        }
        Parameter::Star(_, id, _) => {
            buf.push('*');
            if let Some(id) = id {
                buf.push_str(&id.name);
            }
        }
        Parameter::StarStar(_, id, _) => {
            buf.push_str("**");
            buf.push_str(&id.name);
        }
    }
}

fn print_expr(buf: &mut String, e: &Expr, can_skip_parenthesis: bool) {
    match e {
        Expr::Binary { x, op, y, .. } => {
            if !can_skip_parenthesis {
                buf.push('(');
            }
            print_expr(buf, x, false);
            buf.push(' ');
            buf.push_str(op.text());
            buf.push(' ');
            print_expr(buf, y, false);
            if !can_skip_parenthesis {
                buf.push(')');
            }
        }
        Expr::Comprehension {
            is_dict,
            body,
            clauses,
            ..
        } => {
            buf.push(if *is_dict { '{' } else { '[' });
            match &**body {
                CompBody::Expr(e) => print_expr(buf, e, false),
                CompBody::Entry(k, v) => {
                    print_expr(buf, k, false);
                    buf.push_str(": ");
                    print_expr(buf, v, false);
                }
            }
            for c in clauses {
                buf.push(' ');
                match c {
                    Clause::For { vars, iterable } => {
                        buf.push_str("for ");
                        print_expr(buf, vars, false);
                        buf.push_str(" in ");
                        print_expr(buf, iterable, false);
                    }
                    Clause::If { cond } => {
                        buf.push_str("if ");
                        print_expr(buf, cond, false);
                    }
                }
            }
            buf.push(if *is_dict { '}' } else { ']' });
        }
        Expr::Conditional { then, cond, els } => {
            print_expr(buf, then, false);
            buf.push_str(" if ");
            print_expr(buf, cond, false);
            buf.push_str(" else ");
            print_expr(buf, els, false);
        }
        Expr::Dict { entries, .. } => {
            buf.push('{');
            let mut sep = "";
            for (k, v) in entries {
                buf.push_str(sep);
                print_expr(buf, k, false);
                buf.push_str(": ");
                print_expr(buf, v, false);
                sep = ", ";
            }
            buf.push('}');
        }
        Expr::Dot { obj, field } => {
            print_expr(buf, obj, false);
            buf.push('.');
            buf.push_str(&field.name);
        }
        Expr::Call { func, args } => {
            print_expr(buf, func, false);
            buf.push('(');
            let mut sep = "";
            for a in args {
                buf.push_str(sep);
                print_argument(buf, a);
                sep = ", ";
            }
            buf.push(')');
        }
        Expr::Cast { ty, value, .. } => {
            buf.push_str("cast(");
            print_expr(buf, ty, true);
            buf.push_str(", ");
            print_expr(buf, value, true);
            buf.push(')');
        }
        Expr::Ellipsis { .. } => buf.push_str("..."),
        Expr::Ident(id) => buf.push_str(&id.name),
        Expr::Index { obj, key } => {
            print_expr(buf, obj, false);
            buf.push('[');
            print_expr(buf, key, false);
            buf.push(']');
        }
        Expr::Int { value, .. } => buf.push_str(value),
        Expr::IsInstance { value, ty, .. } => {
            buf.push_str("isinstance(");
            print_expr(buf, value, true);
            buf.push_str(", ");
            print_expr(buf, ty, true);
            buf.push(')');
        }
        Expr::Float { value, .. } => buf.push_str(&java_double_to_string(*value)),
        Expr::Lambda { params, body, .. } => {
            buf.push_str("lambda");
            let mut sep = " ";
            for p in params {
                buf.push_str(sep);
                sep = ", ";
                print_parameter(buf, p);
            }
            buf.push_str(": ");
            print_expr(buf, body, false);
        }
        Expr::List {
            is_tuple, elems, ..
        } => {
            buf.push(if *is_tuple { '(' } else { '[' });
            let mut sep = "";
            for e in elems {
                buf.push_str(sep);
                print_expr(buf, e, true);
                sep = ", ";
            }
            if *is_tuple && elems.len() == 1 {
                buf.push(',');
            }
            buf.push(if *is_tuple { ')' } else { ']' });
        }
        Expr::Slice {
            obj,
            start,
            stop,
            step,
        } => {
            print_expr(buf, obj, false);
            buf.push('[');
            if let Some(s) = start {
                print_expr(buf, s, false);
            }
            buf.push(':');
            if let Some(s) = stop {
                print_expr(buf, s, false);
            }
            if let Some(s) = step {
                buf.push(':');
                print_expr(buf, s, false);
            }
            buf.push(']');
        }
        Expr::Str { value, .. } => {
            buf.push('"');
            for c in value.chars() {
                match c {
                    '"' => buf.push_str("\\\""),
                    '\\' => buf.push_str("\\\\"),
                    '\r' => buf.push_str("\\r"),
                    '\n' => buf.push_str("\\n"),
                    '\t' => buf.push_str("\\t"),
                    c if (c as u32) < 32 => buf.push_str(&format!("\\x{:02x}", c as u32)),
                    c => buf.push(c),
                }
            }
            buf.push('"');
        }
        Expr::Unary { op, x, .. } => {
            if *op == TokenKind::Not {
                buf.push_str("not ");
            } else {
                buf.push_str(op.text());
            }
            if !can_skip_parenthesis {
                buf.push('(');
            }
            print_expr(buf, x, false);
            if !can_skip_parenthesis {
                buf.push(')');
            }
        }
        Expr::TypeApplication { constructor, args } => {
            buf.push_str(&constructor.name);
            buf.push('[');
            let mut sep = "";
            for a in args {
                buf.push_str(sep);
                print_expr(buf, a, true);
                sep = ", ";
            }
            buf.push(']');
        }
    }
}
