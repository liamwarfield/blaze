//! The JDK's line-instance `java.text.BreakIterator`, as `OptionsUsage.paragraphFill` uses it
//! (`BreakIterator.getLineInstance()`, then `first()` and `next()` until `DONE`).
//!
//! PORT: the JDK builds a state table from the rule text `LineBreakRules` of
//! `sun/text/resources/BreakIteratorRules.java` (JDK 25, the JDK Bazel 9.3.2 embeds; the rules
//! are unchanged since JDK 1.4) and `sun.text.RuleBasedBreakIterator.handleNext` runs it: from the
//! current position it follows the table until no continuation is possible and breaks after the
//! last position at which a rule was fully matched (at least one character). Blaze evaluates the
//! same rules directly, as a longest match of the rule expressions over character classes, which
//! gives the same boundaries; `scripts/cmds/java-line-breaks` checks it against the JDK.
//!
//! The rules, with Blaze's names for their character sets:
//!
//! ```text
//! <break>=[\u0003\t\n\f  ];
//! <ignore>=[:Cf:[:Cc:^[<break>\r]]];
//! <enclosing>=[:Mn::Me:];
//! <danda>=[।॥];
//! <glue>=[ ༌ ‑ ﻿];
//! <space>=[:Zs::Cc:^[<glue><break>\r]];
//! <dash>=[:Pd:­^<glue>];
//! <pre-word>=[:Sc::Ps::Pi:^[¢]\"\'];
//! <post-word>=[\":Pe::Pf:\!\%\.\,\:\;\?¢°٪‰-‴℃℅℉、
//!   。々ぁぃぅぇぉっゃゅょゎ゙-ゞ
//!   ァィゥェォッャュョヮヵヶー-ヾ
//!   ！％，．：；？];
//! <kanji>=[一-龥가-힣豈-鶴侮-頻ぁ-ゔァ-ヺ
//!   ^[<post-word><ignore>]];
//! <digit>=[:Nd::No:];
//! <mid-num>=[\.\,];
//! <char>=[^[<break><space><dash><kanji><glue><ignore><pre-word><post-word><mid-num>\r<danda>]];
//! <number>=([<pre-word><dash>]*<digit><digit>*(<mid-num><digit><digit>*)*);
//! <word-core>=(<char>*|<kanji>|<number>);
//! <word-suffix>=((<dash><dash>*|<post-word>*));
//! <word>=(<pre-word>*<word-core><word-suffix>);
//! <hack1>=[\(]; <hack2>=[\)]; <hack3>=[\$\'];
//! <word>(((<space>*<glue><glue>*{<space>})|<hack3>)<word>)*<space>*{<enclosing>*}
//!   {<hack1><hack2><post-word>*}{<enclosing>*}{\r}{<break>};
//! \r<break>;
//! ```
//!
//! In the rule language `^` inside a set subtracts the one set that follows it, `{x}` is optional, and
//! characters of `<ignore>` never change the state (they stay with the preceding character).

/// The Unicode general categories the rules mention. Other categories are `Other`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Category {
    Cc,
    Cf,
    Zs,
    Pd,
    Ps,
    Pe,
    Pi,
    Pf,
    Sc,
    Nd,
    No,
    // Combining marks (U+0300 and up) are outside the characters `category` knows, which fail
    // loudly; the variants keep `<enclosing>` written as the rules state it.
    #[allow(dead_code)]
    Mn,
    #[allow(dead_code)]
    Me,
    Other,
}

