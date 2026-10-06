//! Operators, comparisons, math functions, dice and random draws.

use super::walker::{child, moved_constant, with_post};
use super::{Eval, Unwind, Walker};
use molangx::catalog::{MAX_MATH_ARGS, MathImpl};
use molangx::hash::HashedStr;
use molangx::internals::{Node, Payload, math_decl};
use molangx::numeric::{self, PostOp, arith};
use molangx::ops::ExpressionOp as Op;
use molangx::rng;
use molangx::stdlib::math;
use molangx::vm::{EvalCx, Host, Value};

type Unary = fn(f32, PostOp) -> f32;

type Binary = fn(f32, f32, PostOp) -> f32;

type Ternary = fn(f32, f32, f32, PostOp) -> f32;

/// `math.random` or `math.random_integer`.
#[derive(Copy, Clone)]
enum Draw {
    Float,
    Integer,
}

impl<H: Host> Walker<H> {
    /// The value of an operator or math-function node; any other op takes the generic constant
    /// path. A thin dispatcher, to keep stack frames small.
    pub(super) fn arithmetic(&mut self, op: Op, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        match op {
            Op::Add => self.add(n, cx),
            Op::Mul => self.mul(n, cx),
            Op::Div => self.div(n, cx),
            // A literal divisor has no zero test, a run-time one does.
            Op::Mod if n.children().len() == 1 => self.binary_math(math::mod_const, n, cx),
            Op::Mod => self.binary_math(math::mod_runtime, n, cx),
            Op::Atan2 => self.binary_math(math::atan2, n, cx),
            Op::CopySign => self.binary_math(math::copy_sign, n, cx),
            Op::Max => self.binary_math(math::max, n, cx),
            Op::Min => self.binary_math(math::min, n, cx),
            Op::Pow => self.binary_math(math::pow, n, cx),
            Op::LessThan => self.comparison(numeric::lt, n, cx),
            Op::LessEqual => self.comparison(numeric::le, n, cx),
            Op::GreaterEqual => self.comparison(numeric::ge, n, cx),
            Op::GreaterThan => self.comparison(numeric::gt, n, cx),
            Op::LogicalEqual => {
                let equal = self.equality(n, cx)?;
                Ok(Value::Float(n.post().select(equal)))
            }
            Op::LogicalNotEqual => {
                let equal = self.equality(n, cx)?;
                Ok(Value::Float(n.post().select(!equal)))
            }
            Op::Random => self.random(Draw::Float, n, cx),
            Op::RandomInt => self.random(Draw::Integer, n, cx),
            Op::DieRoll => self.die_roll(math::DieRoll::new, n, cx),
            Op::DieRollInt => self.die_roll(math::DieRoll::new_integer, n, cx),
            Op::HostMath | Op::HostMathVolatile => self.host_math(n, cx),
            op => match (unary(op), ternary(op)) {
                (Some(f), _) => self.unary_math(f, n, cx),
                (None, Some(f)) => self.ternary_math(f, n, cx),
                // `Return` outside a statement list, `Pi`, `{`, the closing tokens, `:` and `,`
                // take the generic constant path: the node's value with its post-op.
                (None, None) => {
                    self.step(cx)?;
                    Ok(Value::Float(n.post().apply(moved_constant(n))))
                }
            },
        }
    }

    /// `a * b` (`Mul`): `a` is pushed, `b` is the accumulator; `acc·(top·S) + O`.
    #[inline(never)]
    fn mul(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        // Which operand is scaled first is shared with the VM.
        let (a, b) = self.two(n, cx)?;
        Ok(Value::Float(numeric::mul(b, a, n.post())))
    }

    /// `a / b` (`Div`): the divisor first, then the guard; the numerator only if it passes.
    #[inline(never)]
    fn div(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let d = self.node(child(n, 1), cx)?.as_f32();
        self.step(cx)?;
        match numeric::div_guard(self.signed_division, d) {
            None => Ok(Value::ZERO),
            Some(divisor) => {
                let a = self.node(child(n, 0), cx)?.as_f32();
                self.step(cx)?;
                Ok(Value::Float(numeric::div(a, divisor, n.post())))
            }
        }
    }

    /// A two-operand math function. With one child the constant operand folded into the node is
    /// the second operand.
    #[inline(never)]
    fn binary_math(&mut self, f: Binary, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let (a, b) = self.operands(n, cx)?;
        Ok(Value::Float(f(a, b, n.post())))
    }

