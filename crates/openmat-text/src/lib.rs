#![doc = "Provider-neutral text algorithms used by `OpenMat` language built-ins."]
#![forbid(unsafe_code)]

use std::{error::Error, fmt};

use fancy_regex::RegexBuilder;

/// A zero-based, half-open range measured in UTF-16 code units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Utf16Range {
    /// First included UTF-16 code-unit offset.
    pub start: u64,
    /// First excluded UTF-16 code-unit offset.
    pub end: u64,
}

/// One complete regular-expression match with optional capture ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexMatch {
    /// Range of capture group zero.
    pub full: Utf16Range,
    /// Capture groups one through N in declaration order.
    pub captures: Vec<Option<Utf16Range>>,
}

/// Provider-neutral result of compiling and executing one expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexMatches {
    /// Optional capture names for groups one through N.
    pub capture_names: Vec<Option<String>>,
    /// Non-overlapping matches in source order.
    pub matches: Vec<RegexMatch>,
}

/// Matching flags owned by `OpenMat` rather than a backend crate.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegexOptions {
    /// Match ASCII and Unicode letters without case distinctions.
    pub case_insensitive: bool,
    /// Allow dot to match newline code points.
    pub dot_matches_new_line: bool,
    /// Make line anchors operate at embedded line boundaries.
    pub multi_line: bool,
    /// Ignore unescaped pattern whitespace and comments.
    pub ignore_whitespace: bool,
    /// Bound backend backtracking work for non-linear features.
    pub backtrack_limit: usize,
}

impl Default for RegexOptions {
    fn default() -> Self {
        Self {
            case_insensitive: false,
            dot_matches_new_line: false,
            multi_line: false,
            ignore_whitespace: false,
            backtrack_limit: 1_000_000,
        }
    }
}

/// Stable failure classes exposed by a regular-expression provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegexErrorKind {
    /// Pattern text is not valid UTF-16.
    InvalidPatternText,
    /// Input text is not valid UTF-16.
    InvalidInputText,
    /// The selected backend rejected the expression grammar.
    InvalidPattern,
    /// Matching exceeded the configured bounded backtracking work.
    BacktrackLimit,
}

/// Provider-independent regular-expression failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegexError {
    /// Stable failure class; backend diagnostics are deliberately not retained.
    pub kind: RegexErrorKind,
}

impl fmt::Display for RegexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            RegexErrorKind::InvalidPatternText => "regular-expression pattern is invalid UTF-16",
            RegexErrorKind::InvalidInputText => "regular-expression input is invalid UTF-16",
            RegexErrorKind::InvalidPattern => "regular-expression pattern is invalid",
            RegexErrorKind::BacktrackLimit => "regular-expression work limit was exceeded",
        })
    }
}

impl Error for RegexError {}

/// Replaceable backend contract for matching exact UTF-16 text.
pub trait RegexProvider: Send + Sync {
    /// Executes one expression and returns UTF-16 ranges only.
    ///
    /// # Errors
    ///
    /// Returns a stable input, pattern, or bounded-execution error.
    fn find_all(
        &self,
        pattern: &[u16],
        input: &[u16],
        options: RegexOptions,
    ) -> Result<RegexMatches, RegexError>;
}

/// Default memory-safe backend based on `fancy-regex`.
#[derive(Clone, Copy, Debug, Default)]
pub struct FancyRegexProvider;

impl RegexProvider for FancyRegexProvider {
    fn find_all(
        &self,
        pattern: &[u16],
        input: &[u16],
        options: RegexOptions,
    ) -> Result<RegexMatches, RegexError> {
        let pattern = String::from_utf16(pattern).map_err(|_| RegexError {
            kind: RegexErrorKind::InvalidPatternText,
        })?;
        let input = String::from_utf16(input).map_err(|_| RegexError {
            kind: RegexErrorKind::InvalidInputText,
        })?;
        let mut builder = RegexBuilder::new(&pattern);
        builder
            .case_insensitive(options.case_insensitive)
            .dot_matches_new_line(options.dot_matches_new_line)
            .multi_line(options.multi_line)
            .ignore_whitespace(options.ignore_whitespace)
            .backtrack_limit(options.backtrack_limit);
        let expression = builder.build().map_err(|_| RegexError {
            kind: RegexErrorKind::InvalidPattern,
        })?;
        let offsets = utf16_offsets(&input);
        let capture_names = expression
            .capture_names()
            .skip(1)
            .map(|name| name.map(str::to_owned))
            .collect::<Vec<_>>();
        let mut matches = Vec::new();
        for captures in expression.captures_iter(&input) {
            let captures = captures.map_err(|_| RegexError {
                kind: RegexErrorKind::BacktrackLimit,
            })?;
            let full = captures.get(0).ok_or(RegexError {
                kind: RegexErrorKind::BacktrackLimit,
            })?;
            let full = mapped_range(&offsets, full.start(), full.end())?;
            let groups = (1..captures.len())
                .map(|index| {
                    captures
                        .get(index)
                        .map(|value| mapped_range(&offsets, value.start(), value.end()))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            matches.push(RegexMatch {
                full,
                captures: groups,
            });
        }
        Ok(RegexMatches {
            capture_names,
            matches,
        })
    }
}

fn utf16_offsets(value: &str) -> Vec<Option<u64>> {
    let mut offsets = vec![None; value.len() + 1];
    let mut utf16 = 0_u64;
    for (byte, character) in value.char_indices() {
        offsets[byte] = Some(utf16);
        utf16 += u64::try_from(character.len_utf16()).expect("char UTF-16 length fits u64");
    }
    offsets[value.len()] = Some(utf16);
    offsets
}

fn mapped_range(
    offsets: &[Option<u64>],
    start: usize,
    end: usize,
) -> Result<Utf16Range, RegexError> {
    let start = offsets.get(start).copied().flatten().ok_or(RegexError {
        kind: RegexErrorKind::BacktrackLimit,
    })?;
    let end = offsets.get(end).copied().flatten().ok_or(RegexError {
        kind: RegexErrorKind::BacktrackLimit,
    })?;
    Ok(Utf16Range { start, end })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_returns_utf16_ranges_names_lookaround_and_backreferences() {
        let provider = FancyRegexProvider;
        let pattern = r"(?<=b)(?<pair>(\d)\2)".encode_utf16().collect::<Vec<_>>();
        let input = "a🙂b22 c33".encode_utf16().collect::<Vec<_>>();
        let result = provider
            .find_all(&pattern, &input, RegexOptions::default())
            .unwrap();
        assert_eq!(result.capture_names, [Some("pair".to_owned()), None]);
        assert_eq!(
            result.matches,
            [RegexMatch {
                full: Utf16Range { start: 4, end: 6 },
                captures: vec![
                    Some(Utf16Range { start: 4, end: 6 }),
                    Some(Utf16Range { start: 4, end: 5 }),
                ],
            }]
        );
    }

    #[test]
    fn provider_rejects_invalid_utf16_and_invalid_patterns_without_backend_types() {
        let provider = FancyRegexProvider;
        assert_eq!(
            provider.find_all(&[0xd800], &[], RegexOptions::default()),
            Err(RegexError {
                kind: RegexErrorKind::InvalidPatternText,
            })
        );
        assert_eq!(
            provider.find_all(&[u16::from(b'.')], &[0xd800], RegexOptions::default()),
            Err(RegexError {
                kind: RegexErrorKind::InvalidInputText,
            })
        );
        assert_eq!(
            provider.find_all(&[u16::from(b'(')], &[], RegexOptions::default()),
            Err(RegexError {
                kind: RegexErrorKind::InvalidPattern,
            })
        );
    }
}
