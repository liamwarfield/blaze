//! Tests of [`Table`], the stand-in for Guava's `ImmutableTable` as built by
//! `ImmutableTable.builder()` in `BazelDepGraphFunction`.
//!
//! Not a JUnit port. Every expected line below was printed by Guava 33.5.0-jre (the version
//! Bazel 9.3.2 builds with) for `ImmutableTable.builder()` given the first `n` cells:
//!
//! ```java
//! sb.append(n).append(" ").append(t.getClass().getSimpleName());
//! sb.append(" rows=").append(t.rowKeySet());
//! sb.append(" cols=").append(t.columnKeySet());
//! for (String r : t.rowKeySet()) sb.append(" row(" + r + ")=").append(t.row(r).keySet());
//! for (String c : t.columnKeySet()) sb.append(" col(" + c + ")=").append(t.column(c).keySet());
//! sb.append(" cells=").append(t.values());
//! ```
//!
//! where the value of cell `(r, c)` is the string `r + c`.

use blaze_bzlmod_model::Table;

fn list<'a>(items: impl Iterator<Item = &'a String>) -> String {
    format!(
        "[{}]",
        items.map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
    )
}

/// The Java dump line for the table holding the first `n` cells.
fn dump(cells: &[(&str, &str)], n: usize) -> String {
    let mut t: Table<String, String, String> = Table::new();
    for (r, c) in &cells[..n] {
        t.put(r.to_string(), c.to_string(), format!("{r}{c}"));
    }
    assert_eq!(t.size(), n);
    assert_eq!(t.is_empty(), n == 0);
    let class = match n {
        0 => "SparseImmutableTable", // ImmutableTable.of()
        1 => "SingletonImmutableTable",
        _ if t.is_dense() => "DenseImmutableTable",
        _ => "SparseImmutableTable",
    };
    let mut out = format!(
        "{n} {class} rows={} cols={}",
        list(t.row_key_set()),
        list(t.column_key_set())
    );
    let rows: Vec<String> = t.row_key_set().cloned().collect();
    for r in &rows {
        out.push_str(&format!(" row({r})={}", list(t.row(r).unwrap().keys())));
        // The values follow the keys.
        for (c, v) in t.row(r).unwrap() {
            assert_eq!(v, &format!("{r}{c}"));
            assert_eq!(t.get(r, c), Some(v));
        }
    }
    let columns: Vec<String> = t.column_key_set().cloned().collect();
    for c in &columns {
        out.push_str(&format!(" col({c})={}", list(t.column(c).unwrap().keys())));
        for (r, v) in t.column(c).unwrap() {
            assert_eq!(v, &format!("{r}{c}"));
        }
    }
    let values: Vec<&String> = t.cell_set().map(|(_, _, v)| v).collect();
    out.push_str(&format!(" cells={}", list(values.into_iter())));
    out
}

fn check(cells: &[(&str, &str)], expected: &[&str]) {
    assert_eq!(expected.len(), cells.len() + 1);
    for (n, line) in expected.iter().enumerate() {
        assert_eq!(&dump(cells, n), line);
    }
}

