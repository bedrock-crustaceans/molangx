//! Number and `true` / `false` literals, and the unknown-token failure.

use super::{Lexeme, Lexer, NameChars, TokenFailed};
use crate::compile::ast::Payload;
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;

/// The f32 powers of ten a fraction of up to eight digits is divided by.
const POW10: [f32; 9] = [1.0, 10.0, 100.0, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8];

/// `a` to the integer power `b` by repeated squaring: the only `f64` step of literal reading.
fn powi(mut a: f64, mut b: i32) -> f64 {
    let recip = b < 0;
    let mut r = 1.0;
    loop {
        if b & 1 != 0 {
            r *= a;
        }
        b /= 2;
        if b == 0 {
            break;
        }
        a *= a;
    }
    if recip { 1.0 / r } else { r }
}

/// `number·10^exponent`, the product in `f64`.
fn scaled(number: f32, exponent: i32) -> f32 {
    let product = powi(10.0, exponent) * f64::from(number);
    // `0e400` is ∞·0: the invalid-operation NaN of the build's arithmetic.
    if product.is_nan() {
        crate::numeric::arith::mul(f32::INFINITY, 0.0)
    } else {
        product as f32
    }
}

impl Lexer<'_, '_> {
    /// The last step of reading a token: a number, `true` / `false`, or an unknown token. `partial`
    /// is the length of a discarded operator match (a namespace prefix without a name).
    pub(super) fn read_literal(
        &mut self,
        start: usize,
        partial: usize,
    ) -> Result<Lexeme, TokenFailed> {
        let first = self.at(start);
        if first == b'.' || first.is_ascii_digit() {
            let (value, end) = self.scan_number(start);
            // A number no longer than the discarded match (`.` of `. 1`) is no token.
            if end - start <= partial {
                return Err(TokenFailed);
            }
            return Ok(float(value, end));
        }
        let word_end = self.scan_name(start, NameChars::Plain).ok_or(TokenFailed)?;
        // Any prefix of the word matches (`t`, `tr`, `fals`).
        let word = &self.text[start..word_end];
        let (value, word_len) = if b"true".starts_with(word) {
            (1.0, 4)
        } else if b"false".starts_with(word) {
            (0.0, 5)
        } else {
            let span = self.span(start, word_end);
            self.cx
                .language_rest(Msg::UnknownToken, span, start, self.text);
            return Err(TokenFailed);
        };
        // With the deviation off a prefix advances as far as the full word (`t;` loses its `;`),
        // capped at the end of the input.
        let end = if self.cx.opts.deviations.true_false_prefix_advance {
            word_end
        } else {
            (start + word_len).min(self.text.len())
        };
        Ok(float(value, end))
    }

    /// The number starting at `start` (a digit or a `.`) and where it ends. A malformed exponent
    /// is logged, not a failure: the number becomes 0 and ends after the `e`.
    fn scan_number(&mut self, start: usize) -> (f32, usize) {
        // The integer part wraps as an `i32`.
        let (int, mut p) = self.wrapping_digits(start);
        let mut number = int as f32;
        if self.at(p) == b'.' {
            p += 1;
            // Only the first eight fraction digits count; the rest are consumed.
            let mut fraction: i32 = 0;
            let mut digits = 0;
            while self.at(p).is_ascii_digit() {
                if digits < 8 {
                    fraction = fraction * 10 + i32::from(self.at(p) - b'0');
                    digits += 1;
                }
                p += 1;
            }
            number += fraction as f32 / POW10[digits];
        }
        if self.at(p) == b'e' {
            let sign = self.at(p + 1);
            let digits_at = match sign {
                b'+' | b'-' => p + 2,
                b'0'..=b'9' => p + 1,
                _ => {
                    let span = self.span(start, p + 1);
                    self.cx
                        .language_rest(Msg::BadExponent, span, p + 1, self.text);
                    return (0.0, p + 1);
                }
            };
            let (exponent, end) = self.wrapping_digits(digits_at);
            p = end;
            let exponent = if sign == b'-' {
                exponent.wrapping_neg()
            } else {
                exponent
            };
            number = scaled(number, exponent);
        }
        if self.at(p) | 0x20 == b'f' {
            p += 1;
        }
        (number, p)
    }