    /// A prefix operator or a one-operand math function.
    #[inline(never)]
    fn unary_math(&mut self, f: Unary, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let x = self.node(child(n, 0), cx)?.as_f32();
        self.step(cx)?;
        Ok(Value::Float(f(x, n.post())))
    }

    /// `clamp`, the interpolations and the easings: three operands in order.
    #[inline(never)]
    fn ternary_math(&mut self, function: Ternary, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let (a, b) = self.two(n, cx)?;
        let c = self.node(child(n, 2), cx)?.as_f32();
        self.step(cx)?;
        Ok(Value::Float(function(a, b, c, n.post())))
    }

    /// A host math function: every argument but the last pushed, in order, then the call; the
    /// same function as the VM's, on the same random source.
    #[inline(never)]
    fn host_math(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let mut args = [0.0; MAX_MATH_ARGS as usize];
        let count = n.children().len().min(args.len());
        for (arg, c) in args.iter_mut().zip(n.children()) {
            *arg = self.node(c, cx)?.as_f32();
            // The push after each argument but the last, then the call itself.
            self.step(cx)?;
        }
        let (Payload::HostMath(function), Some(catalog)) = (n.value(), &self.math) else {
            return Ok(Value::ZERO);
        };
        let value = match math_decl(catalog, *function).implementation() {
            MathImpl::Pure(call) => call(&args[..count]),
            MathImpl::Volatile(call) => call(&mut *cx.rng, &args[..count]),
        };
        Ok(Value::Float(n.post().apply(value)))
    }

    /// `math.die_roll` / `math.die_roll_integer`, whose roll `start` begins: one sample per roll,
    /// drawn inside the loop; every roll costs a step.
    #[inline(never)]
    fn die_roll(
        &mut self,
        start: fn(f32, f32, f32) -> math::DieRoll,
        n: &Node,
        cx: &mut EvalCx<'_, '_, H>,
    ) -> Eval<H> {
        let (count, a) = self.two(n, cx)?;
        let b = self.node(child(n, 2), cx)?.as_f32();
        self.step(cx)?;
        let mut roll = start(count, a, b);
        while roll.remaining() > 0 {
            self.step(cx)?;
            roll.roll(rng::sample(cx.rng));
        }
        Ok(Value::Float(roll.finish(n.post())))
    }

    /// `<`, `<=`, `>=`, `>`: the node's true or false constant.
    #[inline(never)]
    fn comparison(
        &mut self,
        holds: fn(f32, f32) -> bool,
        n: &Node,
        cx: &mut EvalCx<'_, '_, H>,
    ) -> Eval<H> {
        let (a, b) = self.operands(n, cx)?;
        Ok(Value::Float(n.post().select(holds(a, b))))
    }