/// The shape of `BazelDepGraphFunction.getExtensionUsagesById`: rows are extensions, columns are
/// modules, cells are put module by module. Modules `A` and `B` use `e1` and `e2` in different
/// orders, so `column(B)` tells the implementations apart; `row(e3)` and `row(e4)` do too once
/// the table is dense again.
#[test]
fn usages_table_follows_guava_for_every_number_of_cells() {
    check(
        &[
            ("e1", "A"),
            ("e2", "A"),
            ("e2", "B"),
            ("e1", "B"),
            ("e3", "C"),
            ("e4", "D"),
            ("e4", "A"),
            ("e3", "A"),
            ("e4", "B"),
            ("e3", "B"),
        ],
        &[
            "0 SparseImmutableTable rows=[] cols=[] cells=[]",
            "1 SingletonImmutableTable rows=[e1] cols=[A] row(e1)=[A] col(A)=[e1] cells=[e1A]",
            "2 DenseImmutableTable rows=[e1, e2] cols=[A] row(e1)=[A] row(e2)=[A] col(A)=[e1, e2] cells=[e1A, e2A]",
            "3 DenseImmutableTable rows=[e1, e2] cols=[A, B] row(e1)=[A] row(e2)=[A, B] col(A)=[e1, e2] col(B)=[e2] cells=[e1A, e2A, e2B]",
            "4 DenseImmutableTable rows=[e1, e2] cols=[A, B] row(e1)=[A, B] row(e2)=[A, B] col(A)=[e1, e2] col(B)=[e1, e2] cells=[e1A, e2A, e2B, e1B]",
            "5 DenseImmutableTable rows=[e1, e2, e3] cols=[A, B, C] row(e1)=[A, B] row(e2)=[A, B] row(e3)=[C] col(A)=[e1, e2] col(B)=[e1, e2] col(C)=[e3] cells=[e1A, e2A, e2B, e1B, e3C]",
            "6 SparseImmutableTable rows=[e1, e2, e3, e4] cols=[A, B, C, D] row(e1)=[A, B] row(e2)=[A, B] row(e3)=[C] row(e4)=[D] col(A)=[e1, e2] col(B)=[e2, e1] col(C)=[e3] col(D)=[e4] cells=[e1A, e2A, e2B, e1B, e3C, e4D]",
            "7 SparseImmutableTable rows=[e1, e2, e3, e4] cols=[A, B, C, D] row(e1)=[A, B] row(e2)=[A, B] row(e3)=[C] row(e4)=[D, A] col(A)=[e1, e2, e4] col(B)=[e2, e1] col(C)=[e3] col(D)=[e4] cells=[e1A, e2A, e2B, e1B, e3C, e4D, e4A]",
            "8 SparseImmutableTable rows=[e1, e2, e3, e4] cols=[A, B, C, D] row(e1)=[A, B] row(e2)=[A, B] row(e3)=[C, A] row(e4)=[D, A] col(A)=[e1, e2, e4, e3] col(B)=[e2, e1] col(C)=[e3] col(D)=[e4] cells=[e1A, e2A, e2B, e1B, e3C, e4D, e4A, e3A]",
            "9 DenseImmutableTable rows=[e1, e2, e3, e4] cols=[A, B, C, D] row(e1)=[A, B] row(e2)=[A, B] row(e3)=[A, C] row(e4)=[A, B, D] col(A)=[e1, e2, e3, e4] col(B)=[e1, e2, e4] col(C)=[e3] col(D)=[e4] cells=[e1A, e2A, e2B, e1B, e3C, e4D, e4A, e3A, e4B]",
            "10 DenseImmutableTable rows=[e1, e2, e3, e4] cols=[A, B, C, D] row(e1)=[A, B] row(e2)=[A, B] row(e3)=[A, B, C] row(e4)=[A, B, D] col(A)=[e1, e2, e3, e4] col(B)=[e1, e2, e3, e4] col(C)=[e3] col(D)=[e4] cells=[e1A, e2A, e2B, e1B, e3C, e4D, e4A, e3A, e4B, e3B]",
        ],
    );
}

/// The shape of `BazelDepGraphFunction.resolveRepoOverrides`: rows are extensions, columns are
/// repo names, cells are put extension by extension. `row(e2)` tells the implementations apart.
#[test]
fn overrides_table_follows_guava_for_every_number_of_cells() {
    check(
        &[
            ("e1", "x"),
            ("e1", "y"),
            ("e2", "y"),
            ("e2", "x"),
            ("e3", "z"),
            ("e4", "w"),
        ],
        &[
            "0 SparseImmutableTable rows=[] cols=[] cells=[]",
            "1 SingletonImmutableTable rows=[e1] cols=[x] row(e1)=[x] col(x)=[e1] cells=[e1x]",
            "2 DenseImmutableTable rows=[e1] cols=[x, y] row(e1)=[x, y] col(x)=[e1] col(y)=[e1] cells=[e1x, e1y]",
            "3 DenseImmutableTable rows=[e1, e2] cols=[x, y] row(e1)=[x, y] row(e2)=[y] col(x)=[e1] col(y)=[e1, e2] cells=[e1x, e1y, e2y]",
            "4 DenseImmutableTable rows=[e1, e2] cols=[x, y] row(e1)=[x, y] row(e2)=[x, y] col(x)=[e1, e2] col(y)=[e1, e2] cells=[e1x, e1y, e2y, e2x]",
            "5 DenseImmutableTable rows=[e1, e2, e3] cols=[x, y, z] row(e1)=[x, y] row(e2)=[x, y] row(e3)=[z] col(x)=[e1, e2] col(y)=[e1, e2] col(z)=[e3] cells=[e1x, e1y, e2y, e2x, e3z]",
            "6 SparseImmutableTable rows=[e1, e2, e3, e4] cols=[x, y, z, w] row(e1)=[x, y] row(e2)=[y, x] row(e3)=[z] row(e4)=[w] col(x)=[e1, e2] col(y)=[e1, e2] col(z)=[e3] col(w)=[e4] cells=[e1x, e1y, e2y, e2x, e3z, e4w]",
        ],
    );
}