    /// The decimal digits from `start` as an `i32` that wraps, and where they end.
    fn wrapping_digits(&self, start: usize) -> (i32, usize) {
        let mut value: i32 = 0;
        let mut p = start;
        while self.at(p).is_ascii_digit() {
            value = value
                .wrapping_mul(10)
                .wrapping_add(i32::from(self.at(p) - b'0'));
            p += 1;
        }
        (value, p)
    }
}

fn float(value: f32, end: usize) -> Lexeme {
    Lexeme {
        op: Op::Float,
        value: Payload::Float(value),
        end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{CompileOptions, Deviations, ast::Payload, lex::test_support::*};
    use crate::diag::Severity;
    use proptest::prelude::*;

    #[test]
    fn a_zero_scaled_by_an_infinite_power_is_the_invalid_operation_nan() {
        for source in ["0e400", "0.0e999", "0e400f"] {
            assert_eq!(
                float_bits(source),
                crate::numeric::test_support::per_arch(0xffc0_0000, 0x7fc0_0000),
                "{source}"
            );
        }
    }

    #[test]
    fn powi_small_exponents() {
        assert_eq!(powi(10.0, 0), 1.0);
        assert_eq!(powi(10.0, 1), 10.0);
        assert_eq!(powi(10.0, 2), 100.0);
        assert_eq!(powi(10.0, 3), 1000.0);
        assert_eq!(powi(2.0, 10), 1024.0);
        assert_eq!(powi(-2.0, 3), -8.0);
        assert_eq!(powi(0.0, 0), 1.0);
    }

    #[test]
    fn powi_negative_exponent_is_the_reciprocal_of_the_positive_power() {
        assert_eq!(powi(10.0, -2), 1.0 / 100.0);
        assert_eq!(powi(10.0, -3), 1.0 / 1000.0);
        assert_eq!(powi(2.0, -1), 0.5);
        assert_eq!(powi(0.0, -1), f64::INFINITY);
    }

    #[test]
    fn powi_extreme_exponents() {
        assert_eq!(powi(10.0, 400), f64::INFINITY);
        assert_eq!(powi(10.0, i32::MAX), f64::INFINITY);
        assert_eq!(powi(10.0, i32::MIN), 0.0);
        assert_eq!(powi(1.0, i32::MIN), 1.0);
        assert_eq!(powi(10.0, -400), 0.0);
    }

    #[test]
    fn integers_are_exact_floats() {
        assert_eq!(float_bits("0"), f(0.0));
        assert_eq!(float_bits("7"), f(7.0));
        assert_eq!(float_bits("123456"), f(123_456.0));
        assert_eq!(toks("42")[0].2, (0, 2));
    }

    #[test]
    fn leading_zeros_are_decimal() {
        assert_eq!(float_bits("007"), f(7.0));
        assert_eq!(float_bits("0010.5"), f(10.5));
        assert_eq!(float_bits("00"), f(0.0));
    }

    #[test]
    fn the_integer_part_wraps_as_a_signed_32_bit_integer() {
        assert_eq!(float_bits("2147483647"), f(2_147_483_647_i32 as f32));
        assert_eq!(float_bits("2147483648"), f(-2_147_483_648.0));
        assert_eq!(float_bits("3000000000"), f(-1_294_967_296.0));
        assert_eq!(float_bits("4294967295"), f(-1.0));
        assert_eq!(float_bits("4294967296"), f(0.0));
        assert_eq!(float_bits("99999999999"), f(1_215_752_191_i32 as f32));
        assert_eq!(float_bits("4294967297"), f(1.0));
    }

    #[test]
    fn a_fraction_is_an_integer_divided_by_a_power_of_ten_in_f32() {
        assert_eq!(float_bits("0.1"), f(1.0 / 10.0));
        assert_eq!(float_bits("0.25"), f(0.25));
        assert_eq!(float_bits("1.5"), f(1.5));
        assert_eq!(float_bits("123.456"), f(123.0 + 456.0f32 / 1000.0));
        assert_eq!(float_bits("0.00000001"), f(1.0 / 1e8));
    }

    #[test]
    fn only_the_first_eight_fraction_digits_count() {
        assert_eq!(float_bits("0.000000001"), f(0.0));
        assert_eq!(float_bits("1.123456789"), f(1.0 + 12_345_678.0f32 / 1e8));
        assert_eq!(float_bits("1.123456780000"), float_bits("1.12345678"));
        assert_eq!(float_bits("0.123456789999"), f(12_345_678.0f32 / 1e8));
        // All the digits are consumed.
        assert_eq!(toks("1.123456789")[0].2, (0, 11));
    }

    #[test]
    fn a_trailing_dot_and_a_leading_dot_are_numbers() {
        assert_eq!(float_bits("5."), f(5.0));
        assert_eq!(toks("5.")[0].2, (0, 2));
        assert_eq!(float_bits(".5"), f(0.5));
        assert_eq!(float_bits(".25"), f(0.25));
    }

    #[test]
    fn a_second_dot_starts_a_new_number() {
        let t = toks("1.5.5");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0], (Op::Float, Payload::Float(1.5), (0, 3)));
        assert_eq!(t[1], (Op::Float, Payload::Float(0.5), (3, 5)));
    }

    #[test]
    fn exponents_scale_by_a_power_of_ten_computed_in_f64() {
        assert_eq!(float_bits("1e3"), f(1000.0));
        assert_eq!(float_bits("1e+3"), f(1000.0));
        assert_eq!(float_bits("1e-3"), f((powi(10.0, -3) * 1.0) as f32));
        assert_eq!(float_bits("1.5e2"), f(150.0));
        assert_eq!(float_bits("2.5e1"), f(25.0));
        assert_eq!(float_bits("1E3"), f(1000.0));
        assert_eq!(float_bits("12e0"), f(12.0));
    }

    #[test]
    fn exponents_past_the_float_range_overflow_or_underflow() {
        assert_eq!(float_bits("1e39"), f(f32::INFINITY));
        assert_eq!(float_bits("1e38"), f(1e38));
        assert_eq!(float_bits("1e-45"), 1);
        assert_eq!(float_bits("1e-50"), f(0.0));
        assert_eq!(float_bits("0e99"), f(0.0));
    }

    #[test]
    fn an_exponent_sign_without_digits_has_exponent_zero() {
        assert_eq!(float_bits("1e+"), f(1.0));
        assert_eq!(float_bits("1e-"), f(1.0));
        assert_eq!(float_bits("7.5e+"), f(7.5));
        assert_eq!(toks("1e+")[0].2, (0, 3));
    }

    #[test]
    fn the_exponent_digits_wrap_as_a_signed_32_bit_integer() {
        // 2147483648 wraps to i32::MIN: 10^MIN is 0.
        assert_eq!(float_bits("1e2147483648"), f(0.0));
        // 4294967296 wraps to 0.
        assert_eq!(float_bits("1e4294967296"), f(1.0));
        assert_eq!(float_bits("1e-2147483648"), f(0.0));
    }

    #[test]
    fn a_malformed_exponent_logs_a_warning_and_the_literal_becomes_zero() {
        for (src, rest) in [("1e", ""), ("12.5e", "")] {
            let r = run(src);
            assert_eq!(r.tokens.as_ref().unwrap().len(), 1, "{src:?}");
            assert_eq!(r.tokens.unwrap()[0].1, Payload::Float(0.0));
            assert_eq!(r.log.len(), 1);
            assert_eq!(r.log[0].0, "E06");
            assert_eq!(r.log[0].1, Severity::Warning);
            assert_eq!(r.log[0].2, (0, src.len() as u32));
            assert_eq!(
                r.log[0].3,
                format!("error parsing float string, expected '+' or '-' after 'e': {rest}")
            );
        }
    }

    #[test]
    fn a_malformed_exponent_does_not_stop_the_lexer() {
        let r = run("1ex");
        assert!(r.tokens.is_none());
        assert_eq!(r.log.len(), 3);
        assert_eq!(
            r.log[0],
            (
                "E06",
                Severity::Warning,
                (0, 2),
                "error parsing float string, expected '+' or '-' after 'e': x".to_owned()
            )
        );
        assert_eq!(
            r.log[1],
            (
                "E03",
                Severity::Error,
                (2, 3),
                "Error: unknown token: x".to_owned()
            )
        );
        assert_eq!(r.log[2].0, "E02");
    }

    #[test]
    fn a_malformed_exponent_in_the_middle_quotes_the_rest_of_the_input() {
        let r = run("1e + 2");
        assert!(r.tokens.is_some());
        assert_eq!(
            r.log[0].3,
            "error parsing float string, expected '+' or '-' after 'e':  + 2"
        );
        assert_eq!(r.tokens.unwrap().len(), 3);
    }

    #[test]
    fn a_malformed_exponent_quotes_the_lowered_text() {
        let r = run("1eX");
        assert!(r.log[0].3.ends_with(": x"), "{}", r.log[0].3);
    }

    #[test]
    fn the_float_suffix_is_swallowed_in_either_case() {
        assert_eq!(toks("1.5f"), [(Op::Float, Payload::Float(1.5), (0, 4))]);
        assert_eq!(toks("1.5F"), [(Op::Float, Payload::Float(1.5), (0, 4))]);
        assert_eq!(toks("3f")[0].2, (0, 2));
        assert_eq!(toks("1e2f")[0].2, (0, 4));
        assert_eq!(toks("1e2f")[0].1, Payload::Float(100.0));
    }

    #[test]
    fn a_second_suffix_letter_is_read_as_a_boolean() {
        let t = toks("1ff");
        assert_eq!(
            t,
            [
                (Op::Float, Payload::Float(1.0), (0, 2)),
                (Op::Float, Payload::Float(0.0), (2, 3))
            ]
        );
    }

    #[test]
    fn a_letter_after_a_number_is_an_unknown_token() {
        assert_eq!(
            failure("1x")[0],
            ("E03", "Error: unknown token: x".to_owned())
        );
        assert_eq!(
            failure("1.5d")[0],
            ("E03", "Error: unknown token: d".to_owned())
        );
        assert_eq!(
            failure("0x10")[0],
            ("E03", "Error: unknown token: x10".to_owned())
        );
        assert_eq!(
            failure("1_000")[0],
            ("E03", "Error: unknown token: _000".to_owned())
        );
    }

    #[test]
    fn a_number_wins_over_the_identifier_scan_that_starts_at_its_first_digit() {
        // The identifier scan finds no name at a digit, so a number always wins.
        assert_eq!(toks("1")[0].2, (0, 1));
        assert_eq!(
            failure("_000")[0],
            ("E03", "Error: unknown token: _000".to_owned())
        );
    }

    #[test]
    fn the_sign_is_a_separate_token() {
        assert_eq!(ops("-1"), [Op::Negate, Op::Float]);
        assert_eq!(ops("1-1"), [Op::Float, Op::Negate, Op::Float]);
        assert_eq!(ops("1e-3"), [Op::Float]);
    }

    #[test]
    fn true_and_its_prefixes_are_one() {
        for src in ["t", "tr", "tru", "true"] {
            assert_eq!(
                toks(src),
                [(Op::Float, Payload::Float(1.0), (0, src.len() as u32))],
                "{src:?}"
            );
        }
    }

    #[test]
    fn false_and_its_prefixes_are_zero() {
        for src in ["f", "fa", "fal", "fals", "false"] {
            assert_eq!(
                toks(src),
                [(Op::Float, Payload::Float(0.0), (0, src.len() as u32))],
                "{src:?}"
            );
        }
    }

    #[test]
    fn words_that_are_no_prefix_of_true_or_false_are_unknown_tokens() {
        for (src, rest) in [
            ("truex", "truex"),
            ("falsey", "falsey"),
            ("tx", "tx"),
            ("e", "e"),
            ("x + 1", "x + 1"),
            ("TRUE1", "true1"),
        ] {
            let r = failure(src);
            assert_eq!(
                r[0],
                ("E03", format!("Error: unknown token: {rest}")),
                "{src:?}"
            );
            assert_eq!(
                r[1],
                ("E02", format!("unrecognized token: {rest}")),
                "{src:?}"
            );
        }
    }

    #[test]
    fn true_and_false_are_lowered() {
        assert_eq!(toks("TRUE"), [(Op::Float, Payload::Float(1.0), (0, 4))]);
        assert_eq!(toks("False")[0].1, Payload::Float(0.0));
    }

    #[test]
    fn a_boolean_prefix_ends_with_its_identifier_under_the_deviation() {
        assert_eq!(
            toks("tr*5"),
            [
                (Op::Float, Payload::Float(1.0), (0, 2)),
                (Op::Mul, Payload::None, (2, 3)),
                (Op::Float, Payload::Float(5.0), (3, 4)),
            ]
        );
    }

    #[test]
    fn without_the_deviation_a_boolean_prefix_takes_the_length_of_the_full_word() {
        let opts = CompileOptions {
            deviations: Deviations {
                true_false_prefix_advance: false,
                ..Deviations::ALL
            },
            ..client()
        };
        let r = run_with("tr*5", &opts);
        assert_eq!(
            r.tokens.unwrap(),
            [(Op::Float, Payload::Float(1.0), (0, 4))]
        );
        let r = run_with("t;", &opts);
        assert_eq!(
            r.tokens.unwrap(),
            [(Op::Float, Payload::Float(1.0), (0, 2))]
        );
        let r = run_with("f+1+1", &opts);
        assert_eq!(
            r.tokens.unwrap(),
            [(Op::Float, Payload::Float(0.0), (0, 5))]
        );
        // The advance stops at the end of the input.
        let r = run_with("t", &opts);
        assert_eq!(
            r.tokens.unwrap(),
            [(Op::Float, Payload::Float(1.0), (0, 1))]
        );
        let r = run_with("true", &opts);
        assert_eq!(
            r.tokens.unwrap(),
            [(Op::Float, Payload::Float(1.0), (0, 4))]
        );
    }

    #[test]
    fn the_full_words_take_their_own_length() {
        let opts = CompileOptions {
            deviations: Deviations {
                true_false_prefix_advance: false,
                ..Deviations::ALL
            },
            ..client()
        };
        for (src, end) in [("true", 4), ("false", 5), ("true;", 4), ("false;", 5)] {
            assert_eq!(
                run_with(src, &opts).tokens.unwrap()[0].2,
                (0, end),
                "{src:?}"
            );
        }
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn any_integer_literal_is_its_wrapped_value(n in 0u64..100_000_000_000) {
            let wrapped = (n as u32) as i32;
            prop_assert_eq!(float_bits(&n.to_string()), f(wrapped as f32));
        }
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::tree;

        #[test]
        fn true_and_false() {
            assert_eq!(tree("t.x"), "t.x");
            assert_eq!(tree("v.cooldown ?? true"), "(NullCoalescing v.cooldown 1)");
        }

        #[test]
        fn true_false_advance_deviation() {
            assert_eq!(tree("t;"), "(Semicolon 1)");
        }
    }
}
