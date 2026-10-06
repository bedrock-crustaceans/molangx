//! The differential check: one generated case on the VM and on the tree walker from the same state.

use super::FuzzEnv;
use super::compare::{same_value, state_difference};
use crate::generator::FuzzCase;
use crate::tree_walker;
use molangx::compile::compile;
use molangx::internals::reference_catalog;

/// What [`differential`] concluded about one case (when the VM and the tree walker do not simply
/// disagree, which is an `Err`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The text did not compile to an expression.
    NotCompiled,
    /// The tree walker does not model the expression ([`tree_walker::models`]): the VM alone ran,
    /// its own checks passed.
    VmOnly,
    /// The VM and the tree walker agree.
    Agree,
}

/// Runs one [`FuzzCase`] on the VM and on the tree walker from the same
/// starting state: the [`Verdict`], or why they disagree.
///
/// Compiled for the client with the `default` query set, the helper queries' set and the helper
/// experiment. On the VM, `eval_f32` must be the float of `eval` with the same state afterwards.
/// The comparison is exact: the value (NaN sign and payload included), every variable map (each
/// variable's value, access and public snapshot), the temps, the messages and the random source.
pub fn differential(case: &FuzzCase) -> Result<Verdict, String> {
    let source = case.source();
    let compiled = compile(&source, &reference_catalog::options(case.version));
    // A rejected text is compared too: both sides evaluate the constant 0.
    let Some(expr) = compiled.expr_or_zero() else {
        return Ok(Verdict::NotCompiled);
    };
    let limits = case.limits();
    let start = FuzzEnv::new(case.subject, case.temps, limits, case.rng());

    let mut vm = start.clone();
    let value = expr.eval(&mut vm.cx());
    let mut again = start.clone();
    let float = expr.eval_f32(&mut again.cx());
    #[cfg(test)]
    let (value, again) = hooks::tamper(&hooks::TAMPER_EARLY, value, again);
    if float.to_bits() != value.as_f32().to_bits() {
        return Err(format!(
            "{source:?}: eval_f32 {float} is not the float of eval {value:?}"
        ));
    }
    if let Some(why) = state_difference(&vm, &again) {
        return Err(format!(
            "{source:?}: eval and eval_f32 leave different states: {why}"
        ));
    }
    if !tree_walker::models(expr) {
        return Ok(Verdict::VmOnly);
    }
    #[cfg(test)]
    let (value, vm) = hooks::tamper(&hooks::TAMPER, value, vm);
    let mut walker = start;
    let expected = tree_walker::eval(expr, &mut walker.cx());
    let why = if same_value(&value, &expected) {
        match state_difference(&vm, &walker) {
            Some(why) => format!("value {value:?}; vm vs walker: {why}"),
            None => return Ok(Verdict::Agree),
        }
    } else {
        format!("vm {value:?}, walker {expected:?}")
    };
    Err(format!("{source:?} (v{}, {limits:?}): {why}", case.version))
}

#[cfg(test)]
mod hooks {
    //! The seam through which a test of [`differential`](super::differential) alters the VM's
    //! result: a closure planted in a thread-local slot runs on the value and the environment just
    //! before the comparison it is meant to disturb.

    use crate::generator::env::{FuzzEnv, test_support::V};
    use std::cell::RefCell;
    use std::thread::LocalKey;

    pub(super) type Tamper = Box<dyn FnMut(&mut V, &mut FuzzEnv)>;

    thread_local! {
        /// Runs on the VM's value and environment after its own checks, before the tree walker's
        /// comparison.
        pub(super) static TAMPER: RefCell<Option<Tamper>> = const { RefCell::new(None) };
        /// Runs on the value of `eval` and on the environment of `eval_f32`, before the two are
        /// compared.
        pub(super) static TAMPER_EARLY: RefCell<Option<Tamper>> = const { RefCell::new(None) };
    }

    pub(super) fn tamper(
        slot: &'static LocalKey<RefCell<Option<Tamper>>>,
        mut value: V,
        mut env: FuzzEnv,
    ) -> (V, FuzzEnv) {
        slot.with(|hook| {
            if let Some(f) = hook.borrow_mut().as_mut() {
                f(&mut value, &mut env);
            }
        });
        (value, env)
    }

    /// Removes the planted closures when dropped, so a test run on a shared thread leaves none
    /// behind.
    pub(super) struct Planted;

