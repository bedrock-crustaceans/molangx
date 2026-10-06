//! Single operations of each architecture from `numeric_bits.json`: this build's own set compared
//! to the bit.
//!
//! ```text
//! {schema, counts{<set>: {rows, transcendental_rows, families{}}}, profiles{X86_64[], Arm64[]}}
//! profiles.<set>[] = {family, function, rows[{args[], samples[]?, post_op: null | {scale, offset}, result_bits | null,
//!                                               transcendental, transcendental_samples[]?}]}
//! ```
//!
//! Every float is its bit pattern as eight hex digits. `function` names the molangx function and
//! `args` are its arguments in order (`div_guard_*`: `result_bits` null when the guard fired).
//! `samples` are the random draws in order. `transcendental: true` marks a row that evaluates a
//! transcendental function; `transcendental_samples` lists the `[function, [argument bits…], result
//! bits]` the row relies on, each also checked alone against the function evaluated in `f64` and
//! rounded to `f32`. `counts` states the file's own size per set.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

mod common;

use common::{Samples, hex_f32, per_arch};

use molangx::numeric::{self, ARCH, Arch, PostOp};
use molangx::rng::FixedRng;
use molangx::stdlib::math;
use serde_json::Value;

const ID: PostOp = PostOp::IDENTITY;

/// `math.die_roll*` stepped with the row's raw samples. When every sample is a word's, the one-call
/// form over a generator playing those words back must agree to the bit and draw once per roll.
fn die_roll_row(integer: bool, a: &[f32], samples: &[f32], post: PostOp) -> f32 {
    let mut roll = if integer {
        math::DieRoll::new_integer(a[0], a[1], a[2])
    } else {
        math::DieRoll::new(a[0], a[1], a[2])
    };
    let rolls = roll.remaining() as usize;
    for &sample in &samples[..rolls] {
        roll.roll(sample);
    }
    let stepped = roll.finish(post);
    if samples
        .iter()
        .all(|&sample| FixedRng::from_sample(sample).is_some())
    {
        let mut rng = Samples::new(samples);
        let whole = if integer {
            math::die_roll_integer(a[0], a[1], a[2], u32::MAX, &mut rng, post)
        } else {
            math::die_roll(a[0], a[1], a[2], u32::MAX, &mut rng, post)
        };
        assert_eq!(
            whole.map(f32::to_bits),
            Some(stepped.to_bits()),
            "die_roll over {samples:?}"
        );
        assert_eq!(rng.draws, rolls, "one draw per roll");
    }
    stepped
}

