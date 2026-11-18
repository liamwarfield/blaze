//! `OptionsParsingResult`: the read-only view of a finished `OptionsParser`.
//! PORT: Java's interface is implemented by `OptionsParser`; Blaze uses the parser directly.

pub use blaze_options::OptionsParser as OptionsParsingResult;