/// `Character.getType(c)`, reduced to the categories the rules use, for the characters whose
/// category Blaze knows (Latin-1, General Punctuation and the characters the rules list).
fn category(c: char) -> Category {
    use Category::*;
    let u = c as u32;
    match u {
        0x00..=0x1f | 0x7f..=0x9f => Cc,
        0x20 | 0xa0 => Zs,
        0x2d => Pd,
        0x28 | 0x5b | 0x7b => Ps,
        0x29 | 0x5d | 0x7d => Pe,
        0xab => Pi,
        0xbb => Pf,
        0x24 | 0xa2..=0xa5 => Sc,
        0x30..=0x39 => Nd,
        0xb2 | 0xb3 | 0xb9 | 0xbc..=0xbe => No,
        0xad => Cf,
        0x21..=0xff => Other,
        // General Punctuation.
        0x2000..=0x200a => Zs,
        0x200b..=0x200f => Cf,
        0x2010..=0x2015 => Pd,
        0x2016 | 0x2017 => Other, // Po
        0x2018 | 0x201b | 0x201c | 0x201f => Pi,
        0x2019 | 0x201d => Pf,
        0x201a | 0x201e => Ps,
        0x2020..=0x2027 => Other, // Po
        0x2028 | 0x2029 => Other, // Zl, Zp
        0x202a..=0x202e => Cf,
        0x202f => Zs,
        0x2030..=0x2038 => Other, // Po
        0x2039 => Pi,
        0x203a => Pf,
        0x203b..=0x2044 => Other, // Po, Pc, Sm
        0x2045 => Ps,
        0x2046 => Pe,
        0x2047..=0x205e => Other, // Po, Sm, Pc
        0x205f => Zs,
        0x2060..=0x206f => Cf, // Cf (U+2065 is unassigned)
        0x20a0..=0x20c0 => Sc, // Currency Symbols
        0x2103 | 0x2105 | 0x2109 => Other,
        0x0964 | 0x0965 | 0x066a | 0x0f0c => Other,
        0x3001 | 0x3002 | 0x3005 => Other,
        0xfeff => Cf,
        0xff01 | 0xff05 | 0xff0c | 0xff0e | 0xff1a | 0xff1b | 0xff1f => Other,
        _ => blaze_util::unported!(
            "BreakIterator.getLineInstance",
            "Unicode general category of U+{:04X}",
            u
        ),
    }
}

fn is_break(c: char) -> bool {
    matches!(c, '\u{3}' | '\t' | '\n' | '\u{c}' | '\u{2028}' | '\u{2029}')
}

fn is_ignore(c: char) -> bool {
    let cat = category(c);
    // PORT: a character of <ignore> that another set of the rules also names (control
    // characters are <space>, U+00AD is <dash>, U+FEFF is <glue>) takes part in the rules; only
    // the rest is transparent. Recorded from the JDK (tests/data/line_breaks.tsv).
    (cat == Category::Cf || (cat == Category::Cc && !is_break(c) && c != '\r'))
        && !is_space(c)
        && !is_dash(c)
        && !is_glue(c)
}

fn is_enclosing(c: char) -> bool {
    matches!(category(c), Category::Mn | Category::Me)
}

fn is_danda(c: char) -> bool {
    matches!(c, '\u{964}' | '\u{965}')
}

fn is_glue(c: char) -> bool {
    matches!(
        c,
        '\u{a0}' | '\u{f0c}' | '\u{2007}' | '\u{2011}' | '\u{202f}' | '\u{feff}'
    )
}

fn is_space(c: char) -> bool {
    matches!(category(c), Category::Zs | Category::Cc) && !is_glue(c) && !is_break(c) && c != '\r'
}

fn is_dash(c: char) -> bool {
    (category(c) == Category::Pd || c == '\u{ad}') && !is_glue(c)
}

fn is_pre_word(c: char) -> bool {
    // `^` subtracts only the set that follows it (`[\u00a2]`); the quotes are added after.
    (matches!(category(c), Category::Sc | Category::Ps | Category::Pi) && c != '\u{a2}')
        || matches!(c, '"' | '\'')
}

fn is_post_word(c: char) -> bool {
    matches!(category(c), Category::Pe | Category::Pf)
        || matches!(
            c,
            '"' | '!'
                | '%'
                | '.'
                | ','
                | ':'
                | ';'
                | '?'
                | '\u{a2}'
                | '\u{b0}'
                | '\u{66a}'
                | '\u{2030}'..='\u{2034}'
                | '\u{2103}'
                | '\u{2105}'
                | '\u{2109}'
                | '\u{3001}'
                | '\u{3002}'
                | '\u{3005}'
                | '\u{3041}'
                | '\u{3043}'
                | '\u{3045}'
                | '\u{3047}'
                | '\u{3049}'
                | '\u{3063}'
                | '\u{3083}'
                | '\u{3085}'
                | '\u{3087}'
                | '\u{308e}'
                | '\u{3099}'..='\u{309e}'
                | '\u{30a1}'
                | '\u{30a3}'
                | '\u{30a5}'
                | '\u{30a7}'
                | '\u{30a9}'
                | '\u{30c3}'
                | '\u{30e3}'
                | '\u{30e5}'
                | '\u{30e7}'
                | '\u{30ee}'
                | '\u{30f5}'
                | '\u{30f6}'
                | '\u{30fc}'..='\u{30fe}'
                | '\u{ff01}'
                | '\u{ff05}'
                | '\u{ff0c}'
                | '\u{ff0e}'
                | '\u{ff1a}'
                | '\u{ff1b}'
                | '\u{ff1f}'
        )
}