/// `None` means the division guard fired.
fn evaluate_row(function: &str, a: &[f32], samples: &[f32], post: PostOp) -> Option<f32> {
    type Ease = fn(f32, f32, f32, PostOp) -> f32;
    let ease: Option<Ease> = match function {
        "ease_in_quad" => Some(math::ease_in_quad),
        "ease_out_quad" => Some(math::ease_out_quad),
        "ease_in_out_quad" => Some(math::ease_in_out_quad),
        "ease_in_cubic" => Some(math::ease_in_cubic),
        "ease_out_cubic" => Some(math::ease_out_cubic),
        "ease_in_out_cubic" => Some(math::ease_in_out_cubic),
        "ease_in_quart" => Some(math::ease_in_quart),
        "ease_out_quart" => Some(math::ease_out_quart),
        "ease_in_out_quart" => Some(math::ease_in_out_quart),
        "ease_in_quint" => Some(math::ease_in_quint),
        "ease_out_quint" => Some(math::ease_out_quint),
        "ease_in_out_quint" => Some(math::ease_in_out_quint),
        "ease_in_sine" => Some(math::ease_in_sine),
        "ease_out_sine" => Some(math::ease_out_sine),
        "ease_in_out_sine" => Some(math::ease_in_out_sine),
        "ease_in_expo" => Some(math::ease_in_expo),
        "ease_out_expo" => Some(math::ease_out_expo),
        "ease_in_out_expo" => Some(math::ease_in_out_expo),
        "ease_in_circ" => Some(math::ease_in_circ),
        "ease_out_circ" => Some(math::ease_out_circ),
        "ease_in_out_circ" => Some(math::ease_in_out_circ),
        "ease_in_bounce" => Some(math::ease_in_bounce),
        "ease_out_bounce" => Some(math::ease_out_bounce),
        "ease_in_out_bounce" => Some(math::ease_in_out_bounce),
        "ease_in_back" => Some(math::ease_in_back),
        "ease_out_back" => Some(math::ease_out_back),
        "ease_in_out_back" => Some(math::ease_in_out_back),
        "ease_in_elastic" => Some(math::ease_in_elastic),
        "ease_out_elastic" => Some(math::ease_out_elastic),
        "ease_in_out_elastic" => Some(math::ease_in_out_elastic),
        _ => None,
    };
    if let Some(ease) = ease {
        return Some(ease(a[0], a[1], a[2], post));
    }
    let value = match function {
        "add" => numeric::add(a[0], a[1], post),
        "mul" => numeric::mul(a[0], a[1], post),
        "div" => numeric::div(a[0], a[1], post),
        "negate" => numeric::negate(a[0], post),
        "div_guard_signed" => return numeric::div_guard(true, a[0]),
        "div_guard_abs" => return numeric::div_guard(false, a[0]),
        "lt" => post.select(numeric::lt(a[0], a[1])),
        "le" => post.select(numeric::le(a[0], a[1])),
        "ge" => post.select(numeric::ge(a[0], a[1])),
        "gt" => post.select(numeric::gt(a[0], a[1])),
        "abs" => math::abs(a[0], post),
        "acos" => math::acos(a[0], post),
        "asin" => math::asin(a[0], post),
        "atan" => math::atan(a[0], post),
        "ceil" => math::ceil(a[0], post),
        "cos" => math::cos(a[0], post),
        "exp" => math::exp(a[0], post),
        "floor" => math::floor(a[0], post),
        "hermite_blend" => math::hermite_blend(a[0], post),
        "ln" => math::ln(a[0], post),
        "min_angle" => math::min_angle(a[0], post),
        "round" => math::round(a[0], post),
        "sin" => math::sin(a[0], post),
        "sqrt" => math::sqrt(a[0], post),
        "trunc" => math::trunc(a[0], post),
        "atan2" => math::atan2(a[0], a[1], post),
        "copy_sign" => math::copy_sign(a[0], a[1], post),
        "pow" => math::pow(a[0], a[1], post),
        "min" => math::min(a[0], a[1], post),
        "max" => math::max(a[0], a[1], post),
        "clamp" => math::clamp(a[0], a[1], a[2], post),
        "lerp" => math::lerp(a[0], a[1], a[2], post),
        "lerprotate" => math::lerprotate(a[0], a[1], a[2], post),
        "inverse_lerp" => math::inverse_lerp(a[0], a[1], a[2], post),
        "mod_const" => math::mod_const(a[0], a[1], post),
        "mod_runtime" => math::mod_runtime(a[0], a[1], post),
        "sign" => math::sign(a[0], post),
        "random" => math::random(a[0], a[1], samples[0], post),
        "random_const" => {
            math::random_folded(samples[0], math::random_const_bounds(a[0], a[1], post))
        }
        "random_integer" => math::random_integer(a[0], a[1], samples[0], post),
        "random_integer_const" => math::random_integer_const_bounds(a[0], a[1], samples[0], post),
        "die_roll" => die_roll_row(false, a, samples, post),
        "die_roll_integer" => die_roll_row(true, a, samples, post),
        other => panic!("the file names a function this test does not know: {other}"),
    };
    Some(value)
}

/// `math.ln` of a negative number: `0x7fc00000` on `X86_64` (`LN_NAN_SIGN` in `data_sweeps.rs`);
/// this file gives `0xffc00000`.
const LN_NAN_SIGN: &str =
    "math.ln of a negative number: 0x7fc00000 on x86-64 (LN_NAN_SIGN in data_sweeps.rs)";

/// A row checked against another value.
struct Override {
    set: &'static str,
    group: usize,
    row: usize,
    bits: u32,
    reason: &'static str,
}

const fn ln_sign(row: usize) -> Override {
    Override {
        set: "X86_64",
        group: 9,
        row,
        bits: 0x7fc0_0000,
        reason: LN_NAN_SIGN,
    }
}

/// Rows checked against another value; each must exist and differ from its own bits.
const OVERRIDES: [Override; 7] = [
    ln_sign(3),
    ln_sign(5),
    ln_sign(7),
    ln_sign(13),
    ln_sign(15),
    ln_sign(22),
    ln_sign(23),
];

