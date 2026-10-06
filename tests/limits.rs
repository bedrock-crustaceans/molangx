//! Limits at the limit and one past it, hostile input, native stack needs and the evaluation
//! budgets.
//!
//! A native stack overflow aborts the test process, so a stack test fails by crashing.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

mod common;

/// A wall-clock bound of `millis`, times the whole-number `MOLANGX_TIME_SCALE` (for an emulator).
fn wall_clock(millis: u64) -> std::time::Duration {
    let scale = std::env::var("MOLANGX_TIME_SCALE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1);
    std::time::Duration::from_millis(millis * scale)
}

mod hostile_input {
    use crate::common::compile_support::{at, client_at, messages};
    use molangx::compile::{
        CompileFailure, CompileOptions, Deviations, Expr, MAX_DEPTH, MAX_SOURCE_LEN, compile,
    };
    use molangx::diag::{DiagCode, Severity};

    const DEPTH: &str = "Error: Expression could not be parsed due to stack depth overflow (too many sub-expressions)";

    fn parses(source: &str) -> bool {
        let compiled = at(source, 13);
        if compiled.failure() == Some(CompileFailure::Rejected) {
            assert_eq!(messages(&compiled), [DEPTH], "rejected for another reason");
            return false;
        }
        true
    }

    fn repeat(prefix: &str, count: usize, middle: &str, suffix: &str) -> String {
        format!("{}{middle}{}", prefix.repeat(count), suffix.repeat(count))
    }

    /// `2^levels` copies of `term` summed in a balanced tree, which keeps the nesting shallow.
    fn balanced_sum(term: &str, levels: usize) -> String {
        (0..levels).fold(term.to_owned(), |sum, _| format!("({sum}+{sum})"))
    }

    #[test]
    fn nesting_at_the_limit() {
        let limit = MAX_DEPTH as usize;
        assert_eq!(limit, 255);
        assert!(parses(&repeat("(", limit, "1", ")")));
        assert!(!parses(&repeat("(", limit + 1, "1", ")")));
        assert!(parses(&repeat("[", limit, "1", "]")));
        assert!(!parses(&repeat("[", limit + 1, "1", "]")));
        assert!(parses(&repeat("!", limit, "1", "")));
        assert!(!parses(&repeat("!", limit + 1, "1", "")));
        assert!(parses(&repeat("array.a[", limit, "0", "]")));
        assert!(!parses(&repeat("array.a[", limit + 1, "0", "]")));
    }

    /// One level per operator, counted before `+` is flattened.
    #[test]
    fn operator_chains_at_the_limit() {
        let chain =
            |operand: &str, operator: &str, operands: usize| vec![operand; operands].join(operator);
        assert!(parses(&chain("v.x", " + ", 256)));
        assert!(!parses(&chain("v.x", " + ", 257)));
        assert!(parses(&chain("1", " + ", 256)));
        assert!(!parses(&chain("1", " + ", 257)));
        assert!(parses(&chain("v.a", " * ", 256)));
        assert!(!parses(&chain("v.a", " * ", 257)));
        let ternary = |depth: usize| format!("{}1", "v.c ? ".repeat(depth));
        assert!(parses(&ternary(255)));
        assert!(!parses(&ternary(256)));
    }

    /// A block level costs three levels and a call two.
    #[test]
    fn blocks_and_calls_at_the_limit() {
        let blocks = |depth: usize| {
            format!(
                "{}v.x = 1;{}",
                "v.c ? { ".repeat(depth),
                " };".repeat(depth)
            )
        };
        assert!(parses(&blocks(84)));
        assert!(!parses(&blocks(85)));
        let loops = |depth: usize| {
            format!(
                "{}v.x = 1;{}",
                "loop(1, {".repeat(depth),
                "});".repeat(depth)
            )
        };
        assert!(parses(&loops(84)));
        assert!(!parses(&loops(85)));
        assert!(parses(&repeat("math.abs(", 127, "1", ")")));
        assert!(!parses(&repeat("math.abs(", 128, "1", ")")));
        let assignments =
            |depth: usize| format!("{}1{};", "v.a = (".repeat(depth), ")".repeat(depth));
        assert!(parses(&assignments(127)));
        assert!(!parses(&assignments(128)));
    }

    /// Nested query calls are three levels deep while grouped and one while folded: 255 nest bare,
    /// 253 under an assignment or `return`.
    #[test]
    fn grouping_passes_add_no_limit() {
        let calls = |depth: usize| repeat("q.is_baby(1, ", depth, "1", ")");
        assert!(parses(&calls(255)));
        assert!(!parses(&calls(256)));
        assert!(parses(&format!("v.y = {};", calls(253))));
        assert!(!parses(&format!("v.y = {};", calls(254))));
        assert!(parses(&format!("return {};", calls(253))));
        assert!(!parses(&format!("return {};", calls(254))));
    }

    #[test]
    fn breadth_is_not_limited() {
        let statements = "v.x=1;".repeat(3000);
        assert_eq!(at(&statements, 13).failure(), None);

        let arguments = vec!["1"; 3000].join(", ");
        let call = at(&format!("query.count({arguments})"), 13);
        assert_eq!(call.failure(), None, "{:?}", messages(&call));
        // The same list under passes that walk the tree after the `,` pass.
        let returned = at(&format!("return query.count({arguments});"), 13);
        assert_eq!(returned.failure(), None, "{:?}", messages(&returned));
        let assigned = at(&format!("v.x = query.count({arguments});"), 13);
        assert_eq!(assigned.failure(), None, "{:?}", messages(&assigned));
        // A math function still counts its arguments.
        let too_many = at(&format!("math.max({arguments})"), 13);
        assert_eq!(
            messages(&too_many),
            [
                "Unexpected number of parameters to Max 'math.max' function - expected 2, found 3000."
            ]
        );
    }

    /// `Deviations::NONE` sets no length limit.
    #[test]
    fn source_length() {
        let padding = |len: usize| format!("{}1", " ".repeat(len - 1));
        let at_limit = at(&padding(MAX_SOURCE_LEN), 13);
        assert_eq!(at_limit.failure(), None);
        assert_eq!(at_limit.expr().and_then(Expr::as_constant), Some(1.0));

        let over = at(&padding(MAX_SOURCE_LEN + 1), 13);
        assert_eq!(over.failure(), Some(CompileFailure::Rejected));
        assert_eq!(over.diagnostics().len(), 1);
        assert_eq!(
            (
                over.diagnostics()[0].code(),
                over.diagnostics()[0].severity()
            ),
            (DiagCode::SourceTooLong, Severity::Error)
        );
        assert!(over.diagnostics()[0].language_message().is_none());

        let no_deviations = compile(
            &padding(MAX_SOURCE_LEN + 1),
            &CompileOptions {
                deviations: Deviations::NONE,
                ..client_at(13)
            },
        );
        assert_eq!(no_deviations.failure(), None);
    }

    /// `count` distinct three-letter names from `first` on (`aaa`, `aab`, …).
    fn names(first: usize, count: usize) -> Vec<String> {
        (first..first + count)
            .map(|i| {
                let letter = |k: usize| char::from(b'a' + u8::try_from(k % 26).expect("letter"));
                format!("{}{}{}", letter(i / 676), letter(i / 26), letter(i))
            })
            .collect()
    }

    /// `groups` parenthesised sums of `size` entity variables joined by `+`; with `distinct` every
    /// group has its own variables, otherwise all groups repeat the same ones.
    fn sum_of_groups(groups: usize, size: usize, distinct: bool) -> String {
        (0..groups)
            .map(|g| {
                let first = if distinct { g * size } else { 0 };
                format!(
                    "({})",
                    names(first, size)
                        .iter()
                        .map(|name| format!("v.{name}"))
                        .collect::<Vec<_>>()
                        .join("+")
                )
            })
            .collect::<Vec<_>>()
            .join("+")
    }

    /// Hostile input of every shape, each at most `MAX_SOURCE_LEN` bytes.
    fn hostile_inputs() -> Vec<String> {
        let n = MAX_SOURCE_LEN;
        let mut hostile: Vec<String> = vec![
            "(".repeat(n),
            ")".repeat(n),
            repeat("(", n / 2, "", ")"),
            repeat("[", n / 2 - 1, "1", "]"),
            "{".repeat(n),
            repeat("{", n / 2 - 1, "1;", "}"),
            "!".repeat(n),
            format!("{}1", "!".repeat(n - 1)),
            "-".repeat(n),
            format!("{}1", "-".repeat(n - 1)),
            format!("{}1", "- ".repeat(n / 2 - 1)),
            format!("{}1", "!-".repeat(n / 2 - 1)),
            ";".repeat(n),
            "1;".repeat(n / 2),
            format!("v.x{}", ".y".repeat(n / 2 - 2)),
            format!("{}1", "1+".repeat(n / 2 - 1)),
            format!("{}1", "1*".repeat(n / 2 - 1)),
            format!("{}1", "1/".repeat(n / 2 - 1)),
            format!("{}1", "1<".repeat(n / 2 - 1)),
            format!("{}1", "1?".repeat(n / 2 - 1)),
            format!("{}1", "1?1:".repeat(n / 4 - 1)),
            format!("{}1", "1:".repeat(n / 2 - 1)),
            format!("{}1;", "v.a=".repeat(n / 4 - 1)),
            format!("{}1", "v.a??".repeat(n / 5 - 1)),
            format!("{}v.a", "v.a->".repeat(n / 5 - 1)),
            format!("return {}1;", "1,".repeat(n / 2 - 5)),
            format!("{}1", "return ".repeat(n / 7 - 1)),
            repeat("math.abs(", n / 10, "1", ")"),
            repeat("q.count(", n / 9, "1", ")"),
            repeat("q.is_baby(1, ", n / 14, "1", ")"),
            format!("v.y = {};", repeat("q.is_baby(1, ", n / 14 - 1, "1", ")")),
            repeat("loop(1,{", n / 11, "1;", "});"),
            repeat("v.c ? {", n / 9 - 1, "v.x = 1;", "};"),
            repeat("1?1:(", n / 6, "1", ")"),
            format!("1?{}", repeat("(", n / 2 - 2, "1", ")")),
            repeat("array.a[", n / 9, "0", "]"),
            format!("array.a{}", "[0]".repeat(n / 3 - 3)),
            format!("{}1", "v.x&&".repeat(n / 5 - 1)),
            format!("({}1)", "v.x+(".repeat(n / 6)),
            format!("q.count({})", vec!["1"; n / 2 - 5].join(",")),
            format!("math.max({})", vec!["1"; n / 2 - 6].join(",")),
            format!("v.x = q.count({});", vec!["v.a"; n / 4 - 5].join(",")),
            "'".repeat(n),
            "'\\".repeat(n / 2),
            "9".repeat(n),
            format!("1e{}", "9".repeat(n - 2)),
            format!("0.{}", "9".repeat(n - 2)),
            "t".repeat(n),
            ",".repeat(n),
            "=".repeat(n),
            "?".repeat(n),
            ":".repeat(n),
            "[]".repeat(n / 2),
            // Terms of `+` that merge, or not, across 200 levels of nesting (54 terms per level).
            sum_of_groups(200, 54, false),
            sum_of_groups(200, 54, true),
            format!("{}0", "v.a-v.a+".repeat(n / 8 - 1)),
            // The term text of a `v.a.b…` chain doubles with every member.
            format!("{0} + {0}", format!("v.a{}", ".b".repeat(250))),
            balanced_sum(&format!("v.a{}", ".b".repeat(8)), 11),
            balanced_sum(&format!("v.a{}", ".b".repeat(40)), 9),
            // One struct level per iteration (the `deep_structs` tests below run them without
            // budgets).
            "v.a.c = 1; loop(1000, {loop(1000, { v.a.b = v.a; }); }); return 1;".to_owned(),
            "t.a.c = 1; loop(1000, {loop(1000, { t.a.b = t.a; }); }); return 1;".to_owned(),
            "v.a.c = 1; loop(1000, {loop(1000, { v.a.b = v.a; v.a.d = v.a; }); }); return v.a == v.a.b;".to_owned(),
        ];
        // Deep parentheses under every binary pass.
        for operator in [
            "->v.x", "/2", "*2", "+v.b", ",1", "<1", "==1", "&&1", "||1", "??1", ".b", "?1:2",
        ] {
            hostile.push(format!(
                "{}{operator}",
                repeat("(", (n - operator.len() - 3) / 2, "v.a", ")")
            ));
        }
        hostile.push(format!("v.x = {};", repeat("(", n / 2 - 6, "1", ")")));
        for source in &hostile {
            assert!(
                source.len() <= MAX_SOURCE_LEN,
                "{:?}… is {} bytes",
                &source[..24],
                source.len()
            );
        }
        hostile
    }

    /// Compiles (and with `vm` evaluates) every hostile input at three versions under both
    /// deviation settings; returns the time of each, slowest first.
    fn compile_hostile_inputs() -> Vec<(std::time::Duration, String)> {
        let mut times = Vec::new();
        for source in &hostile_inputs() {
            for version in [-1, 4, 13] {
                for deviations in [Deviations::ALL, Deviations::NONE] {
                    let started = std::time::Instant::now();
                    let compiled = compile(
                        source,
                        &CompileOptions {
                            deviations,
                            ..client_at(version)
                        },
                    );
                    #[cfg(feature = "vm")]
                    if let Some(expr) = compiled.expr().filter(|_| compiled.is_success()) {
                        std::hint::black_box(expr.eval(&mut molangx::vm::NoHostEnv::new().cx()));
                    }
                    let took = started.elapsed();
                    let label = &source[..source.len().min(24)];
                    times.push((took, format!("{label:?}… v{version}")));
                    if compiled.failure() == Some(CompileFailure::Rejected) {
                        assert!(
                            !compiled.diagnostics().is_empty(),
                            "{label:?}… rejected without a diagnostic"
                        );
                    }
                }
            }
        }
        times.sort_by_key(|time| std::cmp::Reverse(time.0));
        times
    }

    /// Every input of up to 64 KiB within 2 s in an unoptimised build; a quadratic step costs tens
    /// of seconds there. The quietest of up to three timings counts, so a busy machine does not
    /// fail it.
    #[test]
    fn hostile_input_compiles_in_linear_time() {
        let bound = crate::wall_clock(2_000);
        let mut quietest = compile_hostile_inputs();
        for _ in 0..2 {
            if quietest[0].0 < bound {
                break;
            }
            let again = compile_hostile_inputs();
            if again[0].0 < quietest[0].0 {
                quietest = again;
            }
        }
        for (took, label) in quietest.iter().take(8) {
            println!("{took:>12.3?}  {label}");
        }
        let (slowest, label) = &quietest[0];
        assert!(*slowest < bound, "{label} took {slowest:?}");
    }

    /// The whole hostile list compiles and evaluates on a 512 KiB thread in an unoptimised build;
    /// `MOLANGX_HOSTILE_STACK_KIB` overrides the size.
    #[test]
    fn hostile_input_compiles_on_a_small_stack() {
        let kib = std::env::var("MOLANGX_HOSTILE_STACK_KIB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(512);
        let worker = std::thread::Builder::new()
            .stack_size(kib * 1024)
            .spawn(compile_hostile_inputs)
            .expect("cannot spawn the test thread");
        worker.join().expect("the hostile list panicked");
    }

    /// Deep nesting gets the depth message whichever pass meets it first.
    #[test]
    fn deep_nesting_reports_the_depth_limit() {
        let n = MAX_SOURCE_LEN;
        for source in [
            format!("{}1", "!".repeat(n - 1)),
            repeat("(", n / 2 - 1, "1", ")"),
            format!("{}1 + 1", "!".repeat(n - 5)),
            format!("return {}1;", "!".repeat(n - 9)),
        ] {
            let compiled = at(&source, 13);
            assert_eq!(messages(&compiled), [DEPTH], "{:?}…", &source[..16]);
        }
    }
}

mod compile_stack {
    //! The native stack one compile needs at the deepest nesting it accepts.
    //!
    //! Passes over unbounded input use explicit stacks; the helpers that walk an accepted tree
    //! recurse natively, and nested query calls make the deepest frames.

    use std::thread;

    use molangx::compile::{CompileFailure, CompileOptions, MAX_DEPTH, compile};
    use molangx::version::MolangVersion;

    fn nested_calls(depth: usize) -> String {
        format!("{}1{}", "q.is_baby(".repeat(depth), ")".repeat(depth))
    }

    /// Other shapes of deep nesting, each at its deepest accepted level.
    const SHAPES: [(&str, &str, &str); 7] = [
        ("(", "1", ")"),
        ("math.abs(", "1", ")"),
        ("-(", "v.x", ")"),
        ("!(", "v.x", ")"),
        ("v.x ? (", "1", ") : 2"),
        ("loop(1, {", "v.x = 1;", "});"),
        ("math.clamp(v.x, 0, ", "1", ")"),
    ];

    fn deepest(open: &str, inner: &str, close: &str) -> Option<String> {
        (1..=MAX_DEPTH as usize)
            .rev()
            .map(|d| format!("{}{inner}{}", open.repeat(d), close.repeat(d)))
            .find(|src| compile(src, &CompileOptions::server(MolangVersion::LATEST)).is_success())
    }

    fn compile_on(stack: usize, src: String) -> Option<CompileFailure> {
        thread::Builder::new()
            .stack_size(stack)
            .spawn(move || compile(&src, &CompileOptions::server(MolangVersion::LATEST)).failure())
            .expect("spawn")
            .join()
            .expect("the compile thread")
    }

    /// `MOLANGX_STACK_PROBE` (bytes) overrides the size: an optimised x86-64 build needs at most
    /// 128 KiB, an unoptimised one at most 384 KiB.
    #[test]
    fn the_deepest_accepted_nesting_fits_in_512_kib() {
        let stack = std::env::var("MOLANGX_STACK_PROBE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512 * 1024);
        let deepest_calls = (1..=MAX_DEPTH as usize).rev().find(|&d| {
            compile(
                &nested_calls(d),
                &CompileOptions::server(MolangVersion::LATEST),
            )
            .is_success()
        });
        let depth = deepest_calls.expect("some nesting compiles");
        assert!(depth >= 250, "{depth}");
        assert_eq!(compile_on(stack, nested_calls(depth)), None);
        // One level more is rejected (the depth limit), on the same stack.
        assert_eq!(
            compile_on(stack, nested_calls(depth + 1)),
            Some(CompileFailure::Rejected)
        );
        for (open, inner, close) in SHAPES {
            let src = deepest(open, inner, close)
                .unwrap_or_else(|| panic!("{open:?} compiles at some depth"));
            assert_eq!(compile_on(stack, src), None, "{open:?}");
        }
    }
}

#[cfg(feature = "vm")]
mod eval_budgets {
    use crate::common::compile_support::at;
    use molangx::compile::CompileFailure;
    use molangx::hash::HashedStr;

    /// Structs an expression nests without bound: dropping, comparing and printing them must not
    /// recurse.
    mod deep_structs {
        use molangx::compile::{CompileOptions, Expr, compile};
        use molangx::hash::HashedStr;
        use molangx::version::MolangVersion;
        use molangx::vm::{
            EvalLimits, NoHostEnv, StructValue, TempMap, TempName, Temps, Value, VariableName,
        };

        /// A stack far below the main thread's 8 MiB, so recursion on depth would overflow it.
        const SMALL_STACK: usize = 512 * 1024;

        fn on_small_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
            std::thread::Builder::new()
                .stack_size(SMALL_STACK)
                .spawn(f)
                .expect("thread")
                .join()
                .expect("no panic, no overflow")
        }

        fn expr(source: &str) -> Expr {
            compile(source, &CompileOptions::server(MolangVersion::LATEST))
                .expr()
                .cloned()
                .expect("compiles")
        }

        /// A struct `depth` levels deep, built through the API: `{b: {b: … {c: 1} …}}`.
        fn deep(depth: usize, leaf: f32) -> Value<molangx::vm::NoHost> {
            let mut value = Value::structure(StructValue::from([("c", leaf)]));
            for _ in 1..depth {
                value =
                    Value::structure(StructValue::from([("b", value), ("c", Value::Float(leaf))]));
            }
            value
        }

        const DEPTH_LIMIT: &str =
            "molangx: evaluation stopped: a struct would nest deeper than its budget of 32 levels";

        /// With the default budgets the struct depth budget stops the loop at the store that would
        /// make the 33rd level. Without budgets the loop runs 100,000 times (100,001 levels, about
        /// 13 MB; a recursive drop overflows 512 KiB after a few thousand).
        #[test]
        fn deep_struct_from_an_expression_is_dropped_safely() {
            let sources = [
                (
                    "v",
                    "v.a.c = 1; loop(1000, {loop(1000, { v.a.b = v.a; }); }); return 1;",
                ),
                (
                    "t",
                    "t.a.c = 1; loop(1000, {loop(1000, { t.a.b = t.a; }); }); return 1;",
                ),
            ];
            for (root, source) in sources {
                for persistent_temps in [false, true] {
                    for limits in [EvalLimits::DEFAULT, EvalLimits::NONE] {
                        let source = if limits == EvalLimits::NONE {
                            source.replacen("loop(1000", "loop(100", 1)
                        } else {
                            source.to_owned()
                        };
                        let expr = expr(&source);
                        let (result, messages, depth) = on_small_stack(move || {
                            let mut env = NoHostEnv::new();
                            if persistent_temps {
                                env.temps = Temps::Kept(TempMap::new());
                            }
                            env.limits = limits;
                            let result = expr.eval_f32(&mut env.cx());
                            let messages = env.sink.take();
                            let stored = match root {
                                "v" => env.variables.get(VariableName::new("a")),
                                _ => env
                                    .temps
                                    .kept()
                                    .and_then(|temps| temps.get(TempName::new("a"))),
                            };
                            let depth = stored.map(Value::struct_depth);
                            drop(env);
                            (result, messages, depth)
                        });
                        let label =
                            format!("{root} persistent temps {persistent_temps} {limits:?}");
                        let visible = root == "v" || persistent_temps;
                        if limits == EvalLimits::NONE {
                            assert_eq!((result, messages.len()), (1.0, 0), "{label}: {messages:?}");
                            assert_eq!(depth, visible.then_some(100_001), "{label}");
                        } else {
                            assert_eq!(
                                (result, messages),
                                (0.0, vec![DEPTH_LIMIT.to_owned()]),
                                "{label}"
                            );
                            assert_eq!(depth, visible.then_some(32), "{label}");
                        }
                    }
                }
            }
        }

        /// 32 members nest and are written; a 33rd stops the evaluation with 0 and writes nothing.
        #[test]
        fn struct_depth_at_the_limit() {
            assert_eq!(
                EvalLimits::DEFAULT.struct_depth,
                Some(EvalLimits::DEFAULT_STRUCT_DEPTH)
            );
            assert_eq!(EvalLimits::DEFAULT_STRUCT_DEPTH, 32);
            assert_eq!(EvalLimits::NONE.struct_depth, None);
            let store = |members: usize| format!("v.x{} = 1; return 7;", ".m".repeat(members));
            let mut env = NoHostEnv::new();
            assert_eq!(expr(&store(32)).eval_f32(&mut env.cx()), 7.0);
            assert!(env.sink.is_empty());
            assert_eq!(
                env.variables
                    .get(VariableName::new("x"))
                    .map(Value::struct_depth),
                Some(32)
            );
            let mut env = NoHostEnv::new();
            assert_eq!(expr(&store(33)).eval_f32(&mut env.cx()), 0.0);
            assert_eq!(env.sink.take(), [DEPTH_LIMIT]);
            assert_eq!(env.variables.get(VariableName::new("x")), None);
            // Storing a 32-deep value one level down is 33 levels.
            let mut env = NoHostEnv::new();
            let nested =
                |members: usize| format!("v.x{} = 1; v.y.z = v.x; return 7;", ".m".repeat(members));
            assert_eq!(expr(&nested(31)).eval_f32(&mut env.cx()), 7.0);
            assert!(env.sink.is_empty());
            assert_eq!(expr(&nested(32)).eval_f32(&mut env.cx()), 0.0);
            assert_eq!(env.sink.take(), [DEPTH_LIMIT]);
            let mut env = NoHostEnv {
                limits: EvalLimits::NONE,
                ..NoHostEnv::new()
            };
            assert_eq!(expr(&store(200)).eval_f32(&mut env.cx()), 7.0);
            assert_eq!(
                env.variables
                    .get(VariableName::new("x"))
                    .map(Value::struct_depth),
                Some(200)
            );
        }

        /// A 100,000-deep struct (about 13 MB) is cloned, compared, printed, walked, written and
        /// dropped on a 512 KiB stack.
        #[test]
        fn deep_struct_built_through_the_api() {
            const DEPTH: usize = 100_000;
            for limits in [EvalLimits::DEFAULT, EvalLimits::NONE] {
                on_small_stack(move || {
                    let value = deep(DEPTH, 1.0);
                    let copy = value.clone();
                    assert_eq!(value, copy);
                    assert_eq!(value, deep(DEPTH, 1.0));
                    assert_ne!(value, deep(DEPTH, 2.0));
                    let printed = format!("{value:?}");
                    assert!(
                        printed.contains("Struct(..)") && printed.len() < 10_000,
                        "{}",
                        printed.len()
                    );
                    let b = HashedStr::new("b");
                    let path = vec![b; DEPTH - 1];
                    assert_eq!(
                        value
                            .member_path(&path)
                            .and_then(|v| v.member(HashedStr::new("c"))),
                        Some(&Value::Float(1.0))
                    );
                    let mut written = copy;
                    written.set_member_path(&path, Value::Float(5.0));
                    assert_eq!(written.member_path(&path), Some(&Value::Float(5.0)));
                    assert_eq!(
                        value
                            .member_path(&path)
                            .and_then(|v| v.member(HashedStr::new("c"))),
                        Some(&Value::Float(1.0)),
                        "copy-on-write"
                    );
                    drop(written);

                    assert_eq!(value.struct_depth(), 100_000);

                    // A member store into it is over the default budget, and written without one.
                    let mut env = NoHostEnv {
                        limits,
                        ..NoHostEnv::new()
                    };
                    env.variables.set(VariableName::new("deep"), value);
                    let read = expr("v.copy = v.deep; v.deep.x = 2; return v.deep.c;")
                        .eval_f32(&mut env.cx());
                    let deep = env.variables.get(VariableName::new("deep")).expect("set");
                    if limits == EvalLimits::NONE {
                        assert_eq!(read, 1.0);
                        assert_eq!(deep.member(HashedStr::new("x")), Some(&Value::Float(2.0)));
                    } else {
                        assert_eq!((read, env.sink.take()), (0.0, vec![DEPTH_LIMIT.to_owned()]));
                        assert_eq!(deep.member(HashedStr::new("x")), None);
                    }
                    assert!(env.variables.get(VariableName::new("copy")).is_some());
                    drop(env);
                });
            }
        }
    }

    /// Copying a wide struct again and again stays within memory proportional to the step budget.
    mod struct_copies {
        use std::collections::HashSet;
        use std::fmt::Write as _;
        use std::time::{Duration, Instant};

        use molangx::compile::{CompileOptions, Deviations, Expr, compile};
        use molangx::version::MolangVersion;
        use molangx::vm::{
            ContextName, EvalLimits, NoHost, NoHostEnv, StructValue, Value, VariableMap,
            VariableName,
        };

        const WIDTH_LIMIT: &str =
            "molangx: evaluation stopped: a struct would hold more than its budget of 256 members";
        const STEP_LIMIT: &str = "molangx: evaluation stopped after its budget of 1048576 steps";

        /// Compiled without the source-length cap (one repro is longer than 64 KiB).
        fn expr(source: &str) -> Expr {
            compile(
                source,
                &CompileOptions {
                    deviations: Deviations::NONE,
                    ..CompileOptions::server(MolangVersion::LATEST)
                },
            )
            .expr()
            .cloned()
            .expect("compiles")
        }

        /// Members reachable from `map`, each distinct struct counted once; stops at `cap`.
        fn retained_members(map: &VariableMap<NoHost>, cap: usize) -> usize {
            let mut seen: HashSet<*const StructValue<NoHost>> = HashSet::new();
            let mut pending: Vec<&Value<NoHost>> = map.iter().map(|(_, value)| value).collect();
            let mut members = 0;
            while let Some(value) = pending.pop() {
                if let Value::Struct(members_of) = value
                    && seen.insert(std::sync::Arc::as_ptr(members_of))
                {
                    members += members_of.len();
                    if members >= cap {
                        return members;
                    }
                    pending.extend(members_of.iter().map(|(_, member)| member));
                }
            }
            members
        }

        /// `v.s` made `width` members wide, then 31 times: `v.t = v.s` and `copies` stores of `v.t`
        /// into new members of `v.s`, each followed by a store into the stored copy (which copies
        /// it).
        fn looped(width: usize, copies: usize) -> String {
            let mut source = String::new();
            for i in 0..width {
                let _ = write!(source, "v.s.m{i}=1;");
            }
            source.push_str("loop(31,{ v.t=v.s;");
            for k in 0..copies {
                let _ = write!(source, " v.s.c{k}=v.t; v.s.c{k}.q=1;");
            }
            source.push_str(" }); return 1;");
            source
        }

        /// `v.s` made `width` members wide, then `copies` variables each given a copy of it with
        /// one more member.
        fn flat(width: usize, copies: usize) -> String {
            let mut source = String::new();
            for i in 0..width {
                let _ = write!(source, "v.s.m{i}=1;");
            }
            for k in 0..copies {
                let _ = write!(source, "v.c{k}=v.s; v.c{k}.q=1;");
            }
            source.push_str("return 1;");
            source
        }

        /// The result, the messages, the time and the members the variables keep.
        fn run(source: &str, limits: EvalLimits) -> (f32, Vec<String>, Duration, usize) {
            let expr = expr(source);
            let mut env = NoHostEnv {
                limits,
                ..NoHostEnv::new()
            };
            let started = Instant::now();
            let result = expr.eval_f32(&mut env.cx());
            let took = started.elapsed();
            let retained = retained_members(&env.variables, 4_000_000);
            (result, env.sink.take(), took, retained)
        }

        /// Under the defaults the width budget stops these while `v.s` is built; with it off the
        /// step budget, which charges every member a copy may copy, stops them. The variables keep
        /// at most a million members.
        #[test]
        fn wide_struct_copies_are_bounded() {
            let repros = [
                ("2500 x 1000 looped", looped(2500, 1000)),
                ("1000 x 300 looped", looped(1000, 300)),
                ("2700 x 3000 flat", flat(2700, 3000)),
            ];
            let width_off = EvalLimits {
                struct_members: None,
                ..EvalLimits::DEFAULT
            };
            let unlimited_with_steps = EvalLimits {
                total_steps: Some(EvalLimits::DEFAULT_TOTAL_STEPS),
                ..EvalLimits::NONE
            };
            for (label, source) in &repros {
                let (result, messages, took, retained) = run(source, EvalLimits::DEFAULT);
                assert_eq!(
                    (result, messages),
                    (0.0, vec![WIDTH_LIMIT.to_owned()]),
                    "{label}"
                );
                assert!(took < crate::wall_clock(500), "{label}: {took:?}");
                assert_eq!(retained, 256, "{label}: v.s keeps its first 256 members");

                for limits in [width_off, unlimited_with_steps] {
                    let (result, messages, took, retained) = run(source, limits);
                    assert_eq!(
                        (result, messages),
                        (0.0, vec![STEP_LIMIT.to_owned()]),
                        "{label} {limits:?}"
                    );
                    assert!(
                        took < crate::wall_clock(3_000),
                        "{label} {limits:?}: {took:?}"
                    );
                    assert!(
                        retained <= EvalLimits::DEFAULT_TOTAL_STEPS as usize,
                        "{label} {limits:?}: {retained} members kept"
                    );
                }
            }
        }

        /// The step budget bounds what is copied (each copy costs its members plus 4), with or
        /// without the width budget.
        #[test]
        fn copies_of_a_struct_under_the_width_budget_are_bounded() {
            let source = looped(250, 250);
            let (result, messages, _, retained) = run(&source, EvalLimits::DEFAULT);
            assert_eq!(
                (result, messages),
                (0.0, vec![WIDTH_LIMIT.to_owned()]),
                "250 + 250 members pass the budget"
            );
            assert!(
                retained <= EvalLimits::DEFAULT_TOTAL_STEPS as usize,
                "{retained}"
            );
            let source = looped(200, 50);
            let (result, messages, took, retained) = run(&source, EvalLimits::DEFAULT);
            assert_eq!((result, messages), (0.0, vec![STEP_LIMIT.to_owned()]));
            assert!(took < crate::wall_clock(3_000), "{took:?}");
            assert!(
                retained <= EvalLimits::DEFAULT_TOTAL_STEPS as usize,
                "{retained}"
            );
        }

        /// The store that would add a 257th member stops the evaluation with 0 and writes nothing;
        /// an existing member of a struct at or past the budget may still be written.
        #[test]
        fn struct_width_at_the_limit() {
            assert_eq!(
                EvalLimits::DEFAULT.struct_members,
                Some(EvalLimits::DEFAULT_STRUCT_MEMBERS)
            );
            assert_eq!(EvalLimits::DEFAULT_STRUCT_MEMBERS, 256);
            assert_eq!(EvalLimits::NONE.struct_members, None);
            assert!(EvalLimits::NONE.is_unlimited());
            let fill = |members: usize| {
                (0..members).fold(String::new(), |mut s, i| {
                    let _ = write!(s, "v.s.m{i} = {i};");
                    s
                })
            };
            let width = |env: &NoHostEnv| {
                env.variables
                    .get(VariableName::new("s"))
                    .and_then(Value::as_struct)
                    .map(StructValue::len)
            };

            let mut env = NoHostEnv::new();
            assert_eq!(
                expr(&format!("{} v.s.m0 = 7; return 1;", fill(256))).eval_f32(&mut env.cx()),
                1.0
            );
            assert!(env.sink.is_empty());
            assert_eq!(width(&env), Some(256));
            assert_eq!(expr("v.s.m257 = 1; return 1;").eval_f32(&mut env.cx()), 0.0);
            assert_eq!(env.sink.take(), [WIDTH_LIMIT]);
            assert_eq!(width(&env), Some(256), "nothing written");
            // A nested struct at the budget, through a longer path; a temp too.
            let mut env = NoHostEnv::new();
            let nested = fill(256).replace("v.s.", "t.x.y.");
            assert_eq!(
                expr(&format!(
                    "{nested} t.x.z = 1; v.ok = 1; t.x.y.extra = 1; v.not = 1;"
                ))
                .eval_f32(&mut env.cx()),
                0.0
            );
            assert_eq!(env.sink.take(), [WIDTH_LIMIT]);
            assert_eq!(
                env.variables.get(VariableName::new("ok")),
                Some(&Value::Float(1.0))
            );
            assert_eq!(env.variables.get(VariableName::new("not")), None);

            // A struct the host stored, wider than the budget: its members can be written, not
            // added.
            let mut env = NoHostEnv::new();
            let names: Vec<String> = (0..300).map(|i| format!("m{i}")).collect();
            let wide: StructValue<NoHost> = names.iter().map(|name| (name.as_str(), 1.0)).collect();
            env.variables
                .set(VariableName::new("s"), Value::structure(wide));
            assert_eq!(
                expr("v.s.m299 = 5; return v.s.m299;").eval_f32(&mut env.cx()),
                5.0
            );
            assert!(env.sink.is_empty());
            assert_eq!(expr("v.s.m300 = 5; return 1;").eval_f32(&mut env.cx()), 0.0);
            assert_eq!(env.sink.take(), [WIDTH_LIMIT]);

            let mut env = NoHostEnv {
                limits: EvalLimits::NONE,
                ..NoHostEnv::new()
            };
            assert_eq!(
                expr(&format!("{} return 1;", fill(1000))).eval_f32(&mut env.cx()),
                1.0
            );
            assert_eq!(width(&env), Some(1000));
            // A budget of 0 refuses every new member, including the first.
            let mut env = NoHostEnv::new();
            env.limits.struct_members = Some(0);
            assert_eq!(expr("v.s.a = 1; return 1;").eval_f32(&mut env.cx()), 0.0);
            assert_eq!(
                env.sink.take(),
                [
                    "molangx: evaluation stopped: a struct would hold more than its budget of 0 members"
                ]
            );
        }

        /// The smallest step budget under which `expr` completes without a message.
        fn steps_needed(expr: &Expr, env: impl Fn() -> NoHostEnv) -> u64 {
            (0..10_000)
                .find(|&steps| {
                    let mut env = env();
                    env.limits.total_steps = Some(steps);
                    expr.eval(&mut env.cx());
                    env.sink.is_empty()
                })
                .expect("completes within 10,000 steps")
        }

        /// On top of their instructions, every struct on a member store's path costs its members
        /// plus 4, shared or not; a level the store creates costs 4; an actor array costs one step
        /// per entry, before null entries are dropped.
        #[test]
        fn size_proportional_costs() {
            let base = steps_needed(&expr("v.x = 1;"), NoHostEnv::new);
            // `v.s.a = 1` on an unset `v.s`: one created level.
            assert_eq!(steps_needed(&expr("v.s.a = 1;"), NoHostEnv::new), base + 4);
            // Three created levels.
            assert_eq!(
                steps_needed(&expr("v.s.a.b.c = 1;"), NoHostEnv::new),
                base + 12
            );
            // Over a stored 10-member struct whose member `a` is a 3-member struct.
            let stored = || {
                let mut env = NoHostEnv::new();
                let inner = StructValue::from([("x", 1.0), ("y", 1.0), ("z", 1.0)]);
                let names: Vec<String> = (0..9).map(|i| format!("m{i}")).collect();
                let members = names.iter().map(|name| (name.as_str(), Value::Float(1.0)));
                let outer: StructValue<NoHost> = std::iter::once(("a", Value::structure(inner)))
                    .chain(members)
                    .collect();
                env.variables
                    .set(VariableName::new("s"), Value::structure(outer));
                env
            };
            assert_eq!(
                steps_needed(&expr("v.s.a.x = 1;"), stored),
                base + (4 + 10) + (4 + 3)
            );
            assert_eq!(
                steps_needed(&expr("v.s.a.w = 1;"), stored),
                base + (4 + 10) + (4 + 3)
            );
            // An actor array of 1,000 entries (none of which resolves without a host).
            let with_array = || {
                let mut env = NoHostEnv::new();
                env.context
                    .set(ContextName::new("arr"), Value::actor_array(vec![(); 1000]));
                env
            };
            let with_empty_array = || {
                let mut env = NoHostEnv::new();
                env.context
                    .set(ContextName::new("arr"), Value::actor_array(Vec::new()));
                env
            };
            for source in ["v.x = c.arr;", "t.x = c.arr;", "v.s.x = c.arr;"] {
                let expr = expr(source);
                assert_eq!(
                    steps_needed(&expr, with_array),
                    steps_needed(&expr, with_empty_array) + 1000,
                    "{source}"
                );
            }
        }
    }

    /// 32 structs with 2^31 paths to the innermost one: `==` compares each pair of structs once
    /// (shared ones not at all) and `Debug` prints at most 4,096 members.
    #[test]
    fn shared_structs_compare_and_print_once() {
        use molangx::vm::{DistinctStructs, NoHostEnv, StructValue, Value, VariableName};
        let source = "v.a.z=1; loop(31,{v.b=v.a; v.a.x=v.b; v.a.y=v.b;}); return 1;";
        let expr = at(source, 13).expr().cloned().expect("compiles");
        let run = || {
            let mut env = NoHostEnv::new();
            assert_eq!(expr.eval_f32(&mut env.cx()), 1.0);
            assert!(env.sink.is_empty());
            env
        };
        let env = run();
        let started = std::time::Instant::now();
        let printed = format!("{:?}", env.variables);
        let printed_env = format!("{env:?}");
        assert!(
            printed.len() < 1_000_000 && printed.contains(".."),
            "{} bytes",
            printed.len()
        );
        assert!(printed_env.len() < 1_000_000, "{} bytes", printed_env.len());
        // The same map (shared structs), and one built separately (equal, nothing shared).
        assert!(env.variables == env.variables.clone());
        let other = run();
        assert!(env.variables == other.variables);
        let a = env.variables.get(VariableName::new("a")).expect("set");
        assert_eq!(a, other.variables.get(VariableName::new("a")).expect("set"));
        assert_eq!(
            a.as_struct(),
            other
                .variables
                .get(VariableName::new("a"))
                .and_then(Value::as_struct)
        );
        // One different leaf is found.
        let mut changed = a.clone();
        let path = vec![HashedStr::new("y"); 30];
        changed.set_member_path(
            &[path.as_slice(), &[HashedStr::new("x"), HashedStr::new("z")]].concat(),
            Value::Float(2.0),
        );
        assert_ne!(&changed, a);
        assert_eq!(a.distinct_structs().count(), 32);
        assert_eq!(
            a.distinct_structs().map(StructValue::len).sum::<usize>(),
            31 * 3 + 1
        );
        // `v.b` is `v.a` one round earlier: the whole map has the same 32 structs.
        assert_eq!(
            DistinctStructs::of(env.variables.iter().map(|(_, value)| value)).count(),
            32
        );
        let took = started.elapsed();
        assert!(took < crate::wall_clock(2_000), "{took:?}");
    }

    /// The default sink of `HostEnv` and `NoHostEnv` keeps the last 64 messages, each cut to 1 KiB,
    /// and counts the rest; a host's own sink receives every message whole, and `LogOnce` remembers
    /// at most `LogOnce::SEEN_CAP` texts.
    #[test]
    fn default_sink_is_bounded() {
        use molangx::vm::{
            BoundedSink, HostEnv, LogOnce, NoHost, NullSink, QueryError, RuntimeMsg, RuntimeSink,
            Subjects,
        };
        let name = "a".repeat(65_000);
        let expr = at(&format!("v.{name}"), 13)
            .expr()
            .cloned()
            .expect("compiles");
        let mut env: HostEnv<NoHost> = HostEnv::default();
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            assert_eq!(
                expr.eval_f32(&mut env.cx(&mut NoHost, Subjects::none())),
                0.0
            );
        }
        assert!(
            started.elapsed() < crate::wall_clock(10_000),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(env.sink.messages().len(), BoundedSink::DEFAULT_CAPACITY);
        assert_eq!(env.sink.dropped(), 10_000 - 64);
        let kept: usize = env.sink.messages().iter().map(String::len).sum();
        assert!(
            kept <= 64 * (BoundedSink::MAX_MESSAGE_BYTES + '…'.len_utf8()),
            "{kept} bytes kept"
        );
        for message in env.sink.messages() {
            assert!(
                message.starts_with("Error: unhandled request for unknown variable 'variable.aaaa"),
                "{message}"
            );
            assert!(
                message.ends_with('…') && message.len() <= BoundedSink::MAX_MESSAGE_BYTES + 3,
                "{}",
                message.len()
            );
        }
        let taken = env.sink.take();
        assert_eq!(
            (taken.len(), env.sink.dropped(), env.sink.is_empty()),
            (64, 0, true)
        );

        // Capacity 0 keeps nothing.
        let mut none = BoundedSink::with_capacity(0);
        none.runtime(RuntimeMsg::PublicAccessUnderflow);
        assert_eq!((none.is_empty(), none.dropped()), (true, 1));

        struct Lengths(Vec<usize>);
        impl RuntimeSink for Lengths {
            fn runtime(&mut self, msg: RuntimeMsg<'_>) {
                if let RuntimeMsg::UnknownVariable { name, .. } = msg {
                    self.0.push(name.len());
                }
            }
            fn query_error(&mut self, _error: QueryError) {}
        }
        let mut env = HostEnv::<NoHost>::default().with_sink(Lengths(Vec::new()));
        expr.eval_f32(&mut env.cx(&mut NoHost, Subjects::none()));
        assert_eq!(env.sink.0, [65_009], "the host's sink gets the whole name");

        // `LogOnce` keeps at most `SEEN_CAP` hashes: past it a new session starts.
        let mut once = LogOnce::new(NullSink);
        for i in 0..10_000 {
            once.runtime(RuntimeMsg::UnknownVariable {
                name: &format!("variable.n{i}"),
                public_access: false,
            });
            assert!(once.distinct() <= LogOnce::<NullSink>::SEEN_CAP);
        }
        assert_eq!(once.distinct(), 10_000 % LogOnce::<NullSink>::SEEN_CAP);
        let mut once = LogOnce::new(BoundedSink::new());
        for _ in 0..3 {
            once.runtime(RuntimeMsg::UnknownVariable {
                name: "variable.x",
                public_access: false,
            });
        }
        assert_eq!(
            once.into_inner().take().len(),
            1,
            "a repeated text is still logged once"
        );
    }

    /// A `continue` / `break` inside an operand in an inner loop runs the outer loop away: the
    /// default loop budget leaves it after 1,024 passes, and under `EvalLimits::NONE` the
    /// operand-stack cap ends the evaluation.
    #[test]
    fn runaway_outer_loop_is_bounded() {
        use molangx::vm::{EvalLimits, NoHostEnv, Value, VariableName};
        for jump in ["continue", "break"] {
            let source = format!(
                "v.zero = 0; v.i = 0; v.j = 0; loop(2, {{ v.i = v.i + 1; loop(3, {{ v.j = v.j + 1; v.t = v.zero * (v.j > 0 ? {{{jump};}} : 0); }}); }}); return 1;"
            );
            let expr = at(&source, 13).expr().cloned().expect("compiles");
            for limits in [EvalLimits::DEFAULT, EvalLimits::NONE] {
                let expr = expr.clone();
                let (result, messages, passes) = std::thread::Builder::new()
                    .stack_size(512 * 1024)
                    .spawn(move || {
                        let mut env = NoHostEnv {
                            limits,
                            ..NoHostEnv::new()
                        };
                        let started = std::time::Instant::now();
                        let result = expr.eval_f32(&mut env.cx());
                        assert!(
                            started.elapsed() < crate::wall_clock(30_000),
                            "took {:?}",
                            started.elapsed()
                        );
                        let passes = env.variables.get(VariableName::new("i")).map(Value::as_f32);
                        (result, env.sink.take(), passes)
                    })
                    .expect("thread")
                    .join()
                    .expect("no panic");
                if limits == EvalLimits::DEFAULT {
                    // The outer loop is left after its budget; the expression runs on and
                    // returns 1.
                    assert_eq!(result, 1.0, "{jump}");
                    assert_eq!(
                        messages,
                        ["molangx: loop stopped after its budget of 1024 iterations"],
                        "{jump}"
                    );
                    assert_eq!(passes, Some(1024.0), "{jump}");
                } else {
                    assert_eq!(result, 0.0, "{jump}");
                    assert_eq!(
                        messages,
                        [
                            "molangx: evaluation stopped: operands left behind by break / continue passed the cap of 65536"
                        ],
                        "{jump}"
                    );
                    assert!(passes.is_some_and(|p| p > 60_000.0), "{jump}: {passes:?}");
                }
            }
        }
    }

    /// The stack size `Expr::eval` documents for an evaluation thread.
    const DOCUMENTED_EVAL_STACK: usize = if cfg!(debug_assertions) {
        4 << 20
    } else {
        512 << 10
    };

    /// Query arguments are the evaluator's only native recursion, and the parser bounds it at 255
    /// nested calls; the worst case runs on the stack `Expr::eval` documents.
    #[test]
    fn nested_query_arguments_at_the_limit() {
        use crate::common::host::Env;
        use molangx::rng::Xorshift128;
        use molangx::vm::CollectSink;
        let nested = |depth: usize| format!("{}1{}", "query.log(".repeat(depth), ")".repeat(depth));
        assert_eq!(
            at(&nested(256), 13).failure(),
            Some(CompileFailure::Rejected)
        );
        for source in [nested(255), format!("v.y = {}; return v.y;", nested(253))] {
            let value = std::thread::Builder::new()
                .stack_size(DOCUMENTED_EVAL_STACK)
                .spawn(move || {
                    let expr = at(&source, 13).expr().cloned().expect("compiles");
                    let mut env = Env::new();
                    let mut rng = Xorshift128::new();
                    let mut sink = CollectSink::new();
                    env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx))
                })
                .expect("thread")
                .join()
                .expect("no overflow");
            assert_eq!(value, 1.0);
        }
    }

    /// A 128 KiB thread evaluates the deepest nesting under a budget of 8 levels: the ninth
    /// argument ends the evaluation with 0 and one message.
    #[test]
    fn query_depth_budget() {
        use crate::common::host::Env;
        use molangx::rng::Xorshift128;
        use molangx::vm::{CollectSink, EvalLimits};
        assert_eq!(EvalLimits::DEFAULT.query_depth, None);
        assert_eq!(EvalLimits::NONE.query_depth, None);
        let nested = |depth: usize| format!("{}1{}", "query.log(".repeat(depth), ")".repeat(depth));
        let run = |depth: usize, budget: u32| {
            let expr = at(&nested(depth), 13).expr().cloned().expect("compiles");
            std::thread::Builder::new()
                .stack_size(128 << 10)
                .spawn(move || {
                    let mut env = Env::new();
                    env.limits.query_depth = Some(budget);
                    let mut rng = Xorshift128::new();
                    let mut sink = CollectSink::new();
                    let value = env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx));
                    (value, sink.take())
                })
                .expect("thread")
                .join()
                .expect("no overflow")
        };
        assert_eq!(
            run(255, 8),
            (0.0, vec!["molangx: evaluation stopped: query arguments would nest deeper than their budget of 8 levels".to_owned()])
        );
        // Eight nested calls evaluate arguments at levels 1 … 8.
        assert_eq!(run(8, 8), (1.0, Vec::new()));
        assert_eq!(run(9, 8).0, 0.0);
        // A budget of 0 lets a query be called, not evaluate an argument.
        assert_eq!(
            run(1, 0).1,
            [
                "molangx: evaluation stopped: query arguments would nest deeper than their budget of 0 levels"
            ]
        );
    }

    mod loop_and_step_budgets {
        use crate::common::compile_support::server_expr;
        use crate::common::host::Env;

        use molangx::rng::Xorshift128;
        use molangx::vm::{CollectSink, EvalLimits, NoHostEnv, Value, VariableName};

        fn eval_in(env: &mut NoHostEnv, source: &str) -> f32 {
            server_expr(source).eval_f32(&mut env.cx())
        }

        /// The per-loop guard logs once per evaluation, however many loops it leaves.
        #[test]
        fn loop_guard() {
            let expr = server_expr("t.i = 0; loop(5000, {t.i = t.i + 1;}); return t.i;");
            let mut env = NoHostEnv::new();
            assert_eq!(expr.eval_f32(&mut env.cx()), 1024.0);
            assert_eq!(
                env.sink.take(),
                vec!["molangx: loop stopped after its budget of 1024 iterations".to_owned()]
            );

            env.limits = EvalLimits::NONE;
            assert_eq!(expr.eval_f32(&mut env.cx()), 5000.0);
            assert!(env.sink.is_empty());

            // The budget is per loop: each inner loop gets its own.
            env.limits = EvalLimits {
                loop_iterations: Some(10),
                total_steps: None,
                ..EvalLimits::NONE
            };
            assert_eq!(
                eval_in(
                    &mut env,
                    "t.n = 0; loop(20, {loop(20, {t.n = t.n + 1;});}); return t.n;"
                ),
                100.0
            );
            // Eleven loops were left (ten inner, one outer): one message.
            assert_eq!(
                env.sink.take(),
                vec!["molangx: loop stopped after its budget of 10 iterations".to_owned()]
            );
            // Each evaluation reports for itself.
            assert_eq!(
                eval_in(&mut env, "t.n = 0; loop(20, {t.n = t.n + 1;}); return t.n;"),
                10.0
            );
            assert_eq!(env.sink.take().len(), 1);

            // 1,024 inner loops left: one loop message, then the step budget ends the evaluation.
            env.limits = EvalLimits::DEFAULT;
            assert_eq!(
                eval_in(
                    &mut env,
                    "t.n = 0; loop(2000, {loop(2000, {t.n = t.n + 1;});}); return 1;"
                ),
                0.0
            );
            let messages = env.sink.take();
            assert_eq!(
                messages,
                vec![
                    "molangx: loop stopped after its budget of 1024 iterations".to_owned(),
                    "molangx: evaluation stopped after its budget of 1048576 steps".to_owned()
                ]
            );
        }

        /// Also when it is spent inside a query argument and the query goes on to evaluate its
        /// other arguments.
        #[test]
        fn step_budget_logs_once() {
            let mut env = Env::new();
            env.limits = EvalLimits {
                total_steps: Some(500),
                ..EvalLimits::NONE
            };
            let expr = server_expr(
                "query.log(math.die_roll(1000, 1, 2), math.die_roll(1000, 1, 2), 3) + math.die_roll(1000, 1, 2)",
            );
            let mut rng = Xorshift128::new();
            let mut sink = CollectSink::new();
            assert_eq!(
                env.with_cx(&mut rng, &mut sink, |cx| expr.eval_f32(cx)),
                0.0
            );
            assert_eq!(
                sink.take(),
                vec!["molangx: evaluation stopped after its budget of 500 steps".to_owned()]
            );
        }

        /// At the float-only loop's hand-over to the general loop. `v.s` (a string) is a load and
        /// an end: two steps.
        #[test]
        fn a_handed_over_instruction_costs_one_step() {
            let expr = server_expr("v.s");
            assert!(
                expr.flags()
                    .contains(molangx::compile::ProgramFlags::FLOAT_ONLY)
            );
            let run = |steps: u64, as_f32: bool| {
                let mut env = NoHostEnv::new();
                env.variables
                    .set(VariableName::new("s"), Value::string("moo"));
                env.limits = EvalLimits {
                    total_steps: Some(steps),
                    ..EvalLimits::NONE
                };
                let value = if as_f32 {
                    Value::Float(expr.eval_f32(&mut env.cx()))
                } else {
                    expr.eval(&mut env.cx())
                };
                (value, env.sink.take())
            };
            assert_eq!(run(2, false), (Value::string("moo"), vec![]));
            assert_eq!(
                run(2, true),
                (
                    Value::Float(Value::<molangx::vm::NoHost>::string("moo").as_f32()),
                    vec![]
                )
            );
            assert_eq!(
                run(1, false),
                (
                    Value::ZERO,
                    vec!["molangx: evaluation stopped after its budget of 1 steps".to_owned()]
                )
            );

            // The store after the hand-over runs within the program's exact budget of four steps.
            let store = server_expr("v.a = v.s;");
            let mut env = NoHostEnv::new();
            env.variables
                .set(VariableName::new("s"), Value::string("moo"));
            env.limits = EvalLimits {
                total_steps: Some(4),
                ..EvalLimits::NONE
            };
            store.eval(&mut env.cx());
            assert!(env.sink.is_empty(), "{:?}", env.sink.messages());
            assert_eq!(
                env.variables.get(VariableName::new("a")),
                Some(&Value::string("moo"))
            );
        }

        /// The step budget ends with 0 an evaluation `EvalLimits::NONE` does not end; die rolls are
        /// charged to it one by one.
        #[test]
        fn step_budget() {
            let mut env = NoHostEnv {
                limits: EvalLimits {
                    loop_iterations: None,
                    total_steps: Some(10_000),
                    ..EvalLimits::NONE
                },
                ..NoHostEnv::new()
            };
            // An f32 counter of 1e30 never reaches 0.
            assert_eq!(
                eval_in(
                    &mut env,
                    "t.i = 0; loop(1e30, {t.i = t.i + 1;}); return t.i;"
                ),
                0.0
            );
            assert_eq!(
                env.sink.take(),
                vec!["molangx: evaluation stopped after its budget of 10000 steps".to_owned()]
            );
            assert_eq!(eval_in(&mut env, "math.die_roll(1000000, 1, 6)"), 0.0);
            assert_eq!(env.sink.take().len(), 1);
            // Within the budget nothing is reported.
            assert_eq!(eval_in(&mut env, "math.die_roll(3, 2, 2)"), 6.0);
            assert!(env.sink.is_empty());
        }
    }
}

