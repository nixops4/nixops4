//! Lenient command-line parsing for extracting options from partial input.
//!
//! During shell completion, we may have a partial command line. This module
//! provides utilities to extract as many options as possible using binary
//! search to find the longest parseable prefix.

use clap::Parser;

/// Parse the longest prefix of `args` that succeeds with the given parser.
///
/// Uses binary search with bounded linear probing. Assumes the parser has
/// at most `MAX_GAP` consecutive invalid stopping points.
///
/// Returns `None` if even an empty argument list fails to parse.
pub fn parse_longest_prefix<P: Parser>(args: &[String]) -> Option<P> {
    heuristic_largest_valid(args.len(), |n| P::try_parse_from(&args[..n]).ok())
}

// Implementation
// --------------
// The purpose of the rest of this file is to create some sort of lenient
// parsing. This is really a clap or *maybe* a clap_complete responsibility,
// so this code is a *WORKAROUND*. Do not gold-plate it any further.
//
// roberth: I suspect clap is geared towards performance instead of elegance and
//          flexibility. Not even Haskell's optparse-applicative gets this right,
//          and I would argue it should be generalized to optparse-profunctor,
//          allowing the context to be provided to completions. Possibly even a
//          third type to generalize the return type for extra leniency.
//          I wonder if those ideas could be implemented and ported to Rust.
//          Or maybe I'm wrong about the clap design and the need for a "context"
//          type parameter, and it's all over-engineered.
//          I guess what I'm trying to say is: the below is kinda stupid, it
//          kinda works, but don't improve it too much because it is very
//          unprincipled, putting a ceiling on what it can ultimately do.

/// Maximum number of consecutive invalid values between valid values.
///
/// The predicate must return `Some` for at least one value within every
/// `MAX_GAP + 1` consecutive integers. This allows O(log n) binary search
/// with O(CHUNK_SIZE) probing per iteration.
const MAX_GAP: usize = 5;

/// Chunk size for binary search. Must be > MAX_GAP to guarantee at least
/// one valid value per chunk in the valid range.
const CHUNK_SIZE: usize = MAX_GAP + 1;

