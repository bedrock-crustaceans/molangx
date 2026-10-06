//! The query table of the fuzz world: the queries the generator calls and what each does.

// The query functions return a `QueryResult` even when they cannot fail.
#![allow(clippy::unnecessary_wraps)]

use super::FuzzHost;
use molangx::internals::reference_catalog;
use molangx::stdlib::query;
use molangx::vm::{QueryCx, QueryResult, QueryTable, Value};

/// The query table of the fuzz world, over the reference catalogue: nine of the eleven queries
/// `tests/common/host.rs` implements (not `query.client_memory_tier` and
/// `query.has_block_property`), written the same way.
pub fn queries() -> QueryTable<FuzzHost> {
    let implementations: [(&str, Implementation); 9] = [
        (query::COUNT, count),
        (query::IS_BABY, is_baby),
        (query::ANY, any),
        (query::ALL, all),
        (query::IN_RANGE, in_range),
        (query::LOG, log),
        (reference_catalog::GET_NAME_TEST, get_name_test),
        (reference_catalog::SUM_TEST, sum_test),
        (reference_catalog::EXPERIMENTAL_TEST, experimental_test),
    ];
    let mut table = QueryTable::new(reference_catalog::catalog());
    for (name, f) in implementations {
        // Every name is declared in the reference catalogue.
        let _ = table.set(name, f);
    }
    table
}

type Cx<'a, 'w> = QueryCx<'a, 'w, FuzzHost>;
type Implementation = fn(&mut Cx<'_, '_>) -> QueryResult<FuzzHost>;

fn get_name_test(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    let index = cx.arg_f32(0).unwrap_or(0.0);
    Ok(Value::string(if index == 1.0 { "rabbit" } else { "moo" }))
}

fn sum_test(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    let mut sum = 0.0f32;
    for i in 0..cx.arg_count() {
        sum += cx.arg(i).map_or(0.0, |v| v.as_f32());
    }
    Ok(Value::Float(sum))
}

fn experimental_test(_cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    Ok(Value::ONE)
}

fn count(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    let total: usize = (0..cx.arg_count())
        .map(|i| match cx.arg(i) {
            Some(Value::ActorArray(array)) => array.len(),
            _ => 1,
        })
        .sum();
    Ok(Value::Float(total as f32))
}

fn is_baby(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    let baby = cx
        .subjects()
        .actor
        .is_some_and(|actor| cx.host().baby & (1 << (actor.number() % 8)) != 0);
    Ok(Value::bool(baby))
}

fn same_arg(a: &Value<FuzzHost>, b: &Value<FuzzHost>) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

fn any(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    if cx.arg_count() < 3 {
        return Err(cx.error("query.any takes at least three arguments"));
    }
    let first = cx.arg(0).unwrap_or_default();
    let found = (1..cx.arg_count()).any(|i| cx.arg(i).is_some_and(|v| same_arg(&first, &v)));
    Ok(Value::bool(found))
}

fn all(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    if cx.arg_count() < 3 {
        return Err(cx.error("query.all takes at least three arguments"));
    }
    let first = cx.arg(0).unwrap_or_default();
    let every = (1..cx.arg_count()).all(|i| cx.arg(i).is_some_and(|v| same_arg(&first, &v)));
    Ok(Value::bool(every))
}

fn in_range(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    if cx.arg_count() != 3 {
        return Err(cx.error("query.in_range takes three numbers"));
    }
    let mut values = [0.0f32; 3];
    for (i, slot) in values.iter_mut().enumerate() {
        *slot = cx
            .arg_f32(i)
            .ok_or_else(|| cx.error("an argument of query.in_range is not a number"))?;
    }
    let [v, min, max] = values;
    Ok(Value::bool(min <= v && v <= max))
}