fn is_kanji(c: char) -> bool {
    matches!(
        c,
        '\u{4e00}'..='\u{9fa5}'
            | '\u{ac00}'..='\u{d7a3}'
            | '\u{f900}'..='\u{fa2d}'
            | '\u{fa30}'..='\u{fa6a}'
            | '\u{3041}'..='\u{3094}'
            | '\u{30a1}'..='\u{30fa}'
    ) && !is_post_word(c)
        && !is_ignore(c)
}

fn is_digit(c: char) -> bool {
    matches!(category(c), Category::Nd | Category::No)
}

fn is_mid_num(c: char) -> bool {
    matches!(c, '.' | ',')
}

fn is_char(c: char) -> bool {
    !(is_break(c)
        || is_space(c)
        || is_dash(c)
        || is_kanji(c)
        || is_glue(c)
        || is_ignore(c)
        || is_pre_word(c)
        || is_post_word(c)
        || is_mid_num(c)
        || c == '\r'
        || is_danda(c))
}

fn is_pre_word_or_dash(c: char) -> bool {
    is_pre_word(c) || is_dash(c)
}

/// A set of positions in the (ignore-free) character sequence, sorted and without duplicates.
type Positions = Vec<usize>;

struct Matcher<'a> {
    chars: &'a [char],
}

impl Matcher<'_> {
    fn union(a: &Positions, b: &Positions) -> Positions {
        let mut out: Positions = a.iter().chain(b.iter()).copied().collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// One character of the class, from each position.
    fn class(&self, from: &Positions, pred: fn(char) -> bool) -> Positions {
        from.iter()
            .filter(|&&i| i < self.chars.len() && pred(self.chars[i]))
            .map(|&i| i + 1)
            .collect()
    }

    /// `f*` from each position.
    fn star(&self, from: &Positions, f: &dyn Fn(&Self, &Positions) -> Positions) -> Positions {
        let mut result = from.clone();
        let mut frontier = from.clone();
        loop {
            let next = f(self, &frontier);
            let new: Positions = next
                .into_iter()
                .filter(|p| result.binary_search(p).is_err())
                .collect();
            if new.is_empty() {
                return result;
            }
            result = Self::union(&result, &new);
            frontier = new;
        }
    }

    fn class_star(&self, from: &Positions, pred: fn(char) -> bool) -> Positions {
        self.star(from, &|m, s| m.class(s, pred))
    }

    /// `x x*`.
    fn class_plus(&self, from: &Positions, pred: fn(char) -> bool) -> Positions {
        let once = self.class(from, pred);
        self.class_star(&once, pred)
    }

    /// `<number>`.
    fn number(&self, from: &Positions) -> Positions {
        let prefix = self.class_star(from, is_pre_word_or_dash);
        let digits = self.class_plus(&prefix, is_digit);
        self.star(&digits, &|m, s| {
            let mid = m.class(s, is_mid_num);
            m.class_plus(&mid, is_digit)
        })
    }

    /// `<word>`.
    fn word(&self, from: &Positions) -> Positions {
        let pre = self.class_star(from, is_pre_word);
        let core = Self::union(
            &Self::union(&self.class_star(&pre, is_char), &self.class(&pre, is_kanji)),
            &self.number(&pre),
        );
        Self::union(
            &self.class_plus(&core, is_dash),
            &self.class_star(&core, is_post_word),
        )
    }

    /// The end positions of both rules from `start`.
    fn rules(&self, start: usize) -> Positions {
        let first = self.word(&vec![start]);
        // (((<space>*<glue><glue>*{<space>})|<hack3>)<word>)*
        let words = self.star(&first, &|m, s| {
            let spaces = m.class_star(s, is_space);
            let glue = m.class_plus(&spaces, is_glue);
            let glue = Self::union(&glue, &m.class(&glue, is_space));
            let joiner = Self::union(&glue, &m.class(s, |c| matches!(c, '$' | '\'')));
            m.word(&joiner)
        });
        let mut t = self.class_star(&words, is_space);
        t = self.class_star(&t, is_enclosing);
        let hack = self.class(&self.class(&t, |c| c == '('), |c| c == ')');
        t = Self::union(&t, &self.class_star(&hack, is_post_word));
        t = self.class_star(&t, is_enclosing);
        t = Self::union(&t, &self.class(&t, |c| c == '\r'));
        t = Self::union(&t, &self.class(&t, is_break));
        // \r<break>
        let crlf = self.class(&self.class(&vec![start], |c| c == '\r'), is_break);
        Self::union(&t, &crlf)
    }
}