/// The bits are equal, NaN payload and sign included.
fn agrees(actual: Option<f32>, expected: Option<f32>) -> bool {
    actual.map(f32::to_bits) == expected.map(f32::to_bits)
}

/// The set of this build's architecture.
fn own_set() -> &'static str {
    per_arch("X86_64", "Arm64")
}

/// The set of the other architecture.
fn other_set() -> &'static str {
    per_arch("Arm64", "X86_64")
}

/// A row's post-op, or the identity.
fn post_of(row: &Value) -> PostOp {
    if row["post_op"].is_null() {
        ID
    } else {
        PostOp::new(
            hex_f32(&row["post_op"]["scale"]),
            hex_f32(&row["post_op"]["offset"]),
        )
    }
}

/// A row's arguments, samples and result (`None`: the guard fired).
fn row_values(row: &Value) -> (Vec<f32>, Vec<f32>, Option<f32>) {
    let args = row["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(hex_f32)
        .collect();
    let samples = row
        .get("samples")
        .and_then(Value::as_array)
        .map(|s| s.iter().map(hex_f32).collect())
        .unwrap_or_default();
    let expected = if row["result_bits"].is_null() {
        None
    } else {
        Some(hex_f32(&row["result_bits"]))
    };
    (args, samples, expected)
}

#[test]
fn this_architecture_matches_its_rows_bit_for_bit() {
    let file = common::data_json("numeric_bits.json");
    let name = own_set();
    let (mut rows, mut nan_rows, mut overridden) = (0_u64, 0_u64, 0);
    let mut failures = Vec::new();
    for (g, group) in file["profiles"][name]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let function = group["function"].as_str().unwrap();
        for (r, row) in group["rows"].as_array().unwrap().iter().enumerate() {
            rows += 1;
            let (args, samples, mut expected) = row_values(row);
            if let Some(o) = OVERRIDES
                .iter()
                .find(|o| o.set == name && o.group == g && o.row == r)
            {
                assert!(
                    !agrees(expected, Some(f32::from_bits(o.bits))),
                    "g{g} r{r} ({}) gives its own bits",
                    o.reason
                );
                expected = Some(f32::from_bits(o.bits));
                overridden += 1;
            }
            nan_rows += u64::from(expected.is_some_and(f32::is_nan));
            let actual = evaluate_row(function, &args, &samples, post_of(row));
            if !agrees(actual, expected) {
                failures.push(format!(
                    "g{g} r{r} {function}({}) samples {:?} post {:?} = {:?}, the reference gives {:?}",
                    row["args"],
                    row.get("samples"),
                    row["post_op"],
                    actual.map(f32::to_bits),
                    expected.map(f32::to_bits)
                ));
            }
        }
    }
    println!("numeric_bits.json [{name}]: {rows} rows, {nan_rows} NaN");
    assert!(
        failures.is_empty(),
        "{} row(s) disagree:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(
        overridden,
        OVERRIDES.iter().filter(|o| o.set == name).count(),
        "an override names no row"
    );
}

/// A reader that loses rows must fail.
#[test]
fn both_sets_hold_the_rows_they_count() {
    let file = common::data_json("numeric_bits.json");
    for name in ["X86_64", "Arm64"] {
        let (mut rows, mut transcendental_rows) = (0_u64, 0_u64);
        let mut families = std::collections::BTreeMap::<String, u64>::new();
        for group in file["profiles"][name].as_array().unwrap() {
            for row in group["rows"].as_array().unwrap() {
                rows += 1;
                transcendental_rows += u64::from(row["transcendental"].as_bool().unwrap());
                *families
                    .entry(group["family"].as_str().unwrap().to_owned())
                    .or_default() += 1;
            }
        }
        let counts = &file["counts"][name];
        assert_eq!(Some(rows), counts["rows"].as_u64(), "{name}");
        assert_eq!(
            Some(transcendental_rows),
            counts["transcendental_rows"].as_u64(),
            "{name}"
        );
        for (family, count) in counts["families"].as_object().unwrap() {
            assert_eq!(
                families.get(family).copied(),
                count.as_u64(),
                "{name} {family}"
            );
        }
        assert_eq!(
            families.len(),
            counts["families"].as_object().unwrap().len(),
            "{name}"
        );
    }
}