/// Find the largest `n` in `0..=max` where `predicate(n)` returns `Some`.
///
/// Uses binary search on chunks with linear probing within each chunk.
/// Only guaranteed to find the largest valid value when there are at most
/// `MAX_GAP` consecutive invalid values in the range `0..=max`.
///
/// Returns `None` if the predicate never returns `Some`.
fn heuristic_largest_valid<R, F>(max: usize, predicate: F) -> Option<R>
where
    F: Fn(usize) -> Option<R>,
{
    // Fast path: if max is valid, return immediately
    if let Some(result) = predicate(max) {
        return Some(result);
    }

    let num_chunks = (max + CHUNK_SIZE) / CHUNK_SIZE;
    let chunks: Vec<usize> = (0..num_chunks).collect();

    let mut best_n: isize = -1;
    let mut best: Option<R> = None;

    let _ = chunks.partition_point(|&chunk_idx| {
        let start = chunk_idx * CHUNK_SIZE;
        let end = ((chunk_idx + 1) * CHUNK_SIZE).min(max + 1);

        // Scan chunk from high to low to find highest valid in this chunk
        for n in (start..end).rev() {
            if let Some(result) = predicate(n) {
                if (n as isize) > best_n {
                    best_n = n as isize;
                    best = Some(result);
                }
                return true; // chunk has valid value
            }
        }
        false // no valid value in chunk
    });

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test parser with a simple flag
    #[derive(Parser, Debug, PartialEq)]
    #[command(no_binary_name = true)]
    struct SimpleFlags {
        #[arg(short, long)]
        verbose: bool,

        #[arg(short, long)]
        quiet: bool,
    }

    /// Test parser with multi-value argument
    #[derive(Parser, Debug, PartialEq)]
    #[command(no_binary_name = true)]
    struct MultiValue {
        #[arg(long, num_args = 2, action = clap::ArgAction::Append)]
        pair: Vec<String>,
    }

    /// Test parser that accepts trailing arguments
    #[derive(Parser, Debug, PartialEq)]
    #[command(no_binary_name = true)]
    struct WithTrailing {
        #[arg(long)]
        flag: bool,

        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    }

    // Test parser that accepts three-item groups
    #[derive(Parser, Debug, PartialEq)]
    #[command(no_binary_name = true)]
    struct KVArgs {
        #[arg(long, num_args = 2, value_names = &["IDENT", "VALUE"])]
        set_value: Vec<String>,
    }

    /// Test parser mimicking OptionsWrapper: global options + trailing for subcommand
    #[derive(Parser, Debug, PartialEq)]
    #[command(no_binary_name = true)]
    struct OptionsLike {
        #[arg(long, num_args = 2)]
        override_input: Vec<String>,

        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    }

    #[test]
    fn test_override_input_before_subcommand() {
        let args: Vec<String> = vec![
            "--override-input",
            "nixpkgs",
            "NixOS/nixpkgs/nixos-unstable",
            "apply",
            "",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let result: Option<OptionsLike> = parse_longest_prefix(&args);
        assert_eq!(
            result,
            Some(OptionsLike {
                override_input: vec![
                    "nixpkgs".to_string(),
                    "NixOS/nixpkgs/nixos-unstable".to_string()
                ],
                rest: vec!["apply".to_string(), "".to_string()]
            })
        );
    }

    #[test]
    fn test_empty_args() {
        let result: Option<SimpleFlags> = parse_longest_prefix(&[]);
        assert_eq!(
            result,
            Some(SimpleFlags {
                verbose: false,
                quiet: false
            })
        );
    }

    #[test]
    fn test_simple_flags() {
        let args = vec!["--verbose".to_string()];
        let result: Option<SimpleFlags> = parse_longest_prefix(&args);
        assert_eq!(
            result,
            Some(SimpleFlags {
                verbose: true,
                quiet: false
            })
        );
    }

    #[test]
    fn test_multiple_flags() {
        let args = vec!["--verbose".to_string(), "--quiet".to_string()];
        let result: Option<SimpleFlags> = parse_longest_prefix(&args);
        assert_eq!(
            result,
            Some(SimpleFlags {
                verbose: true,
                quiet: true
            })
        );
    }

    #[test]
    fn test_multi_value_complete() {
        let args = vec![
            "--pair".to_string(),
            "key1".to_string(),
            "value1".to_string(),
        ];
        let result: Option<MultiValue> = parse_longest_prefix(&args);
        assert_eq!(
            result,
            Some(MultiValue {
                pair: vec!["key1".to_string(), "value1".to_string()]
            })
        );
    }

    #[test]
    fn test_multi_value_multiple() {
        let args = vec![
            "--pair".to_string(),
            "k1".to_string(),
            "v1".to_string(),
            "--pair".to_string(),
            "k2".to_string(),
            "v2".to_string(),
        ];
        let result: Option<MultiValue> = parse_longest_prefix(&args);
        assert_eq!(
            result,
            Some(MultiValue {
                pair: vec![
                    "k1".to_string(),
                    "v1".to_string(),
                    "k2".to_string(),
                    "v2".to_string()
                ]
            })
        );
    }

    #[test]
    fn test_multi_value_incomplete() {
        // Only one value provided instead of two
        let args = vec!["--pair".to_string(), "key1".to_string()];
        let result: Option<MultiValue> = parse_longest_prefix(&args);
        // Should fall back to parsing empty prefix
        assert_eq!(result, Some(MultiValue { pair: vec![] }));
    }

    #[test]
    fn test_unknown_flag_stops_parsing() {
        let args = vec![
            "--verbose".to_string(),
            "--unknown".to_string(),
            "--quiet".to_string(),
        ];
        let result: Option<SimpleFlags> = parse_longest_prefix(&args);
        // Should parse up to --verbose
        assert_eq!(
            result,
            Some(SimpleFlags {
                verbose: true,
                quiet: false
            })
        );
    }

    #[test]
    fn test_trailing_captures_rest() {
        let args = vec!["--flag".to_string(), "pos1".to_string(), "pos2".to_string()];
        let result: Option<WithTrailing> = parse_longest_prefix(&args);
        assert_eq!(
            result,
            Some(WithTrailing {
                flag: true,
                rest: vec!["pos1".to_string(), "pos2".to_string()]
            })
        );
    }

    #[test]
    fn test_trailing_captures_rest_kv_arg() {
        let args = vec![
            "--set-value".to_string(),
            "pos1".to_string(),
            "pos2".to_string(),
            "--set-value".to_string(),
            "pos3".to_string(),
            "pos4".to_string(),
            "bad-1".to_string(),
            "--set-value".to_string(), // doesn't matter
            "bad-2".to_string(),
            "bad-3".to_string(),
            "bad-4".to_string(),
            "bad-5".to_string(),
            "bad-6".to_string(),
            "bad-7".to_string(),
            "bad-8".to_string(),
            "bad-9".to_string(),
            "bad-10".to_string(),
        ];
        let result: Option<KVArgs> = parse_longest_prefix(&args);
        assert_eq!(
            result,
            Some(KVArgs {
                set_value: vec![
                    "pos1".to_string(),
                    "pos2".to_string(),
                    "pos3".to_string(),
                    "pos4".to_string()
                ]
            })
        );
    }

    // Tests for heuristic_largest_valid algorithm

    #[test]
    fn test_heuristic_none_valid() {
        // No values are valid
        let result: Option<usize> = heuristic_largest_valid(10, |_| None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_heuristic_max_zero_invalid() {
        // Edge case: max = 0, but invalid
        let result: Option<usize> = heuristic_largest_valid(0, |_| None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_heuristic_various_max_values() {
        // Test with max invalid (valid only up to max-1) to exercise binary search
        for max in 1..=50 {
            let result = heuristic_largest_valid(max, |n| if n < max { Some(n) } else { None });
            assert_eq!(result, Some(max - 1), "failed for max={}", max);
        }
    }

    #[test]
    fn test_heuristic_various_thresholds() {
        // Test valid up to threshold (excluding threshold=max to exercise binary search)
        for max in [100, 101, 102, 103, 104, 105] {
            for threshold in 0..max {
                let result =
                    heuristic_largest_valid(max, |n| if n <= threshold { Some(n) } else { None });
                assert_eq!(
                    result,
                    Some(threshold),
                    "failed for max={}, threshold={}",
                    max,
                    threshold
                );
            }
        }
    }

    #[test]
    fn test_heuristic_various_gap_sizes() {
        // Test gaps from 1 to MAX_GAP with various max values around chunk boundaries.
        // `gap` is the number of invalid values between consecutive valid values.
        // With n % (gap + 1) == 0: valid at 0, gap+1, 2*(gap+1), ...
        // e.g., gap=5 gives valid at 0, 6, 12, ... with 5 invalid values between each.
        for gap in 1..=MAX_GAP {
            for max in [50, 51, 52, 53, 54, 55] {
                let spacing = gap + 1;
                let result =
                    heuristic_largest_valid(max, |n| if n % spacing == 0 { Some(n) } else { None });
                let expected = (max / spacing) * spacing;
                assert_eq!(
                    result,
                    Some(expected),
                    "failed for gap={}, max={}",
                    gap,
                    max
                );
            }
        }
    }

    #[test]
    fn test_heuristic_single_valid_at_each_position() {
        // Test cutoff at each position (excluding cutoff=max to exercise binary search)
        for max in [30, 31, 32, 33, 34, 35] {
            for cutoff in 0..max {
                let result =
                    heuristic_largest_valid(max, |n| if n <= cutoff { Some(n) } else { None });
                assert_eq!(
                    result,
                    Some(cutoff),
                    "failed for max={}, cutoff={}",
                    max,
                    cutoff
                );
            }
        }
    }

    #[test]
    fn test_heuristic_returns_correct_result_value() {
        // Verify we return the result from the predicate (with max invalid)
        for max in [5, 6, 7, 10, 11, 12, 20, 50, 51, 52] {
            let result = heuristic_largest_valid(max, |n| if n < max { Some(n * 2) } else { None });
            assert_eq!(result, Some((max - 1) * 2), "failed for max={}", max);
        }
    }

    #[test]
    fn test_heuristic_empty_suffix() {
        // Valid from 0 to various cutoff points (excluding cutoff=max)
        for max in [10, 11, 12, 25, 50, 51, 100, 101, 102] {
            for cutoff in [0, 1, max / 4, max / 2, max - 1] {
                let result =
                    heuristic_largest_valid(max, |n| if n <= cutoff { Some(n) } else { None });
                assert_eq!(
                    result,
                    Some(cutoff),
                    "failed for max={}, cutoff={}",
                    max,
                    cutoff
                );
            }
        }
    }

    #[test]
    fn test_heuristic_max_valid_single_call() {
        use std::cell::Cell;

        // Whenever max is valid, it should be found in exactly 1 predicate call
        for max in [
            0, 1, 5, 6, 7, 10, 11, 12, 13, 14, 15, 20, 50, 51, 100, 101, 102, 103, 104, 105, 200,
            500, 1000,
        ] {
            let count = Cell::new(0usize);
            let result = heuristic_largest_valid(max, |n| {
                count.set(count.get() + 1);
                Some(n)
            });
            assert_eq!(result, Some(max), "wrong result for max={}", max);
            assert_eq!(
                count.get(),
                1,
                "expected 1 call for max={}, got {}",
                max,
                count.get()
            );
        }
    }

    #[test]
    fn test_heuristic_predicate_call_count() {
        use std::cell::Cell;

        // Count predicate calls with max invalid to exercise binary search
        // Include values around chunk boundaries (CHUNK_SIZE = 6)
        for max in [
            10, 11, 12, 13, 14, 15, 20, 50, 100, 101, 102, 103, 104, 105, 200, 500, 1000,
        ] {
            let threshold = max / 2;
            let count = Cell::new(0usize);
            let result = heuristic_largest_valid(max, |n| {
                count.set(count.get() + 1);
                if n <= threshold {
                    Some(n)
                } else {
                    None
                }
            });
            assert_eq!(result, Some(threshold));

            let num_chunks = (max + CHUNK_SIZE) / CHUNK_SIZE;
            // Binary search visits O(log num_chunks) chunks
            // Each chunk scans up to CHUNK_SIZE values
            let expected_max_calls = ((num_chunks as f64).log2().ceil() as usize + 1) * CHUNK_SIZE;

            println!(
                "max={:4}, chunks={:3}, calls={:4}, expected_max={:4}",
                max,
                num_chunks,
                count.get(),
                expected_max_calls
            );

            assert!(
                count.get() <= expected_max_calls,
                "Too many calls for max={}: got {}, expected <= {}",
                max,
                count.get(),
                expected_max_calls
            );
        }
    }
}