/// The boundaries `first()`, `next()`, ... of a line instance over `text` (its UTF-16 offsets,
/// as Java reports them), ending with the text's length; `[0]` for an empty text.
// Java: java/text/BreakIterator.java BreakIterator.getLineInstance
// Java: sun/text/RuleBasedBreakIterator.java:876 RuleBasedBreakIterator.handleNext
pub(crate) fn line_boundaries(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    // The characters that take part in the rules, with their index in `chars`.
    let mut filtered: Vec<char> = Vec::new();
    let mut original_index: Vec<usize> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if !is_ignore(c) {
            filtered.push(c);
            original_index.push(i);
        }
    }
    let matcher = Matcher { chars: &filtered };

    // Offsets in chars; converted to UTF-16 offsets at the end.
    let mut boundaries = vec![0usize];
    let mut pos = 0usize;
    while pos < chars.len() {
        // no matter what, we always advance at least one character forward
        let mut result = pos + 1;
        let fstart = original_index.partition_point(|&i| i < pos);
        if fstart < filtered.len() {
            let end = *matcher
                .rules(fstart)
                .last()
                .expect("the rules always match the empty word");
            if end > fstart {
                // The break follows the last matched character and the ignore characters right
                // after it (the state does not change on them, so it stays accepting).
                let mut after = original_index[end - 1] + 1;
                while after < chars.len() && is_ignore(chars[after]) {
                    after += 1;
                }
                result = result.max(after);
            }
        }
        boundaries.push(result);
        pos = result;
    }
    // UTF-16 offsets.
    let mut utf16_offset = Vec::with_capacity(chars.len() + 1);
    let mut acc = 0usize;
    for c in &chars {
        utf16_offset.push(acc);
        acc += c.len_utf16();
    }
    utf16_offset.push(acc);
    boundaries.into_iter().map(|b| utf16_offset[b]).collect()
}

/// The segments between consecutive boundaries of [`line_boundaries`].
pub(crate) fn line_segments(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut utf16_to_byte = std::collections::HashMap::new();
    let mut acc = 0usize;
    for &(b, c) in &chars {
        utf16_to_byte.insert(acc, b);
        acc += c.len_utf16();
    }
    utf16_to_byte.insert(acc, text.len());
    let boundaries = line_boundaries(text);
    boundaries
        .windows(2)
        .map(|w| &text[utf16_to_byte[&w[0]]..utf16_to_byte[&w[1]]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_keep_trailing_space() {
        assert_eq!(
            line_segments("Additional places to search"),
            vec!["Additional ", "places ", "to ", "search"]
        );
    }

    #[test]
    fn dashes_end_a_word() {
        // From `bazel help clean --long` (Bazel 9.3.2): `{--` then `repository_cache}`.
        assert_eq!(
            line_segments("`{--repository_cache}/contents`"),
            vec!["`", "{--", "repository_cache}", "/contents`"]
        );
    }

    #[test]
    fn empty_text_has_only_the_start() {
        assert_eq!(line_boundaries(""), vec![0]);
    }

    /// The escapes of `scripts/cmds/LineBreaks.java`.
    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some('u') => {
                    let hex: String = it.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                }
                Some(other) => out.push_str(&format!("\\{other}")),
                None => out.push('\\'),
            }
        }
        out
    }

    /// Expected boundaries recorded from the JDK (`scripts/cmds/java-line-breaks`, JDK 25).
    #[test]
    fn boundaries_match_the_jdk() {
        let tsv = include_str!("../tests/data/line_breaks.tsv");
        let mut failures = Vec::new();
        for line in tsv.lines() {
            let (expected, case) = line.split_once('\t').expect("b0,b1,...<TAB>case");
            let expected: Vec<usize> = expected.split(',').map(|b| b.parse().unwrap()).collect();
            let actual = line_boundaries(&unescape(case));
            if actual != expected {
                failures.push(format!(
                    "{case:?}\n  jdk:   {expected:?}\n  blaze: {actual:?}"
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
