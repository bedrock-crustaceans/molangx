//! The parse rows of `parse_vectors.json`: each input with its outcome (accepted or rejected), the
//! messages it logs, in order, and the tree the front end builds.
//!
//! ```text
//! {schema: "molangx/parse_vectors/1",
//!  groups{<group>: {tree_float_digits: 6 | 9, about}},
//!  vectors[{group, expr, version, result: "OK" | "FAIL", outcome: "ok" | "ok_with_log" | "error", tree | null,
//!           messages[{text, error_id, level}], input_bytes_hex?, note?, tree_value_unchecked?,
//!           shape?, repetitions?, node_depth?, expr_length?}]}
//! ```
//!
//! Rows are parsed with no query set, so every `query.*` fails to resolve. `error_id` is
//! `LanguageMessage::id`; `level` is the word printed before `[Molang]`. `tree` is an S-expression
//! of `ExpressionOp` variant names: `[x*s+o]` is a node with a folded multiply-add, strings print
//! as their hash, float leaves carry the group's `tree_float_digits` significant digits (trailing
//! zeros dropped), and folded constants are those of the arm64 behaviour; on an `X86_64` build the
//! rows of [`X86_64_TREE_DIFFERS`] fold to another tree and are checked for their outcome and
//! messages only. A `tree_value_unchecked` row compares only that the tree is one constant; some
//! `depth` rows have no tree. A row with `input_bytes_hex` is parsed from those bytes, and its
//! `expr` and message texts hold them decoded as Latin-1. Link-stage messages (#48, #49) are not
//! part of a row.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

mod common;

use std::collections::BTreeMap;

use common::compile_support::client_at;
use molangx::catalog::{QueryAdmission, QuerySetMask};
use molangx::compile::{CompileFailure, CompileOptions, Deviations, compile};
use molangx::diag::{Effect, LanguageMessage, Severity};
use molangx::numeric::{ARCH, Arch};
use molangx::ops::ExpressionOp;
use serde_json::Value;

const FILE: &str = "parse_vectors.json";

fn is_link_stage(message: LanguageMessage) -> bool {
    matches!(
        message,
        LanguageMessage::CompileFailed | LanguageMessage::WriteToOtherMob
    )
}

/// The level every parser message of the rows prints at.
const ERROR_LEVEL: &str = "ERROR";

/// The parse outcome must not depend on the deviations.
const DEVIATIONS: [(&str, Deviations); 2] = [("ALL", Deviations::ALL), ("NONE", Deviations::NONE)];

/// The indexes of the rows whose tree differs on `X86_64`; their outcome and messages do not.
const X86_64_TREE_DIFFERS: &[usize] = &[810, 852, 900, 936];

fn source(row: &Value) -> String {
    row.get("input_bytes_hex")
        .and_then(Value::as_str)
        .map_or_else(
            || row["expr"].as_str().expect("expr").to_owned(),
            common::hex_to_string,
        )
}

fn version(row: &Value) -> i16 {
    i16::try_from(row["version"].as_i64().expect("version")).expect("version fits i16")
}

fn group_digits(file: &Value) -> BTreeMap<String, usize> {
    file["groups"]
        .as_object()
        .expect("groups")
        .iter()
        .map(|(group, about)| {
            let digits = about["tree_float_digits"]
                .as_u64()
                .expect("tree_float_digits");
            (group.clone(), usize::try_from(digits).expect("digits"))
        })
        .collect()
}

#[test]
fn every_recorded_input_gives_its_recorded_outcome_messages_and_tree() {
    let file = common::data_json(FILE);
    let vectors = file["vectors"].as_array().expect("vectors");
    assert_eq!(vectors.len(), 1277, "the file has 1,277 rows");
    let digits = group_digits(&file);

    let mut failures = Vec::new();
    let mut by_group = BTreeMap::<String, [usize; 3]>::new();
    for (index, row) in vectors.iter().enumerate() {
        let group = row["group"].as_str().expect("group");
        let Some(&group_digits) = digits.get(group) else {
            failures.push(format!("#{index}: group {group:?} is not one of `groups`"));
            continue;
        };
        let entry = by_group.entry(group.to_owned()).or_default();
        entry[2] += 1;
        for (slot, (label, deviations)) in DEVIATIONS.iter().enumerate() {
            let tree_differs = ARCH == Arch::X86_64 && X86_64_TREE_DIFFERS.contains(&index);
            let passes = check(row, group_digits, *deviations, tree_differs)
                .map_err(|why| {
                    failures.push(format!(
                        "#{index} [{group}] v{} {:?} ({label}): {why}",
                        row["version"],
                        row["expr"].as_str().unwrap_or("")
                    ))
                })
                .is_ok();
            entry[slot] += usize::from(passes);
        }
    }
    for (group, [all, none, total]) in &by_group {
        println!("{group}: {all}/{total} (ALL), {none}/{total} (NONE)");
    }
    assert!(
        failures.is_empty(),
        "{} checks of {} rows differ:\n{}",
        failures.len(),
        vectors.len(),
        failures.join("\n")
    );
    assert_eq!(by_group.len(), digits.len(), "every group has rows");
}

