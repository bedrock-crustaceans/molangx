//! [`EvalLimits`]: the evaluator's host-protection budgets.

/// The budgets of one evaluation.
///
/// A per-loop budget alone does not bound nested loops, hence the per-evaluation step budget.
///
/// # What the step budget bounds
///
/// [`EvalLimits::total_steps`] counts one step per executed instruction and per die of a die
/// roll, plus every operation whose cost grows with the size of a value:
/// - a member store (`v.a.b = x`) costs, for every struct on its path, its member count plus
///   [`EvalLimits::STRUCT_COPY_STEPS`] (the copy-on-write copy it may make, or the struct it
///   creates);
/// - storing an actor array in a variable or temp costs one step per entry.
///
/// So one evaluation allocates at most about 50 bytes per step (a member is 24 bytes and a copy
/// may double its capacity), some 50 MB under [`EvalLimits::DEFAULT`]. With every other budget
/// off, `total_steps` alone still bounds an evaluation's time and memory.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EvalLimits {
    /// Iterations one `loop` / `for_each` may run before it is left; `None`: no limit. A `loop`
    /// always runs its first iteration, so `Some(0)` leaves it after one, like `Some(1)`; a
    /// `for_each` runs none.
    ///
    /// Only the first loop left in an evaluation sends a
    /// [`RuntimeMsg::LoopLimit`](super::RuntimeMsg::LoopLimit).
    pub loop_iterations: Option<u32>,
    /// Steps one top-level evaluation may take before it ends with 0.0 and a
    /// [`RuntimeMsg::StepLimit`](super::RuntimeMsg::StepLimit); `None`: no limit, `Some(0)`: every
    /// evaluation that runs an instruction ends at once (a folded constant still returns its
    /// value).
    pub total_steps: Option<u64>,
    /// Struct levels a member store may leave in the variable or temp it writes
    /// ([`Value::struct_depth`](super::Value::struct_depth)); `None`: no limit, `Some(0)`: every
    /// member store is refused.
    ///
    /// A deeper store writes nothing and ends the evaluation with 0.0 and a
    /// [`RuntimeMsg::StructDepthLimit`](super::RuntimeMsg::StructDepthLimit).
    pub struct_depth: Option<u32>,
    /// Members a member store may leave in a struct it adds a member to; `None`: no limit,
    /// `Some(0)`: every store adding a member is refused.
    ///
    /// A wider store writes nothing and ends the evaluation with 0.0 and a
    /// [`RuntimeMsg::StructMemberLimit`](super::RuntimeMsg::StructMemberLimit). Storing into an
    /// existing member is never refused, so a wider struct the host stored stays usable.
    pub struct_members: Option<u32>,
    /// How deep query arguments may nest at run time (the top level is 0); `None`: no limit,
    /// `Some(0)`: no query may evaluate an argument.
    ///
    /// An argument that would run deeper ends the evaluation with 0.0 and a
    /// [`RuntimeMsg::QueryDepthLimit`](super::RuntimeMsg::QueryDepthLimit). This is the
    /// evaluator's only native recursion: a host whose threads have less stack than
    /// [`Expr::eval`](crate::compile::Expr::eval) requires sets it.
    pub query_depth: Option<u32>,
}

impl EvalLimits {
    /// Default per-loop iteration budget ([`EvalLimits::loop_iterations`]).
    pub const DEFAULT_LOOP_ITERATIONS: u32 = 1_024;

    /// Default per-evaluation step budget.
    pub const DEFAULT_TOTAL_STEPS: u64 = 1_048_576;

    /// The steps a member store pays per struct on its path on top of one per member: the
    /// allocation and four inline member slots, so storing through small structs is not free.
    pub const STRUCT_COPY_STEPS: u64 = 4;

    /// The most operands the operand stack may hold at a loop's back edge; applies under every
    /// budget, including [`EvalLimits::NONE`].
    ///
    /// Past it the evaluation ends with 0 and a
    /// [`RuntimeMsg::OperandStackLimit`](super::RuntimeMsg::OperandStackLimit). Only operands that
    /// `break` / `continue` leave behind accumulate; 65,536 slots are a few megabytes.
    pub const OPERAND_STACK_CAP: u32 = 65_536;

    /// Default struct nesting budget.
    ///
    /// It bounds the depth of a walk of a stored value, not the nodes it visits: structs share
    /// members, so a walk that revisits shared structs can take time exponential in the depth.
    /// Walk with [`Value::distinct_structs`](super::Value::distinct_structs) or bound the walk.
    pub const DEFAULT_STRUCT_DEPTH: u32 = 32;

    /// Default struct width budget; keeps one struct, and each copy of it, below about 6 KiB.
    pub const DEFAULT_STRUCT_MEMBERS: u32 = 256;

