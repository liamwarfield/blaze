//! `Converters.CaffeineSpecConverter` and Caffeine 3.1.8's `CaffeineSpec`, against results
//! recorded from Bazel 9.3.2's own classes (`scripts/options-converters/run`,
//! `tests/data/caffeine_spec_vectors.tsv`).

use blaze_options::caffeine_spec::{CaffeineSpec, JavaDuration};
use blaze_options::{CaffeineSpecConverter, Converter};

fn duration(d: Option<JavaDuration>) -> String {
    match d {
        None => "null".to_string(),
        Some(d) => format!("{}s{}n", d.seconds, d.nanos),
    }
}

fn strength(s: Option<blaze_options::caffeine_spec::Strength>) -> String {
    match s {
        None => "null".to_string(),
        Some(s) => s.to_string(),
    }
}

#[test]
fn caffeine_spec_converter_matches_java() {
    let data = include_str!("data/caffeine_spec_vectors.tsv");
    let mut n = 0;
    for line in data.lines() {
        let f: Vec<&str> = line.splitn(3, '\t').collect();
        let (input, want) = (f[1], f[2]);
        let got = std::panic::catch_unwind(|| CaffeineSpecConverter.convert(Some(input), None));
        let got = match got {
            Err(_) => "EXC:java.time.format.DateTimeParseException".to_string(),
            Ok(Err(e)) => format!("ERR:{}", e.get_message()),
            Ok(Ok(v)) => {
                let spec: CaffeineSpec = v.unwrap().get::<CaffeineSpec>();
                format!(
                    "OK:{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                    spec,
                    spec.initial_capacity,
                    spec.maximum_size,
                    spec.maximum_weight,
                    strength(spec.key_strength),
                    strength(spec.value_strength),
                    duration(spec.expire_after_write),
                    duration(spec.expire_after_access),
                    duration(spec.refresh_after_write),
                    spec.record_stats
                )
            }
        };
        assert_eq!(got, want, "input {input:?}");
        n += 1;
    }
    assert_eq!(n, 49);
}

/// `CaffeineSpec.equals` compares the parsed settings, not the specification string.
#[test]
fn caffeine_spec_equals_ignores_specification() {
    assert_eq!(
        CaffeineSpec::parse("maximumSize=5").unwrap(),
        CaffeineSpec::parse(" maximumSize = 5 ,").unwrap()
    );
    assert_ne!(
        CaffeineSpec::parse("maximumSize=5").unwrap(),
        CaffeineSpec::parse("maximumSize=6").unwrap()
    );
}