    /// Whether the operands of `==` / `!=` are equal: on values, not floats.
    #[inline(never)]
    fn equality(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Result<bool, Unwind<H>> {
        // Dispatches on the right operand's kind and reads the left payload as that kind; a moved
        // constant is the right operand.
        let a = self.node(child(n, 0), cx)?;
        self.step(cx)?;
        let b = if n.children().len() == 1 {
            match *n.value() {
                Payload::Hash(hash) => Value::Hash(HashedStr::from_u64(hash)),
                _ => Value::Float(n.float()),
            }
        } else {
            let b = self.node(child(n, 1), cx)?;
            self.step(cx)?;
            b
        };
        Ok(a.molang_eq(&b, |actor| cx.resolve_actor(actor)))
    }

    /// The two operands of a binary node as floats, or the one operand and the constant moved
    /// into the node.
    fn operands(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Result<(f32, f32), Unwind<H>> {
        // A constant operand counts as the second operand whichever side it was written on (shared
        // with the VM).
        if n.children().len() == 1 {
            let a = self.node(child(n, 0), cx)?.as_f32();
            self.step(cx)?;
            Ok((a, moved_constant(n)))
        } else {
            self.two(n, cx)
        }
    }

    /// A post-op applied where two paths meet (`?:`, `??`): one step when there is one.
    pub(super) fn post_step(
        &mut self,
        value: Value<H>,
        post: PostOp,
        cx: &mut EvalCx<'_, '_, H>,
    ) -> Eval<H> {
        if post.is_identity() {
            return Ok(value);
        }
        self.step(cx)?;
        Ok(with_post(value, post))
    }

    /// The two operands of a binary node as floats: `a`, (push), `b`, (the operation).
    fn two(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Result<(f32, f32), Unwind<H>> {
        let a = self.node(child(n, 0), cx)?;
        self.step(cx)?;
        let b = self.node(child(n, 1), cx)?;
        self.step(cx)?;
        Ok((a.as_f32(), b.as_f32()))
    }

    /// The n-ary `+` (`Add`): the first term pushed, the middle ones accumulated into it, the
    /// last one added with the post-op.
    #[inline(never)]
    fn add(&mut self, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let post = n.post();
        let Some((first, rest)) = n.children().split_first() else {
            return Ok(Value::ZERO);
        };
        let Some((last, middle)) = rest.split_last() else {
            // A one-term `+` never survives optimisation: the term with the post-op.
            let value = self.node(first, cx)?;
            return self.post_step(value, post, cx);
        };
        let mut sum = self.node(first, cx)?.as_f32();
        self.step(cx)?;
        for term in middle {
            let value = self.node(term, cx)?.as_f32();
            self.step(cx)?;
            sum = arith::add(sum, value);
        }
        let value = self.node(last, cx)?.as_f32();
        self.step(cx)?;
        Ok(Value::Float(numeric::add(value, sum, post)))
    }

    /// `math.random` / `math.random_integer`: one sample, drawn after both bounds are
    /// evaluated. Two literal bounds are folded into the instruction and are not evaluated.
    #[inline(never)]
    fn random(&mut self, draw: Draw, n: &Node, cx: &mut EvalCx<'_, '_, H>) -> Eval<H> {
        let post = n.post();
        let (a, b) = (child(n, 0), child(n, 1));
        if a.is(Op::Float) && b.is(Op::Float) {
            self.step(cx)?;
            let sample = rng::sample(cx.rng);
            let (a, b) = (a.float(), b.float());
            return Ok(Value::Float(match draw {
                Draw::Float => math::random_folded(sample, math::random_const_bounds(a, b, post)),
                Draw::Integer => math::random_integer_const_bounds(a, b, sample, post),
            }));
        }
        let (a, b) = self.two(n, cx)?;
        let sample = rng::sample(cx.rng);
        Ok(Value::Float(match draw {
            Draw::Float => math::random(a, b, sample, post),
            Draw::Integer => math::random_integer(a, b, sample, post),
        }))
    }
}

/// The prefix operators and the one-operand math functions.
fn unary(op: Op) -> Option<Unary> {
    Some(match op {
        Op::Negate => numeric::negate,
        Op::LogicalNot => numeric::not,
        Op::Abs => math::abs,
        Op::Acos => math::acos,
        Op::Asin => math::asin,
        Op::Atan => math::atan,
        Op::Ceil => math::ceil,
        Op::Cos => math::cos,
        Op::Exp => math::exp,
        Op::Floor => math::floor,
        Op::HermiteBlend => math::hermite_blend,
        Op::Ln => math::ln,
        Op::MinAngle => math::min_angle,
        Op::Round => math::round,
        Op::Sin => math::sin,
        Op::Sign => math::sign,
        Op::Sqrt => math::sqrt,
        Op::Trunc => math::trunc,
        _ => return None,
    })
}

/// The three-operand math functions: `clamp`, the interpolations, the 30 easings.
fn ternary(op: Op) -> Option<Ternary> {
    Some(match op {
        Op::Clamp => math::clamp,
        Op::Lerp => math::lerp,
        Op::LerpRotate => math::lerprotate,
        Op::InverseLerp => math::inverse_lerp,
        Op::EaseInQuad => math::ease_in_quad,
        Op::EaseOutQuad => math::ease_out_quad,
        Op::EaseInOutQuad => math::ease_in_out_quad,
        Op::EaseInCubic => math::ease_in_cubic,
        Op::EaseOutCubic => math::ease_out_cubic,
        Op::EaseInOutCubic => math::ease_in_out_cubic,
        Op::EaseInQuart => math::ease_in_quart,
        Op::EaseOutQuart => math::ease_out_quart,
        Op::EaseInOutQuart => math::ease_in_out_quart,
        Op::EaseInQuint => math::ease_in_quint,
        Op::EaseOutQuint => math::ease_out_quint,
        Op::EaseInOutQuint => math::ease_in_out_quint,
        Op::EaseInSine => math::ease_in_sine,
        Op::EaseOutSine => math::ease_out_sine,
        Op::EaseInOutSine => math::ease_in_out_sine,
        Op::EaseInExpo => math::ease_in_expo,
        Op::EaseOutExpo => math::ease_out_expo,
        Op::EaseInOutExpo => math::ease_in_out_expo,
        Op::EaseInCirc => math::ease_in_circ,
        Op::EaseOutCirc => math::ease_out_circ,
        Op::EaseInOutCirc => math::ease_in_out_circ,
        Op::EaseInBounce => math::ease_in_bounce,
        Op::EaseOutBounce => math::ease_out_bounce,
        Op::EaseInOutBounce => math::ease_in_out_bounce,
        Op::EaseInBack => math::ease_in_back,
        Op::EaseOutBack => math::ease_out_back,
        Op::EaseInOutBack => math::ease_in_out_back,
        Op::EaseInElastic => math::ease_in_elastic,
        Op::EaseOutElastic => math::ease_out_elastic,
        Op::EaseInOutElastic => math::ease_in_out_elastic,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_walker::test_support::*;
    use molangx::compile::compile;
    use molangx::numeric::{ARCH, Arch};
    use molangx::rng::{Xorshift128, sample};

    #[test]
    fn arithmetic_with_post_ops() {
        assert_eq!(float("v.x + v.y"), 1.0);
        assert_eq!(float("v.x - v.y"), 5.0);
        assert_eq!(float("v.x * v.y"), -6.0);
        assert_eq!(float("v.x * 2 + 1"), 7.0);
        assert_eq!(float("1 + v.x + v.y + 4"), 6.0);
        assert_eq!(float("-v.x"), -3.0);
        assert_eq!(float("- (v.x * 2)"), -6.0);
        assert_eq!(float("!v.x"), 0.0);
        assert_eq!(float("!v.zero"), 1.0);
        assert_eq!(float("v.x * v.y * v.x"), -18.0);
        assert_eq!(float("(v.x + 1) * 3 - 2"), 10.0);
    }

    #[test]
    fn division_follows_the_version_guard() {
        // `v.y` is -2: up to version 6 the guard divides by its absolute value; a zero divisor
        // gives 0 at every version.
        for (version, expected) in [(4i16, 1.5f32), (6, 1.5), (7, -1.5), (13, -1.5)] {
            let expr = build_at("v.x / v.zero", version);
            let (vm, walker) = (vm_run(&expr, &Env::new()), walker_run(&expr, &Env::new()));
            assert_agree("x / 0", &vm, &walker);
            assert_eq!(walker.f(), 0.0, "version {version}");
            let expr = build_at("v.x / v.y", version);
            let (vm, walker) = (vm_run(&expr, &Env::new()), walker_run(&expr, &Env::new()));
            assert_agree("x / y", &vm, &walker);
            assert_eq!(walker.f(), expected, "version {version}");
        }
    }

    #[test]
    fn a_divisor_below_epsilon_in_magnitude_is_guarded_in_both_signs_and_versions() {
        for (divisor, version) in [
            (1.0e-9f32, 6i16),
            (-1.0e-9, 6),
            (1.0e-9, 7),
            (-1.0e-9, 7),
            (0.0, 7),
            (-0.0, 6),
        ] {
            let start = Env::new().with_var("d", Value::Float(divisor));
            let expr = build_at("v.x / v.d", version);
            let (vm, walker) = (vm_run(&expr, &start), walker_run(&expr, &start));
            assert_agree("x / d", &vm, &walker);
            assert_eq!(walker.f(), 0.0, "{divisor} at version {version}");
        }
    }

    #[test]
    fn a_nan_divisor_follows_the_architecture() {
        // A NaN divisor gives NaN on `X86_64` and 0 on `Arm64`.
        let start = Env::new().with_var("d", Value::Float(f32::NAN));
        let compiled = compile("v.x / v.d", &options(13));
        let expr = compiled.expr().cloned().expect("compiles");
        let (vm, walker) = (vm_run(&expr, &start), walker_run(&expr, &start));
        assert_agree("x / NaN", &vm, &walker);
        assert_eq!(walker.f().is_nan(), ARCH == Arch::X86_64);
    }

    #[test]
    fn a_zero_divisor_skips_the_numerator() {
        // The numerator would draw a random sample or assign; neither happens.
        let run = both("return math.random(0, v.x * 0 + 1) / v.zero;");
        assert_eq!(run.f(), 0.0);
        assert_eq!(
            run.env.rng,
            FuzzRng::Xorshift(Xorshift128::new()),
            "no draw"
        );
    }

    #[test]
    fn modulo_with_a_literal_and_a_run_time_divisor() {
        assert_eq!(float("math.mod(v.x, 2)"), 1.0);
        assert_eq!(float("math.mod(v.x, v.zero)"), 0.0);
        // A literal divisor has no zero test (the plain remainder): NaN.
        assert!(float("math.mod(v.x, 0)").is_nan());
        assert_eq!(float("math.mod(7, v.x) * 2"), 2.0);
    }

    #[test]
    fn comparisons_give_zero_or_one() {
        assert_eq!(float("v.x < v.y"), 0.0);
        assert_eq!(float("v.x > v.y"), 1.0);
        assert_eq!(float("v.x <= 3"), 1.0);
        assert_eq!(float("v.x >= 4"), 0.0);
        assert_eq!(float("v.y < 0"), 1.0);
    }

    #[test]
    fn equality_between_floats_strings_and_mixed_kinds() {
        assert_eq!(float("v.x == 3"), 1.0);
        assert_eq!(float("v.x != 3"), 0.0);
        assert_eq!(float("v.x == v.y"), 0.0);
        assert_eq!(float("v.s == 'moo'"), 1.0);
        assert_eq!(float("v.s != 'moo'"), 0.0);
        assert_eq!(float("v.s == 'cow'"), 0.0);
        assert_eq!(float("v.s == v.s"), 1.0);
        // Mixed kinds: a string and a number are never equal.
        assert_eq!(float("v.s == 3"), 0.0);
        assert_eq!(float("v.s != 3"), 1.0);
        assert_eq!(float("v.x == 'moo'"), 0.0);
        assert_eq!(float("v.x != 'moo'"), 1.0);
        // Actors are compared as resolved identities.
        assert_eq!(float("c.other == c.other"), 1.0);
        assert_eq!(float("c.other == v.x"), 0.0);
    }

    #[test]
    fn equality_with_a_moved_constant_and_a_variable_operand() {
        let start = Env::new()
            .with_var("t", Value::Float(1.0))
            .with_var("u", Value::string("cow"));
        assert_eq!(both_on("v.t == 1", &start).f(), 1.0);
        assert_eq!(both_on("v.u == 'cow'", &start).f(), 1.0);
        assert_eq!(both_on("v.u == v.t", &start).f(), 0.0);
        assert_eq!(both_on("(v.t == 1) == (v.u == 'cow')", &start).f(), 1.0);
    }

    #[test]
    fn every_arity_of_math_function() {
        assert_eq!(float("math.abs(v.y)"), 2.0);
        assert_eq!(float("math.ceil(v.x / 2)"), 2.0);
        assert_eq!(float("math.floor(v.x / 2)"), 1.0);
        assert_eq!(float("math.sqrt(v.x * 3)"), 3.0);
        assert_eq!(float("math.sign(v.y)"), -1.0);
        assert_eq!(float("math.trunc(v.y / 4)"), 0.0);
        assert_eq!(float("math.round(v.x / 2)"), 2.0);
        assert_eq!(float("math.cos(v.zero)"), 1.0);
        assert_eq!(float("math.sin(v.zero)"), 0.0);
        assert_eq!(float("math.exp(v.zero)"), 1.0);
        assert_eq!(float("math.ln(v.x - 2)"), 0.0);
        assert_eq!(float("math.atan(v.zero)"), 0.0);
        assert_eq!(float("math.acos(v.x - 2)"), 0.0);
        assert_eq!(float("math.asin(v.zero)"), 0.0);
        assert_eq!(float("math.min_angle(v.x * 120)"), 0.0);
        assert_eq!(float("math.hermite_blend(v.zero)"), 0.0);
        assert_eq!(float("math.max(v.x, v.y)"), 3.0);
        assert_eq!(float("math.min(v.x, v.y)"), -2.0);
        assert_eq!(float("math.pow(v.x, 2)"), 9.0);
        assert_eq!(float("math.pow(2, v.x)"), 8.0);
        assert_eq!(float("math.copy_sign(v.x, v.y)"), -3.0);
        assert_eq!(float("math.atan2(v.zero, v.x)"), 0.0);
        assert_eq!(float("math.mod(v.x, v.x)"), 0.0);
        assert_eq!(float("math.clamp(v.x, 0, 2)"), 2.0);
        assert_eq!(float("math.clamp(v.y, 0, 2)"), 0.0);
        assert_eq!(float("math.lerp(v.zero, 10, 0.5)"), 5.0);
        assert_eq!(float("math.inverse_lerp(v.zero, 10, 5)"), 0.5);
        assert_eq!(float("math.lerprotate(v.zero, 90, 0.5)"), 45.0);
        assert_eq!(float("math.ease_in_quad(0, 1, v.x / 6)"), 0.25);
        assert_eq!(float("math.ease_out_quad(0, 1, v.zero)"), 0.0);
        assert_eq!(float("math.ease_in_out_sine(0, 1, v.zero + 1)"), 1.0);
        assert_eq!(float("math.ease_in_back(0, v.x, v.zero)"), 0.0);
        assert_eq!(float("math.pi"), std::f32::consts::PI);
    }

    #[test]
    fn a_constant_operand_moved_into_a_math_node_is_its_second_operand() {
        assert_eq!(float("math.max(v.y, 1)"), 1.0);
        assert_eq!(float("math.max(1, v.y)"), 1.0);
        assert_eq!(float("math.min(v.x, 1)"), 1.0);
        assert_eq!(float("math.pow(v.x, 2)"), 9.0);
        assert_eq!(float("math.mod(v.x, 2)"), 1.0);
        assert_eq!(float("math.mod(7, v.x)"), 1.0);
    }

    #[test]
    fn math_post_ops_are_applied_once() {
        assert_eq!(float("math.abs(v.y) * 2 + 1"), 5.0);
        assert_eq!(float("math.max(v.x, v.y) * 3 - 1"), 8.0);
        assert_eq!(float("math.clamp(v.x, 0, 2) * 2"), 4.0);
    }

    #[test]
    fn the_random_operators_with_a_fixed_sample() {
        let at = |sample: f32, src: &str| both_on(src, &Env::new().with_fixed_random(sample)).f();
        // Literal bounds are folded into the instruction.
        assert_eq!(at(0.5, "math.random(2, 4)"), 3.0);
        assert_eq!(at(0.0, "math.random(2, 4)"), 2.0);
        assert_eq!(at(1.0, "math.random(2, 4)"), 4.0);
        // Run-time bounds are evaluated first, left to right, then the sample is drawn.
        assert_eq!(at(0.5, "math.random(v.zero, v.x * 2)"), 3.0);
        assert_eq!(at(0.5, "math.random(v.y, v.x)"), 0.5);
        assert_eq!(at(0.5, "math.random_integer(v.zero, v.x * 2)"), 3.0);
        assert_eq!(at(0.0, "math.random_integer(v.zero, v.x * 2)"), 0.0);
        assert_eq!(at(0.5, "math.random_integer(1, 5)"), 3.0);
        // The post-op is applied to the result.
        assert_eq!(at(0.5, "math.random(2, 4) * 2 + 1"), 7.0);
    }

    #[test]
    fn the_random_operators_draw_once_in_evaluation_order() {
        let run = both("math.random(0, 1) + math.random(0, 1) * 2");
        let mut rng = Xorshift128::new();
        let (a, b) = (sample(&mut rng), sample(&mut rng));
        assert_eq!(bits(run.f()), bits(a + b * 2.0));
        assert_eq!(run.env.rng, FuzzRng::Xorshift(rng));
        // Bounds that draw are evaluated before the sample.
        let run = both("math.random(math.random(0, 1), 2)");
        let mut rng = Xorshift128::new();
        let first = sample(&mut rng);
        let second = sample(&mut rng);
        assert_eq!(bits(run.f()), bits(first + (2.0 - first) * second));
    }

    #[test]
    fn host_math_functions_run_the_same_function_in_order() {
        // `v.x` is 3: half the larger less a quarter of the second, then the post-op.
        assert_eq!(float("math.helper_mix(v.x, 4) * 2 + 1"), 3.0);
        assert_eq!(float("math.helper_mix(4, v.x) * 2 + 1"), 3.5);
        // Each argument times its position.
        assert_eq!(float("math.helper_sum(v.x, 1, 10, -v.x)"), 23.0);
        assert_eq!(float("math.helper_sum(-v.x, 10, 1, v.x)"), 32.0);
        assert_eq!(
            float("math.helper_sum(v.x, 1, 2, 4, 8, 16, 32, -v.x)"),
            363.0
        );
        assert_eq!(
            float("math.helper_sum(1, 2, 4, 8, 16, 32, 64, 128)"),
            1793.0
        );
        // A pure call of constants folds; the folder and the evaluators agree.
        assert_eq!(float("math.helper_mix(1, 4)"), 1.0);
        let run = both("math.random(0, 1) + math.helper_noise(v.x) * 2 + math.random(0, 1) * 4");
        let mut rng = Xorshift128::new();
        let (a, b, c) = (sample(&mut rng), sample(&mut rng), sample(&mut rng));
        assert_eq!(bits(run.f()), bits(a + (3.0 + b) * 2.0 + c * 4.0));
        assert_eq!(run.env.rng, FuzzRng::Xorshift(rng));
    }

    #[test]
    fn a_host_math_call_costs_its_pushes_and_one_step() {
        let start = Env::new();
        assert_eq!(
            smallest_budget("math.helper_sum(v.x, v.y, v.x)", &start),
            smallest_budget("math.clamp(v.x, v.y, v.x)", &start)
        );
        assert_eq!(
            smallest_budget("math.helper_noise(v.x)", &start),
            smallest_budget("math.abs(v.x)", &start)
        );
        // Load, push, load, call, end.
        steps("math.helper_mix(v.x, v.y)", 5);
        steps("math.helper_noise(v.x) + 1", 3);
    }

    #[test]
    fn die_rolls_draw_once_per_roll() {
        let run = both_on("math.die_roll(3, 1, 6)", &Env::new().with_fixed_random(0.5));
        assert_eq!(run.f(), 3.0 * 3.5);
        let run = both_on(
            "math.die_roll_integer(v.x, 1, 6)",
            &Env::new().with_fixed_random(0.0),
        );
        assert_eq!(run.f(), 3.0);
        let run = both("math.die_roll(v.x, 0, 1)");
        let mut rng = Xorshift128::new();
        for _ in 0..3 {
            sample(&mut rng);
        }
        assert_eq!(run.env.rng, FuzzRng::Xorshift(rng));
        let run = both("math.die_roll(0, 0, 1)");
        assert_eq!(run.f(), 0.0);
        assert_eq!(
            run.env.rng,
            FuzzRng::Xorshift(Xorshift128::new()),
            "zero rolls draw nothing"
        );
    }

    #[test]
    fn the_unary_table_names_the_matching_math_function() {
        let table: [(Op, Unary); 16] = [
            (Op::Abs, math::abs),
            (Op::Acos, math::acos),
            (Op::Asin, math::asin),
            (Op::Atan, math::atan),
            (Op::Ceil, math::ceil),
            (Op::Cos, math::cos),
            (Op::Exp, math::exp),
            (Op::Floor, math::floor),
            (Op::HermiteBlend, math::hermite_blend),
            (Op::Ln, math::ln),
            (Op::MinAngle, math::min_angle),
            (Op::Round, math::round),
            (Op::Sin, math::sin),
            (Op::Sign, math::sign),
            (Op::Sqrt, math::sqrt),
            (Op::Trunc, math::trunc),
        ];
        for (op, expected) in table {
            for x in [-0.5f32, 0.25, 0.75, 2.5, -7.0] {
                let got = unary(op).expect("a unary function")(x, PostOp::IDENTITY);
                let want = expected(x, PostOp::IDENTITY);
                assert_eq!(
                    got.to_bits(),
                    want.to_bits(),
                    "{op:?}({x}): {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn the_unary_table_distinguishes_its_functions() {
        // Guards the table above against a degenerate "every entry is trunc": these differ at -0.5.
        let at = |op: Op| unary(op).expect("a unary function")(-0.5, PostOp::IDENTITY);
        assert_eq!(at(Op::Abs), 0.5);
        assert_eq!(at(Op::Ceil), -0.0);
        assert_eq!(at(Op::Floor), -1.0);
        assert_eq!(at(Op::Sign), -1.0);
        assert_eq!(at(Op::Trunc), -0.0);
        assert!(at(Op::Sqrt).is_nan());
        assert!(at(Op::Ln).is_nan());
    }

    #[test]
    fn an_op_with_no_unary_function_has_none() {
        for op in [Op::Add, Op::Max, Op::Float, Op::Loop] {
            assert!(unary(op).is_none(), "{op:?}");
        }
    }

    #[test]
    fn the_ternary_table_has_exactly_the_thirty_four_ops_of_clamp_lerp_and_the_easings() {
        let with: Vec<Op> = molangx::ops::OpSet::all()
            .iter()
            .filter(|&op| ternary(op).is_some())
            .collect();
        assert_eq!(with.len(), 34, "{with:?}");
        let names: Vec<&str> = with.iter().map(|op| op.meta().name).collect();
        for expected in [
            "Clamp",
            "Lerp",
            "LerpRotate",
            "InverseLerp",
            "EaseInQuad",
            "EaseInOutElastic",
            "EaseOutBounce",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} is missing from {names:?}"
            );
        }
        assert_eq!(names.iter().filter(|n| n.starts_with("Ease")).count(), 30);
        for op in [
            Op::Max,
            Op::Min,
            Op::Add,
            Op::Abs,
            Op::Pi,
            Op::Return,
            Op::Loop,
            Op::Mod,
        ] {
            assert!(ternary(op).is_none(), "{op:?}");
        }
    }

    #[test]
    fn the_ternary_table_computes_what_its_name_says() {
        let id = PostOp::IDENTITY;
        assert_eq!(ternary(Op::Clamp).map(|f| f(5.0, 0.0, 2.0, id)), Some(2.0));
        assert_eq!(ternary(Op::Lerp).map(|f| f(0.0, 10.0, 0.5, id)), Some(5.0));
        assert_eq!(
            ternary(Op::InverseLerp).map(|f| f(0.0, 10.0, 5.0, id)),
            Some(0.5)
        );
        assert_eq!(
            ternary(Op::EaseInQuad).map(|f| f(0.0, 1.0, 0.5, id)),
            Some(0.25)
        );
        assert_eq!(
            ternary(Op::EaseOutQuad).map(|f| f(0.0, 1.0, 0.5, id)),
            Some(0.75)
        );
    }

    #[test]
    fn post_step_pays_a_step_only_for_a_real_post_op() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        assert_eq!(
            walker
                .post_step(Value::Float(2.0), PostOp::IDENTITY, &mut cx)
                .ok(),
            Some(Value::Float(2.0))
        );
        assert_eq!(walker.steps, 0);
        assert_eq!(
            walker
                .post_step(Value::Float(2.0), PostOp::new(3.0, 1.0), &mut cx)
                .ok(),
            Some(Value::Float(7.0))
        );
        assert_eq!(walker.steps, 1);
    }

    #[test]
    fn operands_use_the_moved_constant_as_the_second_operand() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        // One child and a constant in the node's payload: (child, constant).
        let one = Node::new(
            Op::Max,
            Payload::Float(9.0),
            PostOp::IDENTITY,
            vec![number(4.0)],
        );
        assert_eq!(walker.operands(&one, &mut cx).ok(), Some((4.0, 9.0)));
        assert_eq!(walker.steps, 2, "the literal and the push after it");
        let two = node(Op::Max, vec![number(4.0), number(5.0)]);
        assert_eq!(walker.operands(&two, &mut cx).ok(), Some((4.0, 5.0)));
        assert_eq!(walker.steps, 2 + 4, "both literals and both pushes");
    }

    #[test]
    fn a_binary_math_node_with_a_moved_constant_uses_the_literal_divisor_form_of_mod() {
        let mut walker = walker_for("v.x");
        let mut env = Env::new();
        let mut cx = env.cx();
        let one = Node::new(
            Op::Mod,
            Payload::Float(0.0),
            PostOp::IDENTITY,
            vec![number(7.0)],
        );
        let literal = walker
            .arithmetic(Op::Mod, &one, &mut cx)
            .ok()
            .map(|v| v.as_f32());
        assert!(
            literal.is_some_and(f32::is_nan),
            "a literal zero divisor has no zero test: {literal:?}"
        );
        let two = node(Op::Mod, vec![number(7.0), number(0.0)]);
        assert_eq!(
            walker
                .arithmetic(Op::Mod, &two, &mut cx)
                .ok()
                .map(|v| v.as_f32()),
            Some(0.0)
        );
    }
}