    impl Drop for Planted {
        fn drop(&mut self) {
            TAMPER.with(|hook| *hook.borrow_mut() = None);
            TAMPER_EARLY.with(|hook| *hook.borrow_mut() = None);
        }
    }

    pub(super) fn plant(late: Option<Tamper>, early: Option<Tamper>) -> Planted {
        TAMPER.with(|hook| *hook.borrow_mut() = late);
        TAMPER_EARLY.with(|hook| *hook.borrow_mut() = early);
        Planted
    }
}

#[cfg(test)]
mod tests {
    use super::hooks::*;
    use super::*;
    use crate::generator::test_support::{b, buffer, case_of, n, var};
    use crate::generator::{
        BinOp, Ex, Ns, Program, Stmt, Style, Var,
        env::{FuzzActor, FuzzRng, Subject, TempLifetime, test_support::*},
    };
    use molangx::catalog::Side;
    use molangx::compile::{CompileOptions, compile};
    use molangx::numeric::{ARCH, Arch};
    use molangx::rng::Xorshift128;
    use molangx::rng::{FixedRng, sample};
    use molangx::version::RawVersion;
    use molangx::vm::{Access, EvalLimits, TempName, Value, VariableName};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn entity(name: u8) -> Ex {
        Ex::Var(Var::plain(Ns::Entity, name))
    }

    /// A case with the defaults of a plain run.
    fn case(program: Program) -> FuzzCase {
        FuzzCase {
            program,
            style: Style::default(),
            version: 13,
            temps: TempLifetime::PerEvaluation,
            steps: 5000,
            loop_iterations: 64,
            struct_members: EvalLimits::DEFAULT_STRUCT_MEMBERS,
            query_depth: None,
            forced_random: None,
            subject: Subject::Detached,
        }
    }

    fn simple(ex: Ex) -> FuzzCase {
        case(Program::Simple(ex))
    }

    /// `v.x`: the starting value, 1.5.
    fn x() -> Ex {
        entity(0)
    }

    #[test]
    fn ordinary_cases_agree() {
        let programs = [
            simple(n(1)),
            simple(Ex::Bin(BinOp::Add, b(x()), b(n(2)))),
            simple(Ex::Bin(BinOp::Div, b(n(1)), b(entity(1)))),
            simple(Ex::Math(18, vec![x(), n(2)])),
            simple(Ex::Query(0, vec![n(1), n(2)])),
            simple(Ex::Cond(b(x()), b(n(1)), Some(b(n(2))))),
            simple(Ex::Arrow(b(Ex::Var(Var::plain(Ns::Context, 0))), b(x()))),
            case(Program::Complex(vec![
                Stmt::Expr(Ex::Assign(
                    Var::plain(Ns::Entity, 7),
                    b(Ex::Bin(BinOp::Add, b(x()), b(n(1)))),
                )),
                Stmt::Return(entity(7)),
            ])),
            case(Program::Complex(vec![
                Stmt::Expr(Ex::Loop(
                    b(n(3)),
                    vec![
                        Stmt::Expr(Ex::Assign(Var::plain(Ns::Temp, 0), b(n(1)))),
                        Stmt::Break,
                    ],
                )),
                Stmt::Return(Ex::Var(Var::plain(Ns::Temp, 0))),
            ])),
        ];
        for program in programs {
            for (temps, subject, forced_random) in [
                (TempLifetime::PerEvaluation, Subject::Detached, None),
                (
                    TempLifetime::Persistent,
                    Subject::Actor,
                    Some(FixedRng::HALF),
                ),
                (
                    TempLifetime::Persistent,
                    Subject::Actor,
                    Some(FixedRng(u32::MAX)),
                ),
                (
                    TempLifetime::Persistent,
                    Subject::Actor,
                    Some(FixedRng(0x8000_0000)),
                ),
            ] {
                let case = FuzzCase {
                    temps,
                    forced_random,
                    subject,
                    ..program.clone()
                };
                assert_eq!(
                    differential(&case),
                    Ok(Verdict::Agree),
                    "{:?} printed as {:?}",
                    case.program,
                    case.source()
                );
            }
        }
        // A `for_each` is not modelled by the tree walker: the VM alone runs it.
        let each = case(Program::Complex(vec![
            Stmt::Expr(Ex::ForEach(
                Var::plain(Ns::Temp, 3),
                b(Ex::Var(Var::plain(Ns::Context, 2))),
                vec![Stmt::Continue],
            )),
            Stmt::Return(n(2)),
        ]));
        assert_eq!(
            differential(&each),
            Ok(Verdict::VmOnly),
            "{:?}",
            each.source()
        );
    }