fn significant_digits(token: &str) -> usize {
    let mantissa = token.split('e').next().unwrap_or(token);
    mantissa
        .chars()
        .filter(char::is_ascii_digit)
        .skip_while(|&c| c == '0')
        .count()
}

/// No float token has more digits than its group's `tree_float_digits`, so a regenerated file that
/// carries more is compared at what it carries.
#[test]
fn float_tokens_carry_their_groups_digits() {
    let file = common::data_json(FILE);
    let digits = group_digits(&file);
    let mut longest = BTreeMap::<&str, usize>::new();
    for row in file["vectors"].as_array().expect("vectors") {
        let group = row["group"].as_str().expect("group");
        let allowed = digits[group];
        let tree = row["tree"].as_str().unwrap_or("");
        for token in tree.split([' ', '(', ')', '[', ']', '*', '+']) {
            let is_float = !token.is_empty()
                && token
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e'));
            // Hashes print as integers of up to 20 digits; a float leaf has at most the group's
            // digits and a sign.
            if is_float
                && (token.contains('.') || token.contains('e') || token.len() <= allowed + 1)
            {
                let entry = longest.entry(group).or_default();
                *entry = (*entry).max(significant_digits(token));
            }
        }
    }
    for (group, allowed) in &digits {
        let found = longest.get(group.as_str()).copied().unwrap_or(0);
        assert!(
            found <= *allowed,
            "{group}: a float token of {found} digits, more than {allowed}"
        );
    }
    let six_digit_groups = digits
        .iter()
        .filter(|&(_, &d)| d == 6)
        .map(|(group, _)| group.as_str())
        .collect::<Vec<_>>();
    assert_eq!(six_digit_groups, ["operands", "syntax"]);
    let longest_of_six = six_digit_groups
        .iter()
        .map(|group| longest.get(group).copied().unwrap_or(0))
        .max();
    assert_eq!(
        longest_of_six,
        Some(6),
        "the longest float token of the six-digit groups"
    );
}

/// The operators whose `ExpressionOp::friendly_name` occurs in a message text of the rows.
#[test]
fn recorded_friendly_names() {
    use ExpressionOp::*;
    const RECORDED: [ExpressionOp; 41] = [
        LeftBrace,
        RightBrace,
        LeftBracket,
        RightBracket,
        LeftParenthesis,
        RightParenthesis,
        Negate,
        LogicalNot,
        Abs,
        Add,
        Clamp,
        Div,
        Floor,
        Max,
        Min,
        Mul,
        Random,
        ContextVariable,
        StringLiteral,
        GeometryVariable,
        MaterialVariable,
        TextureVariable,
        LessThan,
        GreaterEqual,
        GreaterThan,
        LogicalOr,
        LogicalAnd,
        NullCoalescing,
        Conditional,
        ConditionalElse,
        Float,
        Pi,
        Array,
        Loop,
        Break,
        Continue,
        Assignment,
        Pointer,
        Return,
        Comma,
        This,
    ];
    let file = common::data_json(FILE);
    let texts: Vec<&str> = file["vectors"]
        .as_array()
        .expect("vectors")
        .iter()
        .flat_map(|row| row["messages"].as_array().expect("messages"))
        .map(|message| message["text"].as_str().expect("text"))
        .collect();
    let recorded: Vec<ExpressionOp> = ExpressionOp::all()
        .iter()
        .copied()
        .filter(|op| texts.iter().any(|text| text.contains(op.friendly_name())))
        .collect();
    assert_eq!(recorded, RECORDED);
}