    /// The defaults: [`EvalLimits::DEFAULT_LOOP_ITERATIONS`] iterations per loop,
    /// [`EvalLimits::DEFAULT_TOTAL_STEPS`] steps, [`EvalLimits::DEFAULT_STRUCT_DEPTH`] levels,
    /// [`EvalLimits::DEFAULT_STRUCT_MEMBERS`] members and no query-argument budget.
    pub const DEFAULT: Self = Self {
        loop_iterations: Some(Self::DEFAULT_LOOP_ITERATIONS),
        total_steps: Some(Self::DEFAULT_TOTAL_STEPS),
        struct_depth: Some(Self::DEFAULT_STRUCT_DEPTH),
        struct_members: Some(Self::DEFAULT_STRUCT_MEMBERS),
        query_depth: None,
    };

    /// No limits: an expression may run for ever and build structs of any size.
    pub const NONE: Self = Self {
        loop_iterations: None,
        total_steps: None,
        struct_depth: None,
        struct_members: None,
        query_depth: None,
    };

    /// Whether every budget is off.
    pub const fn is_unlimited(self) -> bool {
        self.loop_iterations.is_none()
            && self.total_steps.is_none()
            && self.struct_depth.is_none()
            && self.struct_members.is_none()
            && self.query_depth.is_none()
    }
}

impl Default for EvalLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn eval_limits() {
        let limits = EvalLimits::default();
        assert_eq!(
            limits.loop_iterations,
            Some(EvalLimits::DEFAULT_LOOP_ITERATIONS)
        );
        assert_eq!(limits.loop_iterations, Some(1024));
        assert_eq!(limits.total_steps, Some(1_048_576));
        assert_eq!(limits.struct_depth, Some(32));
        assert!(!limits.is_unlimited());
        assert!(EvalLimits::NONE.is_unlimited());
    }

    #[test]
    fn default_matches_the_documented_budgets() {
        assert_eq!(
            EvalLimits::DEFAULT,
            EvalLimits {
                loop_iterations: Some(1024),
                total_steps: Some(1_048_576),
                struct_depth: Some(32),
                struct_members: Some(256),
                query_depth: None,
            }
        );
        assert_eq!(EvalLimits::default(), EvalLimits::DEFAULT);
        assert_eq!(EvalLimits::DEFAULT_TOTAL_STEPS, 1_048_576);
        assert_eq!(EvalLimits::DEFAULT_STRUCT_DEPTH, 32);
        assert_eq!(EvalLimits::DEFAULT_STRUCT_MEMBERS, 256);
        assert_eq!(EvalLimits::STRUCT_COPY_STEPS, 4);
        assert_eq!(EvalLimits::OPERAND_STACK_CAP, 65_536);
    }

    #[test]
    fn none_switches_every_budget_off() {
        assert_eq!(
            EvalLimits::NONE,
            EvalLimits {
                loop_iterations: None,
                total_steps: None,
                struct_depth: None,
                struct_members: None,
                query_depth: None,
            }
        );
        assert!(EvalLimits::NONE.is_unlimited());
    }

    #[test]
    fn zero_is_a_budget_not_off() {
        let zero = EvalLimits {
            loop_iterations: Some(0),
            total_steps: Some(0),
            struct_depth: Some(0),
            struct_members: Some(0),
            query_depth: Some(0),
        };
        assert!(!zero.is_unlimited());
        let largest = EvalLimits {
            loop_iterations: Some(u32::MAX),
            total_steps: Some(u64::MAX),
            struct_depth: Some(u32::MAX),
            struct_members: Some(u32::MAX),
            query_depth: Some(u32::MAX),
        };
        assert!(!largest.is_unlimited());
        assert_ne!(largest, EvalLimits::NONE);
    }

    #[test]
    fn one_budget_alone_makes_the_limits_not_unlimited() {
        let base = EvalLimits::NONE;
        let cases = [
            EvalLimits {
                loop_iterations: Some(5),
                ..base
            },
            EvalLimits {
                total_steps: Some(5),
                ..base
            },
            EvalLimits {
                struct_depth: Some(5),
                ..base
            },
            EvalLimits {
                struct_members: Some(5),
                ..base
            },
            EvalLimits {
                query_depth: Some(5),
                ..base
            },
        ];
        for limits in cases {
            assert!(!limits.is_unlimited(), "{limits:?}");
        }
    }

    #[test]
    fn limits_are_copy_eq_hash_and_debug() {
        let a = EvalLimits::DEFAULT;
        let b = a;
        assert_eq!(a, b);
        let set: HashSet<EvalLimits> = [EvalLimits::DEFAULT, EvalLimits::DEFAULT, EvalLimits::NONE]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 2);
        let text = format!("{a:?}");
        assert!(text.contains("loop_iterations: Some(1024)"), "{text}");
        assert!(text.contains("total_steps: Some(1048576)"), "{text}");
        assert!(text.contains("struct_depth: Some(32)"), "{text}");
        assert!(text.contains("struct_members: Some(256)"), "{text}");
        assert!(text.contains("query_depth: None"), "{text}");
    }
}