#[cfg(feature = "vm")]
mod eval_limit_fields {
    //! Each `EvalLimits` field: the program that crosses it, the value it ends with and the one
    //! message it logs.

    use crate::common::compile_support::server_expr;
    use molangx::catalog::Side;

    use molangx::stdlib::query;
    use molangx::vm::{EvalLimits, NoHost, NoHostEnv, QueryCx, QueryError, QueryTable, Value};

    fn run(limits: EvalLimits, source: &str) -> (f32, Vec<String>) {
        let mut queries = QueryTable::new(molangx::stdlib::queries(Side::Server));
        queries.set(query::IS_BABY, first_argument).unwrap();
        let mut env = NoHostEnv {
            queries: Some(queries),
            limits,
            ..NoHostEnv::new()
        };
        let value = server_expr(source).eval_f32(&mut env.cx());
        (value, env.sink.take())
    }

    /// Evaluates its first argument, one level deeper.
    fn first_argument(cx: &mut QueryCx<'_, '_, NoHost>) -> Result<Value<NoHost>, QueryError> {
        Ok(cx.arg(0).unwrap_or_default())
    }

    #[test]
    fn loop_iterations_leave_the_loop_and_say_so_once() {
        let limits = EvalLimits {
            loop_iterations: Some(3),
            ..EvalLimits::NONE
        };
        let source = "t.n = 0; loop(10, {t.n = t.n + 1;}); return t.n;";
        assert_eq!(
            run(limits, source),
            (
                3.0,
                vec!["molangx: loop stopped after its budget of 3 iterations".to_owned()]
            )
        );
        assert_eq!(
            run(limits, "t.n = 0; loop(3, {t.n = t.n + 1;}); return t.n;"),
            (3.0, vec![])
        );
    }