#[cfg_attr(not(feature = "fuzz"), allow(unused_variables))]
fn check(
    row: &Value,
    digits: usize,
    deviations: Deviations,
    tree_differs: bool,
) -> Result<(), String> {
    let has_bytes = row.get("input_bytes_hex").and_then(Value::as_str).is_some();
    let options = CompileOptions {
        admission: QueryAdmission::Sets(QuerySetMask::empty()),
        deviations,
        ..client_at(version(row))
    };
    let compiled = compile(&source(row), &options);

    let all: Vec<(String, &str, String)> = compiled
        .diagnostics()
        .iter()
        .filter_map(|d| {
            d.language_message()
                .filter(|m| !is_link_stage(*m))
                .map(|m| {
                    (
                        m.id().to_owned(),
                        if d.language_message().is_some() {
                            ERROR_LEVEL
                        } else {
                            ""
                        },
                        d.message().trim().to_owned(),
                    )
                })
        })
        .collect();
    let theirs: Vec<(String, &str, String)> = row["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|m| {
            let text = m["text"].as_str().expect("text");
            let text = if has_bytes {
                common::latin1_to_utf8(text)
            } else {
                text.to_owned()
            };
            (
                m["error_id"].as_str().expect("error_id").to_owned(),
                m["level"].as_str().expect("level"),
                text.trim().to_owned(),
            )
        })
        .collect();
    if all != theirs {
        return Err(format!(
            "messages differ\n    all:   {all:?}\n    theirs: {theirs:?}"
        ));
    }

    let parsed = compiled.parsed();
    let expect_parsed = row["result"] == "OK";
    if parsed != expect_parsed {
        return Err(format!(
            "result: all {}, theirs {}",
            if parsed { "OK" } else { "FAIL" },
            row["result"]
        ));
    }
    let outcome = match (parsed, all.is_empty()) {
        (true, true) => "ok",
        (true, false) => "ok_with_log",
        (false, _) => "error",
    };
    if row["outcome"] != outcome {
        return Err(format!("outcome: all {outcome}, theirs {}", row["outcome"]));
    }
    // A rejection carries an Error. A message of a kept expression is a Warning under ALL and an
    // Error under NONE, except a message whose effect is Keep (the exponent message, #6), which is
    // a Warning under both.
    let rejected = compiled.failure() == Some(CompileFailure::Rejected);
    let severities: Vec<Severity> = compiled
        .diagnostics()
        .iter()
        .filter(|d| d.language_message().is_some())
        .map(|d| d.severity())
        .collect();
    if rejected && !severities.contains(&Severity::Error) {
        return Err(format!("rejected without an Error: {severities:?}"));
    }
    if !rejected && deviations.validate_nested && severities.contains(&Severity::Error) {
        return Err(format!("kept, but an Error under ALL: {severities:?}"));
    }
    if !rejected && !deviations.validate_nested {
        for d in compiled
            .diagnostics()
            .iter()
            .filter(|d| d.language_message().is_some())
        {
            let expected = if d
                .language_message()
                .is_some_and(|m| m.effect() == Effect::Keep)
            {
                Severity::Warning
            } else {
                Severity::Error
            };
            if d.severity() != expected {
                return Err(format!(
                    "{:?} kept at {:?} under NONE, expected {expected:?}",
                    d.language_message(),
                    d.severity()
                ));
            }
        }
    }

    // The tree is compared only where `Compiled::tree_notation` exists (`fuzz`).
    #[cfg(feature = "fuzz")]
    {
        if expect_parsed {
            let all = compiled
                .tree_notation(digits)
                .expect("a parsed expression has a tree");
            // Some depth-limit rows have no tree: it was too large to print.
            let Some(theirs) = row["tree"].as_str() else {
                return Ok(());
            };
            if row["tree_value_unchecked"] == true {
                if all.parse::<f64>().is_err() {
                    return Err(format!("tree: all {all}, expected a folded constant"));
                }
            } else if tree_differs {
                if all == theirs {
                    return Err(format!(
                        "listed in X86_64_TREE_DIFFERS, but the tree is {theirs}"
                    ));
                }
            } else if all != theirs {
                return Err(format!(
                    "tree differs\n    all:     {all}\n    expected: {theirs}"
                ));
            }
        }
    }
    Ok(())
}