/// The other architecture's rows fail on this build, so each set is held only on its own
/// architecture.
#[test]
fn this_architecture_fails_some_rows_of_the_other() {
    let file = common::data_json("numeric_bits.json");
    let other = other_set();
    let mut differing = std::collections::BTreeSet::new();
    for group in file["profiles"][other].as_array().unwrap() {
        let function = group["function"].as_str().unwrap();
        for row in group["rows"].as_array().unwrap() {
            let (args, samples, expected) = row_values(row);
            if function.starts_with("die_roll")
                && samples.len() != math::die_roll_count(args[0]) as usize
            {
                // The two architectures roll a different number of times here.
                differing.insert(function.to_owned());
                continue;
            }
            if !agrees(
                evaluate_row(function, &args, &samples, post_of(row)),
                expected,
            ) {
                differing.insert(function.to_owned());
            }
        }
    }
    println!(
        "{ARCH:?} differs from the {other} rows in {} functions",
        differing.len()
    );
    for function in [
        "add",
        "mul",
        "div",
        "negate",
        "div_guard_signed",
        "lt",
        "le",
        "acos",
        "asin",
        "hermite_blend",
        "min",
        "max",
        "clamp",
        "lerp",
        "sign",
        "random",
        "random_integer",
        "die_roll",
        "die_roll_integer",
        "ease_in_quart",
        "ease_in_elastic",
    ] {
        assert!(
            differing.contains(function),
            "{function} does not tell {ARCH:?} from {other}"
        );
    }
}

/// `x` and its `n` neighbouring floats on each side.
fn around(x: f32, n: i32) -> impl Iterator<Item = f32> {
    (-n..=n).map(move |k| f32::from_bits(x.to_bits().wrapping_add_signed(k)))
}

/// The degrees whose product with `DEG_TO_RAD` is `radians`.
fn degrees_of(radians: f32) -> Option<f32> {
    if !radians.is_finite() {
        return Some(radians);
    }
    around(radians / math::DEG_TO_RAD, 8).find(|&d| d * math::DEG_TO_RAD == radians)
}

/// `sin(radians)` through the sine table of `ease_out_sine`, when `radians` is one of its angles:
/// entry `i` is `sin(i / SCALE)` on `X86_64` and `sin(i · STEP)` on `Arm64`.
fn sine_table(radians: f32) -> Option<f32> {
    const SCALE: f32 = f32::from_bits(0x4622_f983);
    const STEP: f32 = f32::from_bits(0x38c9_0fdb);
    const HALF_PI: f32 = f32::from_bits(0x3fc9_0fdb);
    let entry = |i: i32| per_arch(i as f32 / SCALE, i as f32 * STEP);
    let guess = (radians * SCALE).round() as i32;
    let index =
        (guess - 1..=guess + 1).find(|&i| (0..65_536).contains(&i) && entry(i) == radians)?;
    let t = around(index as f32 / 16_384.0, 64)
        .find(|&t| ((t * HALF_PI) * SCALE) as i32 & 0xffff == index)?;
    Some(math::ease_out_sine(0.0, 1.0, t, ID))
}

/// `exp2(x)` through `ease_in_expo` from 0 to 1, when some `t` gives the exponent `x`: `t·10 − 10`
/// rounded once on `Arm64`, `(t − 1)·10` on `X86_64`.
fn exp2_through_an_easing(x: f32) -> Option<f32> {
    if !x.is_finite() {
        return Some(math::ease_in_expo(0.0, 1.0, x, ID));
    }
    let t = match ARCH {
        Arch::X86_64 => around(x / 10.0 + 1.0, 32).find(|&t| (t - 1.0) * 10.0 == x),
        Arch::Arm64 => around((x + 10.0) / 10.0, 32).find(|&t| t.mul_add(10.0, -10.0) == x),
    };
    t.map(|t| math::ease_in_expo(0.0, 1.0, t, ID))
}

/// The `exp2` exponents no `ease_in_expo` reaches; the unit tests of `exp2` pin them.
const EXP2_UNREACHED: [u32; 5] = [
    0xbf80_0000,
    0xc080_0000,
    0xbfd6_ab74,
    0xbf13_7db9,
    0xc08c_be3f,
];