    #[test]
    fn total_steps_end_the_evaluation_with_run() {
        let limits = EvalLimits {
            total_steps: Some(40),
            ..EvalLimits::NONE
        };
        assert_eq!(
            run(limits, "t.n = 0; loop(100, {t.n = t.n + 1;}); return t.n;"),
            (
                0.0,
                vec!["molangx: evaluation stopped after its budget of 40 steps".to_owned()]
            )
        );
        assert_eq!(run(limits, "1 + v.a * 2 ?? 3"), (3.0, vec![]));
    }

    #[test]
    fn struct_depth_refuses_a_store_that_nests_deeper() {
        let limits = EvalLimits {
            struct_depth: Some(1),
            ..EvalLimits::NONE
        };
        assert_eq!(run(limits, "v.a.b = 1; return 5;"), (5.0, vec![]));
        assert_eq!(
            run(limits, "v.a.b.c = 1; return 5;"),
            (0.0, vec!["molangx: evaluation stopped: a struct would nest deeper than its budget of 1 levels".to_owned()])
        );
    }

    #[test]
    fn struct_members_refuse_a_store_that_widens_a_struct() {
        let limits = EvalLimits {
            struct_members: Some(2),
            ..EvalLimits::NONE
        };
        assert_eq!(
            run(
                limits,
                "v.s.a = 1; v.s.b = 2; v.s.a = 3; return v.s.a + v.s.b;"
            ),
            (5.0, vec![])
        );
        assert_eq!(
            run(limits, "v.s.a = 1; v.s.b = 2; v.s.c = 3; return 9;"),
            (0.0, vec!["molangx: evaluation stopped: a struct would hold more than its budget of 2 members".to_owned()])
        );
    }