#[test]
fn one_column_table_is_dense() {
    check(
        &[("e1", "A"), ("e2", "A"), ("e3", "A")],
        &[
            "0 SparseImmutableTable rows=[] cols=[] cells=[]",
            "1 SingletonImmutableTable rows=[e1] cols=[A] row(e1)=[A] col(A)=[e1] cells=[e1A]",
            "2 DenseImmutableTable rows=[e1, e2] cols=[A] row(e1)=[A] row(e2)=[A] col(A)=[e1, e2] cells=[e1A, e2A]",
            "3 DenseImmutableTable rows=[e1, e2, e3] cols=[A] row(e1)=[A] row(e2)=[A] row(e3)=[A] col(A)=[e1, e2, e3] cells=[e1A, e2A, e3A]",
        ],
    );
}

/// Exactly half of the cells filled is still sparse (`cells > rows * columns / 2`).
#[test]
fn half_filled_table_is_sparse() {
    check(
        &[("e1", "A"), ("e2", "B"), ("e2", "A")],
        &[
            "0 SparseImmutableTable rows=[] cols=[] cells=[]",
            "1 SingletonImmutableTable rows=[e1] cols=[A] row(e1)=[A] col(A)=[e1] cells=[e1A]",
            "2 SparseImmutableTable rows=[e1, e2] cols=[A, B] row(e1)=[A] row(e2)=[B] col(A)=[e1] col(B)=[e2] cells=[e1A, e2B]",
            "3 DenseImmutableTable rows=[e1, e2] cols=[A, B] row(e1)=[A] row(e2)=[A, B] col(A)=[e1, e2] col(B)=[e2] cells=[e1A, e2B, e2A]",
        ],
    );
}

#[test]
fn empty_and_missing_keys() {
    let mut t: Table<String, String, String> = Table::new();
    assert!(t.is_empty());
    assert!(!t.is_dense());
    // ImmutableTable.of().row("x") and .column("x") are empty maps.
    assert!(t.row(&"x".to_string()).is_none());
    assert!(t.column(&"x".to_string()).is_none());
    t.put("r".to_string(), "c".to_string(), "v".to_string());
    assert!(!t.is_dense());
    assert!(t.row(&"zz".to_string()).is_none());
    assert!(t.contains_row(&"r".to_string()));
    assert!(!t.contains_row(&"zz".to_string()));
}

/// `AbstractTable.equals` compares cell sets, so the insertion order does not matter.
#[test]
fn equality_ignores_insertion_order() {
    let mut a: Table<String, String, String> = Table::new();
    let mut b: Table<String, String, String> = Table::new();
    for (r, c) in [("e1", "A"), ("e2", "A"), ("e2", "B")] {
        a.put(r.to_string(), c.to_string(), format!("{r}{c}"));
    }
    for (r, c) in [("e2", "B"), ("e1", "A"), ("e2", "A")] {
        b.put(r.to_string(), c.to_string(), format!("{r}{c}"));
    }
    assert_eq!(a, b);
    b.put("e3".to_string(), "A".to_string(), "e3A".to_string());
    assert_ne!(a, b);
}

/// `checkNoDuplicate`: "Duplicate key: (row=r, column=c), values: [v2, v1]."
#[test]
#[should_panic(expected = "Duplicate key: (row=\"r\", column=\"c\")")]
fn duplicate_cell_is_an_illegal_argument() {
    let mut t: Table<String, String, String> = Table::new();
    t.put("r".to_string(), "c".to_string(), "v1".to_string());
    t.put("r2".to_string(), "c".to_string(), "x".to_string());
    t.put("r".to_string(), "c".to_string(), "v2".to_string());
}
