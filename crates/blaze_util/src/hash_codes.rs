//! Port of `lib/util/HashCodes.java`: Java hash codes of several objects.
//!
//! The arguments are the objects' Java `hashCode()`s (0 for null, `hashObject`); each overload
//! of `hashObjects` is the same expression `h_n + 31 * (h_{n-1} + 31 * (... (h1 + 31)))`, which
//! is Horner's scheme from 1 in 32-bit wrapping arithmetic.

// PORT: skipped, a private constructor of a static-only class: HashCodes.HashCodes

/// `HashCodes.MULTIPLIER`.
// Java: com/google/devtools/build/lib/util/HashCodes.java:37 HashCodes.MULTIPLIER
pub const MULTIPLIER: i32 = 31;

/// `hashObject(obj)`: the object's `hashCode()`, or 0 for null.
// Java: com/google/devtools/build/lib/util/HashCodes.java:42 HashCodes.hashObject
pub fn hash_object(hash_code: Option<i32>) -> i32 {
    hash_code.unwrap_or(0)
}

/// `hashObjects(o1, ..., on)` for 2 to 8 objects, given their `hashObject` values.
// Java: com/google/devtools/build/lib/util/HashCodes.java:46 HashCodes.hashObjects
// Java: com/google/devtools/build/lib/util/HashCodes.java:52 HashCodes.hashObjects
// Java: com/google/devtools/build/lib/util/HashCodes.java:59 HashCodes.hashObjects
// Java: com/google/devtools/build/lib/util/HashCodes.java:68 HashCodes.hashObjects
// Java: com/google/devtools/build/lib/util/HashCodes.java:84 HashCodes.hashObjects
// Java: com/google/devtools/build/lib/util/HashCodes.java:106 HashCodes.hashObjects
// Java: com/google/devtools/build/lib/util/HashCodes.java:133 HashCodes.hashObjects
pub fn hash_objects(hashes: &[i32]) -> i32 {
    assert!(
        (2..=8).contains(&hashes.len()),
        "HashCodes.hashObjects has overloads for 2 to 8 objects"
    );
    hashes
        .iter()
        .fold(1i32, |acc, h| MULTIPLIER.wrapping_mul(acc).wrapping_add(*h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a Java test: the expressions of the 2- and 3-object overloads written out as in Java.
    #[test]
    fn matches_the_java_expressions() {
        let (h1, h2, h3) = (7, -123_456_789, i32::MAX);
        assert_eq!(
            hash_objects(&[h1, h2]),
            h2.wrapping_add(MULTIPLIER.wrapping_mul(h1.wrapping_add(MULTIPLIER)))
        );
        assert_eq!(
            hash_objects(&[h1, h2, h3]),
            h3.wrapping_add(MULTIPLIER.wrapping_mul(
                h2.wrapping_add(MULTIPLIER.wrapping_mul(h1.wrapping_add(MULTIPLIER)))
            ))
        );
        // Guava's Objects.hashCode / Arrays.hashCode is the same scheme.
        assert_eq!(hash_objects(&[1, 2]), 31 * (31 + 1) + 2);
        assert_eq!(hash_object(None), 0);
        assert_eq!(hash_object(Some(5)), 5);
    }
}