    #[test]
    fn query_depth_refuses_an_argument_that_runs_too_deep() {
        let limits = EvalLimits {
            query_depth: Some(1),
            ..EvalLimits::NONE
        };
        assert_eq!(run(limits, "q.is_baby(7)"), (7.0, vec![]));
        assert_eq!(
            run(limits, "q.is_baby(q.is_baby(7))"),
            (0.0, vec!["molangx: evaluation stopped: query arguments would nest deeper than their budget of 1 levels".to_owned()])
        );
        // Zero lets no query evaluate an argument.
        let none = EvalLimits {
            query_depth: Some(0),
            ..EvalLimits::NONE
        };
        assert_eq!(
            run(none, "q.is_baby(7)").1,
            [
                "molangx: evaluation stopped: query arguments would nest deeper than their budget of 0 levels"
            ]
        );
    }

    /// What `Some(0)` means for each field, as the field docs say.
    #[test]
    fn a_zero_budget() {
        let loops = EvalLimits {
            loop_iterations: Some(0),
            ..EvalLimits::NONE
        };
        assert_eq!(
            run(loops, "t.n = 0; loop(2, {t.n = t.n + 1;}); return t.n + 5;"),
            (
                6.0,
                vec!["molangx: loop stopped after its budget of 0 iterations".to_owned()]
            ),
            "a loop runs its first iteration"
        );
        let steps = EvalLimits {
            total_steps: Some(0),
            ..EvalLimits::NONE
        };
        assert_eq!(
            run(steps, "1 + 2"),
            (3.0, vec![]),
            "a folded constant runs no instruction"
        );
        assert_eq!(
            run(steps, "v.a ?? 4").1,
            ["molangx: evaluation stopped after its budget of 0 steps"]
        );
        let depth = EvalLimits {
            struct_depth: Some(0),
            ..EvalLimits::NONE
        };
        assert_eq!(
            run(depth, "v.a.b = 1; return 5;"),
            (0.0, vec!["molangx: evaluation stopped: a struct would nest deeper than its budget of 0 levels".to_owned()])
        );
        assert_eq!(run(depth, "v.a = 1; return 5;"), (5.0, vec![]));
        let members = EvalLimits {
            struct_members: Some(0),
            ..EvalLimits::NONE
        };
        assert_eq!(
            run(members, "v.s.a = 1; return 9;"),
            (0.0, vec!["molangx: evaluation stopped: a struct would hold more than its budget of 0 members".to_owned()])
        );
    }

    #[test]
    fn the_defaults_bound_what_evallimits_none_leaves_open() {
        let default = EvalLimits::DEFAULT;
        assert_eq!(
            (
                default.loop_iterations,
                default.total_steps,
                default.struct_depth,
                default.struct_members
            ),
            (Some(1024), Some(1_048_576), Some(32), Some(256))
        );
        let unlimited = EvalLimits::NONE;
        assert_eq!(
            (
                unlimited.loop_iterations,
                unlimited.total_steps,
                unlimited.struct_depth,
                unlimited.struct_members,
                unlimited.query_depth
            ),
            (None, None, None, None, None)
        );
        assert_eq!(
            run(
                default,
                "t.n = 0; loop(5000, {t.n = t.n + 1;}); return t.n;"
            )
            .0,
            1024.0
        );
        assert_eq!(
            run(
                unlimited,
                "t.n = 0; loop(5000, {t.n = t.n + 1;}); return t.n;"
            ),
            (5000.0, vec![])
        );
    }
}