fn log(cx: &mut Cx<'_, '_>) -> QueryResult<FuzzHost> {
    let first = cx.arg(0).unwrap_or_default();
    for i in 1..cx.arg_count() {
        let _ = cx.arg(i);
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::env::{FuzzEnv, FuzzRng, Subject, TempLifetime, test_support::*};
    use molangx::rng::{FixedRng, Xorshift128, sample};
    use molangx::vm::EvalLimits;

    #[test]
    fn the_query_table_implements_the_fuzz_queries_and_the_test_ones() {
        let table = queries();
        assert_eq!(table.implemented(), 9);
        assert!(table.catalog().same(reference_catalog::catalog()));
        for name in [
            query::COUNT,
            query::IS_BABY,
            query::ANY,
            query::ALL,
            query::IN_RANGE,
            query::LOG,
            reference_catalog::GET_NAME_TEST,
            reference_catalog::SUM_TEST,
            reference_catalog::EXPERIMENTAL_TEST,
        ] {
            assert_eq!(table.is_stub(name), Ok(false), "{name}");
        }
        assert!(table.is_stub(query::TIME_OF_DAY).unwrap());
        assert_eq!(env().queries.implemented(), table.implemented());
    }

    mod queries_through_the_vm {
        use super::*;
        use molangx::compile::{CompileOptions, compile};

        fn options() -> CompileOptions {
            molangx::internals::reference_catalog::options(13)
        }

        /// Runs `src` on `env`, which keeps what it left behind.
        fn run(src: &str, env: &mut FuzzEnv) -> V {
            let compiled = compile(src, &options());
            let expr = compiled
                .expr()
                .cloned()
                .unwrap_or_else(|| panic!("{src:?}: {:?}", compiled.diagnostics()));
            expr.eval(&mut env.cx())
        }

        fn value(src: &str) -> V {
            run(src, &mut xorshift_env())
        }

        fn number(src: &str) -> f32 {
            value(src).as_f32()
        }

        /// How many random draws running `src` takes (counted up to 10).
        fn draws(src: &str) -> usize {
            let mut env = xorshift_env();
            run(src, &mut env);
            let mut count = 0;
            let mut reference = Xorshift128::new();
            while env.rng != FuzzRng::Xorshift(reference.clone()) && count < 10 {
                sample(&mut reference);
                count += 1;
            }
            count
        }

        #[test]
        fn count_adds_the_length_of_arrays_and_one_for_anything_else() {
            assert_eq!(number("q.count(c.arr)"), 3.0);
            assert_eq!(
                number("q.count(v.arr)"),
                4.0,
                "the array of the subject has a null entry"
            );
            assert_eq!(number("q.count(1, 2)"), 2.0);
            assert_eq!(number("q.count(v.x, c.arr)"), 4.0);
            assert_eq!(number("q.count(c.arr, c.arr)"), 6.0);
            assert_eq!(number("q.count(v.s)"), 1.0);
        }

        #[test]
        fn is_baby_asks_the_subject_actor() {
            assert_eq!(number("q.is_baby"), 0.0, "actor 1 is no baby");
            assert_eq!(number("c.other -> q.is_baby"), 1.0, "actor 2 is");
            assert_eq!(
                number("v.e -> q.is_baby"),
                1.0,
                "v.e is the id of actor 2, which the subject resolves"
            );
            let mut detached = detached_env();
            assert_eq!(
                run("q.is_baby", &mut detached).as_f32(),
                0.0,
                "no subject actor"
            );
            let mut babies = env();
            babies.world.baby = 0b0010;
            assert_eq!(run("q.is_baby", &mut babies).as_f32(), 1.0);
        }

        #[test]
        fn any_is_one_when_a_later_argument_equals_the_first() {
            assert_eq!(number("q.any(1, 1, 2)"), 1.0);
            assert_eq!(number("q.any(1, 2, 1)"), 1.0);
            assert_eq!(number("q.any(1, 2, 3)"), 0.0);
            assert_eq!(number("q.any(v.x, 3, 1.5)"), 1.0);
            assert_eq!(number("q.any('moo', 'cow', 'moo')"), 1.0);
            assert_eq!(number("q.any('moo', 'cow', 3)"), 0.0);
            assert_eq!(number("q.any(v.x, v.s, 5)"), 0.0, "a number is no string");
        }

        #[test]
        fn any_and_all_compare_floats_by_their_bits() {
            assert_eq!(number("q.any(0, 0 * -1, 1)"), 0.0, "0 and -0 differ");
            assert_eq!(number("q.any(v.n, v.n, 1)"), 1.0, "a NaN is the same NaN");
            assert_eq!(number("q.all(v.n, v.n, v.n)"), 1.0);
            assert_eq!(number("q.all(0, 0, 0 * -1)"), 0.0);
        }

        #[test]
        fn all_is_one_when_every_later_argument_equals_the_first() {
            assert_eq!(number("q.all(1, 1, 1)"), 1.0);
            assert_eq!(number("q.all(1, 1, 2)"), 0.0);
            assert_eq!(number("q.all(1, 2, 1)"), 0.0);
            assert_eq!(number("q.all('moo', 'moo', 'moo')"), 1.0);
            assert_eq!(number("q.all(v.x, 1.5, v.x, 1.5)"), 1.0);
        }

        #[test]
        fn any_and_all_ask_for_three_arguments() {
            for (src, text) in [
                ("q.any(1)", "query.any takes at least three arguments"),
                ("q.all(1, 2)", "query.all takes at least three arguments"),
            ] {
                let mut env = xorshift_env();
                assert_eq!(run(src, &mut env), Value::ZERO, "{src}");
                assert!(
                    env.sink.messages.iter().any(|m| m.contains(text)),
                    "{src}: {:?}",
                    env.sink.messages
                );
            }
        }

        #[test]
        fn in_range_is_inclusive_at_both_ends() {
            assert_eq!(number("q.in_range(5, 1, 10)"), 1.0);
            assert_eq!(number("q.in_range(1, 1, 10)"), 1.0);
            assert_eq!(number("q.in_range(10, 1, 10)"), 1.0);
            assert_eq!(number("q.in_range(11, 1, 10)"), 0.0);
            assert_eq!(number("q.in_range(0, 1, 10)"), 0.0);
            assert_eq!(number("q.in_range(v.x, 1, 2)"), 1.0);
            assert_eq!(number("q.in_range(v.n, 1, 2)"), 0.0, "NaN is in no range");
        }

        #[test]
        fn in_range_needs_exactly_three_arguments() {
            for src in ["q.in_range(1, 2)", "q.in_range(1, 2, 3, 4)"] {
                let mut env = xorshift_env();
                assert_eq!(run(src, &mut env), Value::ZERO, "{src}");
                assert!(
                    env.sink
                        .messages
                        .iter()
                        .any(|m| m.contains("query.in_range takes three numbers")),
                    "{src}: {:?}",
                    env.sink.messages
                );
            }
        }

        #[test]
        fn log_returns_its_first_argument_and_evaluates_all_of_them() {
            assert_eq!(number("q.log(7, 8)"), 7.0);
            assert_eq!(value("q.log('moo', 8)"), Value::string("moo"));
            // Every argument is evaluated, in order: each random draw moves the generator.
            let mut env = xorshift_env();
            run("q.log(1, math.random(0, 1), math.random(0, 1))", &mut env);
            let mut reference = Xorshift128::new();
            sample(&mut reference);
            sample(&mut reference);
            assert_eq!(env.rng, FuzzRng::Xorshift(reference));
            // The first one is the value, and it is drawn first.
            let mut env = xorshift_env();
            let first = run("q.log(math.random(0, 1), math.random(0, 1))", &mut env).as_f32();
            assert_eq!(first.to_bits(), sample(&mut Xorshift128::new()).to_bits());
        }

        #[test]
        fn any_stops_evaluating_at_the_first_match_and_all_at_the_first_mismatch() {
            assert_eq!(
                draws("q.any(1, 1, math.random(0, 1))"),
                0,
                "matched before the draw"
            );
            assert_eq!(draws("q.any(1, 2, math.random(0, 1))"), 1);
            assert_eq!(
                draws("q.all(1, 2, math.random(0, 1))"),
                0,
                "mismatched before the draw"
            );
            assert_eq!(draws("q.all(1, 1, math.random(0, 1))"), 1);
            assert_eq!(
                draws("q.count(math.random(0, 1), math.random(0, 1))"),
                2,
                "count evaluates all"
            );
        }

        #[test]
        fn arguments_cost_steps_only_when_the_query_evaluates_them() {
            let steps = |src: &str| {
                let compiled = compile(src, &options());
                let expr = compiled
                    .expr()
                    .cloned()
                    .unwrap_or_else(|| panic!("{src:?}"));
                (0..200u64)
                    .find(|&budget| {
                        let limits = EvalLimits {
                            total_steps: Some(budget),
                            ..EvalLimits::DEFAULT
                        };
                        let mut env = FuzzEnv::new(
                            Subject::Actor,
                            TempLifetime::PerEvaluation,
                            limits,
                            FuzzRng::Fixed(FixedRng::HALF),
                        );
                        expr.eval(&mut env.cx());
                        env.sink.messages.is_empty()
                    })
                    .expect("a budget that completes")
            };
            let one = steps("q.log(1)");
            let two = steps("q.log(1, 2)");
            let three = steps("q.log(1, 2, 3)");
            assert!(one < two && two < three, "{one} {two} {three}");
            assert_eq!(
                two - one,
                three - two,
                "each further argument costs the same"
            );
            // `any` skips what comes after its match.
            assert!(steps("q.any(1, 1, 2)") < steps("q.any(1, 2, 2)"));
        }

        #[test]
        fn the_test_queries_return_what_their_names_say() {
            assert_eq!(value("q.get_name_test(1)"), Value::string("rabbit"));
            assert_eq!(value("q.get_name_test(0)"), Value::string("moo"));
            assert_eq!(
                value("q.get_name_test(2)"),
                Value::string("moo"),
                "only index 1 is the rabbit"
            );
            assert_eq!(value("q.get_name_test(v.x - 0.5)"), Value::string("rabbit"));
            assert_eq!(value("q.get_name_test(v.x - 1)"), Value::string("moo"));
            assert_eq!(number("q.sum_test(1, 2, 3)"), 6.0);
            assert_eq!(number("q.sum_test(1)"), 1.0);
            assert_eq!(number("q.sum_test(v.x, c.n, -1)"), 4.5);
            assert_eq!(number("q.experimental_test"), 1.0);
        }

        #[test]
        fn the_test_queries_evaluate_the_arguments_they_use_and_count_them() {
            // `sum_test` evaluates every argument, `get_name_test` only the first,
            // `experimental_test` none.
            assert_eq!(
                draws("q.sum_test(math.random(0, 1), math.random(0, 1), math.random(0, 1))"),
                3
            );
            assert_eq!(draws("q.get_name_test(math.random(0, 1))"), 1);
            assert_eq!(
                draws("q.get_name_test(1, math.random(0, 1))"),
                0,
                "the second argument is never asked for"
            );
            assert_eq!(draws("q.experimental_test"), 0);
            // The value is the sum of the values drawn.
            let mut env = xorshift_env();
            let total = run("q.sum_test(math.random(0, 1), math.random(0, 1))", &mut env).as_f32();
            let mut reference = Xorshift128::new();
            let expected = sample(&mut reference) + sample(&mut reference);
            assert_eq!(total.to_bits(), expected.to_bits());
        }
    }
}