/// On `Arm64`, `math.asin` and `math.acos` read a NaN argument as −1, so their plain function of a
/// NaN is not reached.
fn reaches_the_inverse_trig_of(x: f32) -> bool {
    !(ARCH == Arch::Arm64 && x.is_nan())
}

/// Every `transcendental_samples` entry, `[function, [argument bits…], result bits]`, is to the bit
/// what the crate's function gives: trigonometry through the degree conversion, `exp2` through
/// `ease_in_expo`. The samples of an [`OVERRIDES`] row of this build's set are as overridden. A NaN
/// the other set records for an invalid operation is the other architecture's, so there only its
/// being a NaN is compared.
#[test]
fn every_transcendental_sample_is_the_crates_function() {
    let file = common::data_json("numeric_bits.json");
    let (mut checked, mut failures, mut unreached) =
        (0_u32, Vec::new(), std::collections::BTreeSet::new());
    let in_degrees = |radians: f32| radians * math::RAD_TO_DEG;
    for set in ["X86_64", "Arm64"] {
        for (g, group) in file["profiles"][set].as_array().unwrap().iter().enumerate() {
            let rows = group["rows"].as_array().unwrap().iter().enumerate();
            for (r, sample) in rows
                .filter_map(|(r, row)| Some((r, row.get("transcendental_samples")?)))
                .flat_map(|(r, s)| s.as_array().unwrap().iter().map(move |s| (r, s)))
            {
                let a: Vec<f32> = sample[1].as_array().unwrap().iter().map(hex_f32).collect();
                let overridden = OVERRIDES
                    .iter()
                    .find(|o| set == own_set() && o.set == set && o.group == g && o.row == r);
                let expected =
                    overridden.map_or_else(|| hex_f32(&sample[2]), |o| f32::from_bits(o.bits));
                let (actual, expected) = match (sample[0].as_str().unwrap(), a.as_slice()) {
                    ("sin", &[x]) => (
                        degrees_of(x)
                            .map(|d| math::sin(d, ID))
                            .or_else(|| sine_table(x)),
                        expected,
                    ),
                    ("cos", &[x]) => (degrees_of(x).map(|d| math::cos(d, ID)), expected),
                    ("asin", &[x]) => (
                        reaches_the_inverse_trig_of(x).then(|| math::asin(x, ID)),
                        in_degrees(expected),
                    ),
                    ("acos", &[x]) => (
                        reaches_the_inverse_trig_of(x).then(|| math::acos(x, ID)),
                        in_degrees(expected),
                    ),
                    ("atan", &[x]) => (Some(math::atan(x, ID)), in_degrees(expected)),
                    ("atan2", &[y, x]) => (Some(math::atan2(y, x, ID)), in_degrees(expected)),
                    ("exp", &[x]) => (Some(math::exp(x, ID)), expected),
                    ("exp2", &[x]) => (exp2_through_an_easing(x), expected),
                    ("ln", &[x]) => (Some(math::ln(x, ID)), expected),
                    ("pow", &[x, y]) => (Some(math::pow(x, y, ID)), expected),
                    (other, _) => {
                        panic!("the file names a function this test does not know: {other}")
                    }
                };
                let Some(actual) = actual else {
                    unreached.insert((sample[0].as_str().unwrap().to_owned(), a[0].to_bits()));
                    continue;
                };
                checked += 1;
                let invalid_of_the_other =
                    set != own_set() && expected.is_nan() && !a.iter().any(|x| x.is_nan());
                let agrees = if invalid_of_the_other {
                    actual.is_nan()
                } else {
                    actual.to_bits() == expected.to_bits()
                };
                if !agrees {
                    failures.push(format!(
                        "{set} {sample} gives {:08x}, expected {:08x}",
                        actual.to_bits(),
                        expected.to_bits()
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} sample(s) disagree:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let mut expected_unreached: std::collections::BTreeSet<_> = EXP2_UNREACHED
        .iter()
        .map(|&bits| ("exp2".to_owned(), bits))
        .collect();
    if ARCH == Arch::Arm64 {
        expected_unreached.extend([
            ("asin".to_owned(), 0x7fc0_0000),
            ("acos".to_owned(), 0x7fc0_0000),
        ]);
    }
    assert_eq!(unreached, expected_unreached);
    assert_eq!(checked, per_arch(1468 - 18, 1468 - 18 - 3));
}
