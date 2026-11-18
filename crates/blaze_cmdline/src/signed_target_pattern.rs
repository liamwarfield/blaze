//! Port of `SignedTargetPattern`.

use crate::target_parsing_exception::TargetParsingException;
use crate::target_pattern::{TargetPattern, TargetPatternParser};

/// Whether a target pattern begins with a minus sign (NEGATIVE) or not (POSITIVE).
// Java: com/google/devtools/build/lib/cmdline/SignedTargetPattern.java:31 SignedTargetPattern.Sign
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignedTargetPatternSign {
    Positive,
    Negative,
}

/// A [`TargetPattern`] with a potential minus sign in front of it, signifying exclusion.
// Java: com/google/devtools/build/lib/cmdline/SignedTargetPattern.java:21 SignedTargetPattern
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignedTargetPattern {
    pattern: TargetPattern,
    sign: SignedTargetPatternSign,
}

impl SignedTargetPattern {
    // Java: com/google/devtools/build/lib/cmdline/SignedTargetPattern.java:26 SignedTargetPattern.create
    pub fn create(pattern: TargetPattern, sign: SignedTargetPatternSign) -> Self {
        SignedTargetPattern { pattern, sign }
    }

    pub fn pattern(&self) -> &TargetPattern {
        &self.pattern
    }

    pub fn sign(&self) -> SignedTargetPatternSign {
        self.sign
    }

    // Java: com/google/devtools/build/lib/cmdline/SignedTargetPattern.java:36 SignedTargetPattern.parse
    pub fn parse(
        pattern: &str,
        parser: &TargetPatternParser,
    ) -> Result<SignedTargetPattern, TargetParsingException> {
        if let Some(rest) = pattern.strip_prefix('-') {
            Ok(Self::create(
                parser.parse(rest)?,
                SignedTargetPatternSign::Negative,
            ))
        } else {
            Ok(Self::create(
                parser.parse(pattern)?,
                SignedTargetPatternSign::Positive,
            ))
        }
    }
}
