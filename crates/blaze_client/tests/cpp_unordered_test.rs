//! `cpp_unordered`: the iteration order of `std::unordered_multiset<std::string>` that
//! `AreStartupOptionsDifferent` (src/main/cpp/blaze.cc:1021) prints.
//!
//! There is no C++ unit test of this order at the pin. The expected values come from the C++:
//! a program that performs the same operations on a `std::unordered_multiset<std::string>`
//! and prints `std::hash<std::string>` values and the iteration order, built with g++ 16.2.1
//! against the system's libstdc++ (the library Bazel's dynamically linked client uses at run
//! time on this machine).

use blaze_client::cpp_unordered::{CppUnorderedMultiset, std_hash_string};

#[test]
fn std_hash_matches_libstdcxx() {
    assert_eq!(4993892634952068459, std_hash_string(b"a"));
    assert_eq!(3663726644998027833, std_hash_string(b"abc"));
    assert_eq!(10113836418918046885, std_hash_string(b"12345678"));
    assert_eq!(12315982496267331008, std_hash_string(b"123456789"));
    assert_eq!(
        5163442703065866516,
        std_hash_string(b"--host_jvm_args=-Dfoo=1")
    );
    assert_eq!(14188689160394075173, std_hash_string(b"-Dfoo=1"));
    assert_eq!(6142509188972423790, std_hash_string(b""));
}

fn order(set: &CppUnorderedMultiset) -> Vec<&str> {
    set.iter().map(String::as_str).collect()
}

#[test]
fn small_set_iterates_in_libstdcxx_order() {
    let mut set = CppUnorderedMultiset::new();
    for s in ["a", "b", "c"] {
        set.insert(s.to_string());
    }
    assert_eq!(vec!["c", "b", "a"], order(&set));

    let mut set = CppUnorderedMultiset::new();
    for s in ["--host_jvm_args=-Dfoo=1", "-Dfoo=1", "x", "x", "y"] {
        set.insert(s.to_string());
    }
    assert_eq!(
        vec!["y", "x", "x", "-Dfoo=1", "--host_jvm_args=-Dfoo=1"],
        order(&set)
    );
}

#[test]
fn rehashed_set_with_erasures_iterates_in_libstdcxx_order() {
    let mut set = CppUnorderedMultiset::new();
    for i in 1..=40 {
        set.insert(format!("--opt{}=v", i));
    }
    for s in ["--opt7=v", "--opt33=v", "--opt1=v"] {
        assert!(set.erase_one(s));
    }
    set.insert("-Dfoo=1".to_string());
    set.insert("--host_jvm_args=-Dfoo=1".to_string());
    let expected: Vec<String> = "-Dfoo=1 --opt40=v --opt38=v --opt37=v --opt36=v --opt34=v \
         --host_jvm_args=-Dfoo=1 --opt32=v --opt30=v --opt11=v --opt10=v --opt19=v --opt5=v \
         --opt4=v --opt13=v --opt31=v --opt21=v --opt3=v --opt27=v --opt6=v --opt9=v --opt29=v \
         --opt2=v --opt35=v --opt15=v --opt14=v --opt8=v --opt16=v --opt39=v --opt18=v \
         --opt20=v --opt22=v --opt24=v --opt25=v --opt12=v --opt26=v --opt23=v --opt17=v \
         --opt28=v"
        .split_whitespace()
        .map(str::to_string)
        .collect();
    assert_eq!(expected, set.iter().cloned().collect::<Vec<_>>());
}

#[test]
fn seventy_elements_with_erasures_iterate_in_libstdcxx_order() {
    let mut set = CppUnorderedMultiset::new();
    for i in 1..=70 {
        set.insert(format!("arg{}", i));
    }
    assert!(set.erase_one("arg5"));
    assert!(set.erase_one("arg60"));
    // The last seven elements of the C++ iteration (127 buckets).
    let all: Vec<&str> = order(&set);
    assert_eq!(68, all.len());
    assert_eq!(
        vec!["arg8", "arg35", "arg42", "arg58", "arg55", "arg4", "arg59"],
        all[all.len() - 7..].to_vec()
    );
}