    #[test]
    fn a_rejected_text_runs_as_the_constant_zero_on_both_evaluators() {
        // `Compiled::expr` is the constant 0 for a rejected source, and only a source that needs
        // arrays or resource variables resolved has none. The generator writes neither, so no case
        // of theirs is `NotCompiled`; a rejected text is compared as the constant 0.
        let rejected = [
            // `math.max` with one argument
            simple(Ex::Math(18, vec![n(1)])),
            // a statement list that goes on after a `return`
            case(Program::Complex(vec![Stmt::Return(n(1)), Stmt::Break])),
        ];
        for case in rejected {
            let (verdict, value, env) = observe(&case);
            assert_eq!(verdict, Ok(Verdict::Agree), "{:?}", case.source());
            assert_eq!(value, Value::ZERO, "{:?}", case.source());
            assert!(env.sink.messages.is_empty());
        }
    }

    #[test]
    fn a_verdict_is_a_small_copyable_value() {
        let verdict = Verdict::Agree;
        let copy = verdict;
        assert_eq!(verdict, copy);
        assert_ne!(Verdict::Agree, Verdict::VmOnly);
        assert_ne!(Verdict::VmOnly, Verdict::NotCompiled);
        assert_eq!(format!("{:?}", Verdict::NotCompiled), "NotCompiled");
    }

    /// `loop(2, { v.a = (v.b ?? { break; }) ?? 2; }); return v.never;`
    fn stale_handler() -> FuzzCase {
        let inner = Ex::Bin(
            BinOp::Coalesce,
            b(entity(8)),
            b(Ex::Block(vec![Stmt::Break])),
        );
        let outer = Ex::Bin(BinOp::Coalesce, b(inner), b(n(2)));
        case(Program::Complex(vec![
            Stmt::Expr(Ex::Loop(
                b(n(2)),
                vec![Stmt::Expr(Ex::Assign(Var::plain(Ns::Entity, 7), b(outer)))],
            )),
            Stmt::Return(entity(10)),
        ]))
    }

    #[test]
    fn a_break_out_of_a_coalescing_left_side_inside_a_loop_is_vm_only() {
        let case = stale_handler();
        assert!(case.source().to_ascii_lowercase().contains("break"));
        assert_eq!(differential(&case), Ok(Verdict::VmOnly));
        // With the tree walker's reading of it the case would differ; here only the VM ran.
        let compiled = compile(
            &case.source(),
            &CompileOptions::from_raw_version(
                molangx::stdlib::queries(Side::Client).clone(),
                RawVersion(13),
            ),
        );
        assert!(!crate::tree_walker::models(
            &compiled.expr().cloned().expect("it compiles")
        ));
        // Take away the loop and the tree walker models it: the verdict is a comparison again.
        let no_loop = FuzzCase {
            program: Program::Complex(vec![
                Stmt::Expr(Ex::Assign(
                    Var::plain(Ns::Entity, 7),
                    b(Ex::Bin(
                        BinOp::Coalesce,
                        b(Ex::Bin(
                            BinOp::Coalesce,
                            b(entity(8)),
                            b(Ex::Block(vec![Stmt::Break])),
                        )),
                        b(n(2)),
                    )),
                )),
                Stmt::Return(entity(10)),
            ]),
            ..case
        };
        assert_eq!(differential(&no_loop), Ok(Verdict::Agree));
    }

    #[test]
    fn a_vm_only_case_still_gets_the_vms_own_checks() {
        // The VM's eval and eval_f32 are compared before the tree walker is asked, so a planted
        // difference there is found even for an expression the tree walker declines.
        let case = stale_handler();
        assert_eq!(differential(&case), Ok(Verdict::VmOnly));
        let planted = plant(
            None,
            Some(Box::new(|_, again| {
                again.rng = FuzzRng::Fixed(FixedRng::ZERO);
            })),
        );
        let why = differential(&case)
            .expect_err("the early difference is found before the tree walker is asked");
        assert!(
            why.contains("eval and eval_f32 leave different states"),
            "{why}"
        );
        drop(planted);
        // A late difference is never seen: the tree walker is not asked.
        let planted = plant(Some(Box::new(|value, _| *value = Value::Float(99.0))), None);
        assert_eq!(differential(&case), Ok(Verdict::VmOnly));
        drop(planted);
    }

