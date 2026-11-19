//! Guava `Doubles.tryParse` (used by `ResourceConverter`), against results recorded from the
//! Guava in Bazel 9.3.2's server jar (`scripts/options-converters/run`,
//! `tests/data/doubles_try_parse_vectors.tsv`): decimal and hexadecimal forms, rounding,
//! subnormals, overflow and rejected inputs.

use blaze_optdefs::common::resource_converter::guava_doubles_try_parse;

#[test]
fn doubles_try_parse_matches_java() {
    let data = include_str!("data/doubles_try_parse_vectors.tsv");
    let mut n = 0;
    for line in data.lines() {
        let f: Vec<&str> = line.splitn(3, '\t').collect();
        let (input, want) = (f[1], f[2]);
        let got = match guava_doubles_try_parse(input) {
            None => "null".to_string(),
            Some(d) => format!("{:x}", d.to_bits()),
        };
        assert_eq!(got, want, "input {input:?}");
        n += 1;
    }
    assert_eq!(n, 35);
}