    /// Plants a closure on the VM's result and runs `case`.
    fn with_late(
        case: &FuzzCase,
        f: impl FnMut(&mut V, &mut FuzzEnv) + 'static,
    ) -> Result<Verdict, String> {
        let _planted = plant(Some(Box::new(f)), None);
        differential(case)
    }

    #[test]
    fn nothing_planted_means_agreement() {
        let case = simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))));
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
    }

    #[test]
    fn a_planted_value_difference_is_reported_with_both_values() {
        let case = simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))));
        let why =
            with_late(&case, |value, _| *value = Value::Float(99.0)).expect_err("a disagreement");
        assert!(why.contains("vm Float(99.0), walker Float(3.5)"), "{why}");
        assert!(
            why.contains("V.X") || why.to_ascii_lowercase().contains("v.x"),
            "the source is named: {why}"
        );
    }

    #[test]
    fn a_planted_difference_of_kind_is_reported() {
        let case = simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))));
        assert!(with_late(&case, |value, _| *value = Value::string("moo")).is_err());
        assert!(
            with_late(&case, |value, _| *value =
                Value::Actor(FuzzActor::Handle(1)))
            .is_err()
        );
    }

    #[test]
    fn a_value_that_differs_in_the_sign_of_zero_is_a_disagreement() {
        let case = simple(Ex::Bin(BinOp::Sub, b(n(1)), b(n(1))));
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
        // 1 - 1 is +0: planting -0 must be seen.
        assert!(with_late(&case, |value, _| *value = Value::Float(-0.0)).is_err());
    }

    #[test]
    fn a_nan_with_another_payload_is_a_disagreement() {
        // 0 / 0 guards to 0; build a NaN with `math.sqrt(-1)`.
        let case = simple(Ex::Math(29, vec![Ex::Neg(b(n(1)))]));
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
        assert!(
            with_late(&case, |value, _| *value =
                Value::Float(f32::from_bits(0x7fc0_4321)))
            .is_err()
        );
        assert!(
            with_late(&case, |value, _| *value = Value::Float(0.0)).is_err(),
            "a number where the tree walker has a NaN"
        );
    }

    #[test]
    fn a_planted_variable_difference_names_the_variable_map() {
        let case = case(Program::Complex(vec![
            Stmt::Expr(Ex::Assign(Var::plain(Ns::Entity, 7), b(n(1)))),
            Stmt::Return(n(2)),
        ]));
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
        let why = with_late(&case, |_, vm| {
            vm.vars.local.set(VariableName::new("a"), Value::Float(5.0));
        })
        .expect_err("a disagreement");
        assert!(why.contains("vm vs walker: variable map 0: "), "{why}");
        let subject = FuzzCase {
            subject: Subject::Actor,
            ..case.clone()
        };
        let why = with_late(&subject, |_, vm| {
            vm.vars.actors[1].set(VariableName::new("a"), Value::Float(5.0));
        })
        .expect_err("a disagreement");
        assert!(why.contains("variable map 2: "), "{why}");
        // A variable only the tree walker has written.
        let why = with_late(&subject, |_, vm| {
            vm.vars.actors[1].remove(VariableName::new("a"));
        })
        .expect_err("a disagreement");
        assert!(why.contains("variable map 2: "), "{why}");
        // A change to a variable the program never touched.
        assert!(
            with_late(&case, |_, vm| {
                vm.vars.local.set(VariableName::new("x"), Value::Float(7.0));
            })
            .is_err()
        );
    }

    #[test]
    fn a_planted_difference_in_the_access_of_a_variable_is_reported() {
        // The value of every variable is the same; one side marks a variable public.
        let case = case(Program::Complex(vec![
            Stmt::Expr(Ex::Assign(Var::plain(Ns::Entity, 7), b(n(1)))),
            Stmt::Return(n(2)),
        ]));
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
        let why = with_late(&case, |_, vm| {
            vm.vars.actors[1].set_access(VariableName::new("x"), Access::Public);
        })
        .expect_err("a disagreement");
        assert!(
            why.contains("vm vs walker: variable map 2: access of "),
            "{why}"
        );
        assert!(why.contains("Some(Public) vs Some(Private)"), "{why}");
        // The variable the program writes: its slot is not new, so it keeps the access it had.
        let why = with_late(&case, |_, vm| {
            vm.vars
                .local
                .set_access(VariableName::new("a"), Access::Public);
        })
        .expect_err("a disagreement");
        assert!(why.contains("variable map 0: access of "), "{why}");
        // An actor's map, and the other direction: the tree walker's side is the second.
        let why = with_late(&case, |_, vm| {
            vm.vars.actors[2].set_access(VariableName::new("x"), Access::Private);
        })
        .expect_err("a disagreement");
        assert!(
            why.contains("variable map 3: ") && why.contains("Some(Private) vs Some(Public)"),
            "{why}"
        );
    }

    #[test]
    fn a_planted_difference_in_a_public_snapshot_is_reported() {
        let case = simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))));
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
        // Actor 2's `x` is public with the snapshot 7 and the value 8: an update refreshes the
        // snapshot.
        let why = with_late(&case, |_, vm| vm.vars.actors[2].refresh_snapshots())
            .expect_err("a disagreement");
        assert!(why.contains("variable map 3: public snapshot of "), "{why}");
        assert!(
            why.contains("Some(Float(8.0)) vs Some(Float(7.0))"),
            "{why}"
        );
        // A variable with no snapshot made public on one side.
        let why = with_late(&case, |_, vm| {
            vm.vars
                .local
                .set_access(VariableName::new("x"), Access::Public);
        })
        .expect_err("a disagreement");
        assert!(why.contains("variable map 0: access of "), "{why}");
        // A public variable declared without a value is seen through the count of public variables.
        let why = with_late(&case, |_, vm| {
            vm.vars.actors[4].set_access(VariableName::new("ghost"), Access::Public);
        })
        .expect_err("a disagreement");
        assert!(
            why.contains("variable map 5: has public variables: true vs false"),
            "{why}"
        );
    }

    #[test]
    fn a_difference_in_access_between_eval_and_eval_f32_is_reported_before_the_tree_walker_is_asked()
     {
        let case = stale_handler();
        assert_eq!(differential(&case), Ok(Verdict::VmOnly));
        let planted = plant(
            None,
            Some(Box::new(|_, again| {
                again.vars.actors[1].set_access(VariableName::new("x"), Access::Public);
            })),
        );
        let why = differential(&case).expect_err("the early difference is found");
        assert!(
            why.contains("eval and eval_f32 leave different states: variable map 2: access of "),
            "{why}"
        );
        drop(planted);
    }

    #[test]
    fn a_planted_temp_difference_is_reported_with_kept_temps() {
        let case = FuzzCase {
            temps: TempLifetime::Persistent,
            ..case(Program::Complex(vec![
                Stmt::Expr(Ex::Assign(Var::plain(Ns::Temp, 0), b(n(1)))),
                Stmt::Return(n(2)),
            ]))
        };
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
        let why = with_late(&case, |_, vm| {
            vm.temps
                .as_mut()
                .expect("persistent temps")
                .set(TempName::new("a"), Value::Float(9.0));
        })
        .expect_err("a disagreement");
        assert!(why.contains("temps: "), "{why}");
        let why = with_late(&case, |_, vm| vm.temps = None).expect_err("a disagreement");
        assert!(why.contains("temps: None vs Some("), "{why}");
    }

    #[test]
    fn a_planted_message_difference_is_reported() {
        let case = simple(n(1));
        // Even a constant is compared: the state it leaves is part of the verdict.
        let why = with_late(&case, |_, vm| vm.sink.messages.push("extra".to_owned()))
            .expect_err("a disagreement");
        assert!(why.contains("messages: [\"extra\"] vs []"), "{why}");
        let case = simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))));
        let why = with_late(&case, |_, vm| vm.sink.messages.push("extra".to_owned()))
            .expect_err("a disagreement");
        assert!(why.contains("messages: [\"extra\"] vs []"), "{why}");
    }

    #[test]
    fn a_planted_random_difference_is_reported() {
        let case = FuzzCase {
            forced_random: None,
            ..simple(Ex::Math(24, vec![n(0), n(1)]))
        };
        assert_eq!(with_late(&case, |_, _| {}), Ok(Verdict::Agree));
        let why = with_late(&case, |_, vm| {
            sample(&mut vm.rng);
        })
        .expect_err("a disagreement");
        assert!(why.contains("random source: Xorshift("), "{why}");
        let forced = FuzzCase {
            forced_random: Some(FixedRng::HALF),
            ..case
        };
        let why = with_late(&forced, |_, vm| vm.rng = FuzzRng::Fixed(QUARTER))
            .expect_err("a disagreement");
        assert!(
            why.contains(
                "random source: Fixed(FixedRng(536870912)) vs Fixed(FixedRng(1073741824))"
            ),
            "{why}"
        );
    }

    #[test]
    fn a_difference_between_eval_and_eval_f32_is_reported_before_the_tree_walker_is_asked() {
        let case = simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))));
        let planted = plant(None, Some(Box::new(|value, _| *value = Value::Float(99.0))));
        let why = differential(&case).expect_err("eval_f32 disagrees");
        assert!(
            why.contains("eval_f32 3.5 is not the float of eval Float(99.0)"),
            "{why}"
        );
        drop(planted);
        let _planted = plant(
            None,
            Some(Box::new(|_, again| {
                again
                    .vars
                    .local
                    .set(VariableName::new("x"), Value::Float(0.0));
            })),
        );
        let why = differential(&case).expect_err("the states differ");
        assert!(
            why.contains("eval and eval_f32 leave different states: variable map 0: "),
            "{why}"
        );
    }

    #[test]
    fn a_disagreement_names_the_source_the_version_and_the_limits() {
        let case = FuzzCase {
            version: 9,
            steps: 321,
            ..simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))))
        };
        let why =
            with_late(&case, |value, _| *value = Value::Float(0.0)).expect_err("a disagreement");
        assert!(
            why.starts_with('"') && why.contains("(v9, EvalLimits {"),
            "{why}"
        );
        assert!(why.contains("total_steps: Some(321)"), "{why}");
        assert!(why.contains("struct_depth: Some(32)"), "{why}");
    }

    /// Runs `case` and returns the value, the environment (limits, messages, rng, subject, temps)
    /// the VM ended with.
    fn observe(case: &FuzzCase) -> (Result<Verdict, String>, V, FuzzEnv) {
        let seen: Rc<RefCell<Option<(V, FuzzEnv)>>> = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&seen);
        let verdict = with_late(case, move |value, vm| {
            *sink.borrow_mut() = Some((value.clone(), vm.clone()));
        });
        let (value, env) = seen.borrow_mut().take().expect("the tree walker was asked");
        (verdict, value, env)
    }

    #[test]
    fn the_step_budget_of_the_case_ends_both_evaluations() {
        let program = Program::Complex(vec![
            Stmt::Expr(Ex::Loop(
                b(n(5)),
                vec![Stmt::Expr(Ex::Assign(
                    Var::plain(Ns::Entity, 7),
                    b(Ex::Bin(BinOp::Add, b(entity(7)), b(n(1)))),
                ))],
            )),
            Stmt::Return(entity(7)),
        ]);
        let (verdict, value, env) = observe(&FuzzCase {
            steps: 12,
            ..case(program.clone())
        });
        assert_eq!(verdict, Ok(Verdict::Agree));
        assert_eq!(value, Value::ZERO);
        assert_eq!(env.limits.total_steps, Some(12));
        assert_eq!(
            env.sink.messages,
            ["molangx: evaluation stopped after its budget of 12 steps"]
        );
        let (_, value, env) = observe(&case(program));
        assert_eq!(env.limits.total_steps, Some(5000));
        assert!(
            env.sink.messages.iter().all(|m| !m.contains("budget")),
            "{:?}",
            env.sink.messages
        );
        assert_ne!(value, Value::ZERO);
    }

    #[test]
    fn the_loop_budget_of_the_case_ends_a_loop() {
        let program = Program::Complex(vec![
            Stmt::Expr(Ex::Assign(Var::plain(Ns::Entity, 7), b(n(0)))),
            Stmt::Expr(Ex::Loop(
                b(n(6)),
                vec![Stmt::Expr(Ex::Assign(
                    Var::plain(Ns::Entity, 7),
                    b(Ex::Bin(BinOp::Add, b(entity(7)), b(n(1)))),
                ))],
            )),
            Stmt::Return(entity(7)),
        ]);
        let (verdict, value, env) = observe(&FuzzCase {
            loop_iterations: 3,
            ..case(program.clone())
        });
        assert_eq!(verdict, Ok(Verdict::Agree));
        assert_eq!(value, Value::Float(3.0));
        assert_eq!(env.limits.loop_iterations, Some(3));
        assert_eq!(
            env.sink.messages,
            ["molangx: loop stopped after its budget of 3 iterations"]
        );
        let (_, value, env) = observe(&FuzzCase {
            loop_iterations: 64,
            ..case(program)
        });
        assert_eq!(value, Value::Float(10.0));
        assert!(env.sink.messages.is_empty());
    }

    #[test]
    fn the_width_budget_of_the_case_ends_a_store() {
        let member = |value: u8, member: u8| {
            Stmt::Expr(Ex::Assign(var(Ns::Entity, 4, &[member]), b(n(value))))
        };
        let program = Program::Complex(vec![
            member(1, 0),
            member(2, 1),
            member(3, 2),
            Stmt::Return(n(5)),
        ]);
        let (verdict, value, env) = observe(&FuzzCase {
            struct_members: 2,
            ..case(program.clone())
        });
        assert_eq!(verdict, Ok(Verdict::Agree));
        assert_eq!(
            value,
            Value::ZERO,
            "the third member does not fit: the whole structure was two wide already"
        );
        assert_eq!(env.limits.struct_members, Some(2));
        let (_, value, env) = observe(&case(program));
        assert_eq!(value, Value::Float(7.0), "n(5) is the pool's 7");
        assert_eq!(
            env.limits.struct_members,
            Some(EvalLimits::DEFAULT_STRUCT_MEMBERS)
        );
    }

    #[test]
    fn the_depth_budget_is_the_default_whatever_the_case() {
        let (_, _, env) = observe(&simple(Ex::Bin(BinOp::Add, b(x()), b(n(2)))));
        assert_eq!(
            env.limits.struct_depth,
            Some(EvalLimits::DEFAULT_STRUCT_DEPTH)
        );
        assert_eq!(env.limits.struct_depth, Some(32));
    }

    #[test]
    fn the_query_depth_of_the_case_limits_nested_arguments() {
        let nested = Ex::Query(0, vec![Ex::Query(0, vec![n(1), n(2)]), n(2)]);
        let (verdict, value, env) = observe(&FuzzCase {
            query_depth: Some(1),
            ..simple(nested.clone())
        });
        assert_eq!(verdict, Ok(Verdict::Agree));
        assert_eq!(value, Value::ZERO);
        assert_eq!(env.limits.query_depth, Some(1));
        assert_eq!(
            env.sink.messages,
            [
                "molangx: evaluation stopped: query arguments would nest deeper than their budget of 1 levels"
            ]
        );
        let (_, value, env) = observe(&FuzzCase {
            query_depth: None,
            ..simple(nested)
        });
        assert_eq!(value, Value::Float(1.0));
        assert!(env.sink.messages.is_empty());
    }

    #[test]
    fn the_forced_random_sample_replaces_the_generator() {
        let random = simple(Ex::Math(24, vec![n(0), n(1)]));
        let (_, value, env) = observe(&FuzzCase {
            forced_random: Some(QUARTER),
            ..random.clone()
        });
        assert_eq!(value, Value::Float(0.25));
        assert_eq!(env.rng, FuzzRng::Fixed(QUARTER));
        let (_, value, env) = observe(&FuzzCase {
            forced_random: None,
            ..random
        });
        assert_eq!(
            value.as_f32().to_bits(),
            sample(&mut Xorshift128::new()).to_bits()
        );
        assert!(matches!(env.rng, FuzzRng::Xorshift(_)));
    }

    #[test]
    fn the_subject_and_the_temp_lifetime_of_the_case_are_the_environments() {
        let (_, _, env) = observe(&FuzzCase {
            subject: Subject::Actor,
            temps: TempLifetime::Persistent,
            ..simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))))
        });
        assert_eq!(env.subject, Some(FuzzActor::Handle(1)));
        assert!(env.temps.is_some());
        let (_, _, env) = observe(&FuzzCase {
            subject: Subject::Detached,
            temps: TempLifetime::PerEvaluation,
            ..simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))))
        });
        assert_eq!(env.subject, None);
        assert!(env.temps.is_none());
    }

    #[test]
    fn the_actor_subject_reads_the_actors_map_and_otherwise_the_detached_one() {
        // `v.x` is 1.5 in both; a write lands in the map the subject selects.
        let write = case(Program::Complex(vec![
            Stmt::Expr(Ex::Assign(Var::plain(Ns::Entity, 7), b(n(1)))),
            Stmt::Return(n(2)),
        ]));
        let (_, _, with_subject) = observe(&FuzzCase {
            subject: Subject::Actor,
            ..write.clone()
        });
        assert_eq!(
            with_subject.vars.actors[1].get(VariableName::new("a")),
            Some(&Value::Float(1.0))
        );
        assert_eq!(
            with_subject.vars.local.get(VariableName::new("a")),
            Some(&Value::Float(0.0)),
            "the detached `a` is untouched"
        );
        let (_, _, without) = observe(&FuzzCase {
            subject: Subject::Detached,
            ..write
        });
        assert_eq!(
            without.vars.local.get(VariableName::new("a")),
            Some(&Value::Float(1.0))
        );
        assert_eq!(
            without.vars.actors[1].get(VariableName::new("a")),
            Some(&Value::Float(0.0))
        );
    }

    #[test]
    fn the_version_of_the_case_selects_the_division_rule() {
        // v.y is -3: from version 7 the divisor keeps its sign, up to 6 it is made positive.
        let division = |version| {
            let (verdict, value, _) = observe(&FuzzCase {
                version,
                ..simple(Ex::Bin(BinOp::Div, b(n(2)), b(entity(1))))
            });
            assert_eq!(verdict, Ok(Verdict::Agree), "version {version}");
            value.as_f32()
        };
        assert_eq!(division(13), 2.0 / -3.0);
        assert_eq!(division(7), 2.0 / -3.0);
        assert_eq!(division(6), 2.0 / 3.0);
        assert_eq!(division(1), 2.0 / 3.0);
    }

    #[test]
    fn the_nan_divisor_rule_is_the_architectures() {
        // v.n is a NaN: the division gives NaN on `X86_64` and 0 on `Arm64`.
        let (verdict, value, _) = observe(&simple(Ex::Bin(BinOp::Div, b(n(1)), b(entity(2)))));
        assert_eq!(verdict, Ok(Verdict::Agree));
        assert_eq!(value.as_f32().is_nan(), ARCH == Arch::X86_64);
        if ARCH == Arch::Arm64 {
            assert_eq!(value.as_f32(), 0.0);
        }
    }

    #[test]
    fn the_style_of_the_case_decides_the_text_that_is_run() {
        let plain = simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))));
        let spaced = FuzzCase {
            style: Style {
                structure: vec![],
                cosmetic: vec![200, 31, 7, 9, 255, 1, 2, 3, 4],
            },
            ..plain.clone()
        };
        assert_ne!(plain.source(), spaced.source());
        assert_eq!(differential(&plain), Ok(Verdict::Agree));
        assert_eq!(differential(&spaced), Ok(Verdict::Agree));
        let (_, a, _) = observe(&plain);
        let (_, b, _) = observe(&spaced);
        assert_eq!(a, b, "cosmetics change no value");
    }

    #[test]
    fn a_version_outside_the_range_compiles_or_is_not_compiled_but_never_disagrees() {
        for version in [-2i16, -1, 0, 14, 15] {
            let verdict = differential(&FuzzCase {
                version,
                ..simple(Ex::Bin(BinOp::Add, b(x()), b(n(2))))
            });
            assert!(
                matches!(verdict, Ok(Verdict::Agree | Verdict::NotCompiled)),
                "version {version}: {verdict:?}"
            );
        }
    }

    #[test]
    fn generated_cases_never_disagree() {
        let mut agreed = 0;
        for seed in 0..300u64 {
            let case = case_of(&buffer(seed, 160));
            match differential(&case) {
                Ok(Verdict::Agree) => agreed += 1,
                Ok(_) => {}
                Err(why) => panic!("seed {seed}: {why}"),
            }
        }
        assert!(
            agreed > 30,
            "only {agreed} of 300 generated cases were compared"
        );
    }
}
