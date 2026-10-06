//! The semantic passes: the optimiser, the validator and the tree-level part of the link.
//!
//! [`simplify`] is one post-order walk that checks the shape of every construct, flattens argument
//! lists and statement groups, folds constant sub-trees with the evaluator's arithmetic, and folds
//! `x·c`, `x + c` and `−x` into the post-op of the node they apply to; any error rejects. A
//! constant operand folds to its value first, so a negation, term or factor around it is constant
//! arithmetic, not a post-op (see [`crate::numeric`] for where the results differ).
//! [`validate_root`] logs every finding but rejects only for one at the root.
//!
//! Folding reads a constant child's raw value and ignores its post-op, and a node replaced by a
//! float keeps its post-op: that post-op counts where the constant moves into its parent
//! (`S·v + O`) and not where it stays a child.

use crate::compile::{Cx, Failed, Pass, ast::Node};
use crate::diag::LanguageMessage as Msg;
use crate::ops::ExpressionOp as Op;

mod fold;
mod numerical;
mod shape;
mod sum;
mod validate;

use fold::after_children;
use shape::{before_children, is_leaf};
pub(super) use validate::{contains_op, validate_root};

/// A node at this depth (the root at 0) is rejected with E22.
const DEPTH_LIMIT: u32 = crate::compile::MAX_DEPTH + 1;

/// Clamp, the dice, the interpolations and the easings.
const fn is_three_argument_math(op: Op) -> bool {
    op.is_math_function() && matches!(op.max_children(), Some(3))
}

/// Replaces `node` by its child at `index`, post-op included.
fn lift_child(node: &mut Node, index: usize) -> &mut Node {
    let child = node.children.swap_remove(index);
    *node = child;
    node
}

/// The optimiser: post-order on an explicit stack, whose height is the depth.
///
/// A child is taken out of its parent's list while it is optimised (a placeholder holds its place)
/// and put back, so no list is copied.
pub(super) fn simplify(cx: &mut Cx<'_>, root: &mut Node) -> Pass {
    struct Frame {
        node: Node,
        /// The number of children entered so far; the last of them is out while it is optimised.
        entered: usize,
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut entering = Some(std::mem::replace(root, Node::placeholder()));
    let mut finished: Option<Node> = None;
    loop {
        if let Some(mut node) = entering.take() {
            if stack.len() >= DEPTH_LIMIT as usize {
                cx.language(Msg::DepthOverflow, node.span, &[]);
                return Err(Failed);
            }
            if is_leaf(&node) {
                finished = Some(node);
            } else {
                before_children(cx, &mut node)?;
                stack.push(Frame { node, entered: 0 });
            }
        }
        let Some(frame) = stack.last_mut() else {
            if let Some(node) = finished {
                *root = node;
            }
            return Ok(());
        };
        if let Some(child) = finished.take()
            && let Some(slot) = frame
                .entered
                .checked_sub(1)
                .and_then(|at| frame.node.children.get_mut(at))
        {
            *slot = child;
        }
        if let Some(slot) = frame.node.children.get_mut(frame.entered) {
            frame.entered += 1;
            entering = Some(std::mem::replace(slot, Node::placeholder()));
            continue;
        }
        let Some(Frame { mut node, .. }) = stack.pop() else {
            return Ok(());
        };
        // Trim lists that grew while the passes built them; the kept tree must not outgrow copies.
        node.children.shrink_to_fit();
        after_children(cx, &mut node)?;
        finished = Some(node);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Helpers shared by the optimiser and validator unit tests; nothing here goes through
    //! `compile`.

    use super::*;
    use crate::catalog::Side;
    use crate::compile::{
        CompileOptions,
        ast::{Name, Payload, Span},
        lex, parse,
    };
    use crate::diag::Severity;
    use crate::hash::HashedStr;
    use crate::numeric::PostOp;
    use crate::version::RawVersion;

    pub(super) type Logged = (&'static str, Severity, (u32, u32));

    pub(super) fn client(raw: i16) -> CompileOptions {
        CompileOptions::from_raw_version(
            crate::stdlib::queries(Side::Client).clone(),
            RawVersion(raw),
        )
    }

    pub(super) fn logged(cx: &Cx<'_>) -> Vec<Logged> {
        cx.logged_diagnostics()
            .iter()
            .map(|d| {
                (
                    d.language_message().map_or("-", |v| v.id()),
                    d.severity(),
                    (d.span().start, d.span().end),
                )
            })
            .collect()
    }

    pub(super) fn texts(cx: &Cx<'_>) -> Vec<String> {
        cx.logged_diagnostics()
            .iter()
            .map(|d| d.message().into_owned())
            .collect()
    }

    pub(super) struct Optimised {
        pub root: Node,
        pub ok: bool,
        pub log: Vec<Logged>,
        pub texts: Vec<String>,
    }

    pub(super) fn optimise_with(src: &str, opts: &CompileOptions) -> Optimised {
        let mut cx = Cx::for_test(src, opts);
        let tokens = lex::scan(&mut cx, &lex::lower(src)).expect("the source lexes");
        let used = tokens.used;
        let mut root = parse::group_tokens(&mut cx, tokens.nodes, used).expect("the source groups");
        let ok = simplify(&mut cx, &mut root).is_ok();
        Optimised {
            root,
            ok,
            log: logged(&cx),
            texts: texts(&cx),
        }
    }

    pub(super) fn tree_at(src: &str, raw: i16) -> Node {
        let out = optimise_with(src, &client(raw));
        assert!(
            out.ok,
            "{src:?} was rejected at version {raw}: {:?}",
            out.texts
        );
        assert!(out.log.is_empty(), "{src:?} logged: {:?}", out.texts);
        out.root
    }

    pub(super) fn tree(src: &str) -> Node {
        tree_at(src, 13)
    }

    pub(super) fn shown(src: &str) -> String {
        tree(src).tree_notation(9)
    }

    pub(super) fn rejected_at(src: &str, raw: i16) -> Optimised {
        let out = optimise_with(src, &client(raw));
        assert!(
            !out.ok,
            "{src:?} was accepted at version {raw}: {}",
            out.root.tree_notation(9)
        );
        assert_eq!(
            out.root,
            Node::placeholder(),
            "{src:?}: a failed optimisation leaves the placeholder root"
        );
        out
    }

    /// The `(id, span)` of the only message `src` is rejected with at version 13.
    pub(super) fn rejected(src: &str) -> (&'static str, (u32, u32)) {
        let out = rejected_at(src, 13);
        assert_eq!(out.log.len(), 1, "{src:?}: {:?}", out.texts);
        assert_eq!(out.log[0].1, Severity::Error, "{src:?}");
        (out.log[0].0, out.log[0].2)
    }

    pub(super) fn rejected_text(src: &str) -> String {
        let out = rejected_at(src, 13);
        assert_eq!(out.texts.len(), 1, "{src:?}: {:?}", out.texts);
        out.texts.into_iter().next().unwrap_or_default()
    }

    pub(super) fn at(mut node: Node, start: u32, end: u32) -> Node {
        node.span = Span::new(start, end);
        node
    }

    pub(super) fn float(v: f32) -> Node {
        Node::token(Op::Float, Payload::Float(v), Span::new(0, 1))
    }

    pub(super) fn entity(name: &str) -> Node {
        Node::token(
            Op::EntityVariable,
            Payload::Entity(Name::new(format!("variable.{name}"))),
            Span::new(0, 1),
        )
    }

    pub(super) fn temp(name: &str) -> Node {
        Node::token(
            Op::TempVariable,
            Payload::Temp(Name::new(format!("temp.{name}"))),
            Span::new(0, 1),
        )
    }

    pub(super) fn string(text: &str) -> Node {
        Node::token(
            Op::StringLiteral,
            Payload::Hash(HashedStr::new(text).as_u64()),
            Span::new(0, 1),
        )
    }

    pub(super) fn parent(op: Op, children: Vec<Node>) -> Node {
        let mut node = Node::token(op, Payload::None, Span::new(0, 1));
        node.children = children;
        node
    }

    pub(super) fn member(name: &str, base: Node) -> Node {
        let mut node = Node::token(
            Op::MemberAccessor,
            Payload::Member(Name::new(name)),
            Span::new(0, 1),
        );
        node.children.push(base);
        node
    }

    pub(super) fn with_post(mut node: Node, scale: f32, offset: f32) -> Node {
        node.post = PostOp::new(scale, offset);
        node
    }

    /// A call as the grouping passes leave it: `op[ ( [args as a left-nested comma chain] ) ]`.
    pub(super) fn call(op: Op, args: Vec<Node>) -> Node {
        let mut args = args.into_iter();
        let mut chain = args.next().expect("a call has an argument");
        for arg in args {
            chain = parent(Op::Comma, vec![chain, arg]);
        }
        parent(op, vec![parent(Op::LeftParenthesis, vec![chain])])
    }

    /// A `{ … }` block of statements as the grouping passes leave it: `{ [; [group[stmt]…]] }`.
    pub(super) fn block(statements: Vec<Node>) -> Node {
        let groups = statements
            .into_iter()
            .map(|statement| group(vec![statement]))
            .collect();
        parent(Op::LeftBrace, vec![parent(Op::Semicolon, groups)])
    }

    /// The `Semicolon` node the grouping passes gather a statement under.
    pub(super) fn group(children: Vec<Node>) -> Node {
        parent(Op::Semicolon, children)
    }

    pub(super) fn statements_root(statements: Vec<Node>) -> Node {
        parent(
            Op::Semicolon,
            statements.into_iter().map(|s| group(vec![s])).collect(),
        )
    }

    pub(super) fn optimise_node(
        root: &mut Node,
        opts: &CompileOptions,
    ) -> (bool, Vec<Logged>, Vec<String>) {
        let mut cx = Cx::for_test("abcdefghijklmnopqrstuvwxyz", opts);
        let ok = simplify(&mut cx, root).is_ok();
        (ok, logged(&cx), texts(&cx))
    }

    pub(super) fn folded(mut root: Node) -> Node {
        let (ok, log, texts) = optimise_node(&mut root, &client(13));
        assert!(ok && log.is_empty(), "{texts:?}");
        root
    }

    pub(super) fn assert_float_bits(node: &Node, value: f32) {
        assert!(node.is(Op::Float), "not a float: {}", node.tree_notation(9));
        assert_eq!(
            node.float().to_bits(),
            value.to_bits(),
            "value {} != {value}",
            node.float()
        );
    }

    pub(super) fn assert_post_bits(node: &Node, scale: f32, offset: f32) {
        assert_eq!(
            (node.post.scale.to_bits(), node.post.offset.to_bits()),
            (scale.to_bits(), offset.to_bits()),
            "post-op ({}, {}) != ({scale}, {offset})",
            node.post.scale,
            node.post.offset
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ast::Span;
    use crate::compile::sema::test_support::*;
    use crate::diag::Severity;

    /// `count` nested `!` around a `1`: the leaf is at depth `count`.
    fn not_chain(count: usize) -> Node {
        let mut node = float(1.0);
        for index in 0..count {
            node = at(
                parent(Op::LogicalNot, vec![node]),
                index as u32,
                index as u32 + 1,
            );
        }
        node
    }

    #[test]
    fn three_argument_math_is_clamp_the_dice_the_lerps_and_the_easings() {
        let mut count = 0;
        for &op in Op::all() {
            let expected = matches!(
                op,
                Op::Clamp
                    | Op::DieRoll
                    | Op::DieRollInt
                    | Op::Lerp
                    | Op::LerpRotate
                    | Op::InverseLerp
            ) || format!("{op:?}").starts_with("Ease");
            assert_eq!(is_three_argument_math(op), expected, "{op:?}");
            count += usize::from(expected);
        }
        // clamp, die_roll, die_roll_integer, lerp, lerprotate, inverse_lerp, and 30 easings.
        assert_eq!(count, 36);
    }

    #[test]
    fn children_are_optimised_in_place_and_in_order() {
        let root = tree("v.a = 1 + 2; v.b = v.c * 2; t.x = math.abs(-3); return v.a;");
        assert_eq!(
            root.tree_notation(9),
            "(Semicolon (Assignment v.a) (Assignment v.b [v.c*2+0]) (Assignment t.x) (Return v.a))"
        );
        assert_eq!(root.children.capacity(), root.children.len());
    }

    #[test]
    fn lift_child_replaces_the_node_by_the_whole_child() {
        let child = with_post(
            at(parent(Op::Add, vec![entity("a"), entity("b")]), 5, 9),
            2.0,
            3.0,
        );
        let mut node = parent(Op::Negate, vec![child]);
        let lifted = lift_child(&mut node, 0);
        assert!(lifted.is(Op::Add));
        assert_eq!(lifted.children.len(), 2);
        assert_post_bits(lifted, 2.0, 3.0);
        assert_eq!(lifted.span, Span::new(5, 9));
        assert!(node.is(Op::Add));
    }

    #[test]
    fn lift_child_takes_the_indexed_child() {
        let mut node = parent(Op::Mul, vec![float(2.0), with_post(entity("x"), 4.0, 5.0)]);
        let lifted = lift_child(&mut node, 1);
        assert!(lifted.is(Op::EntityVariable));
        assert_post_bits(lifted, 4.0, 5.0);
    }

    #[test]
    fn a_leaf_at_depth_255_is_accepted_and_one_at_256_is_not() {
        let mut accepted = not_chain(255);
        let (ok, log, _) = optimise_node(&mut accepted, &client(13));
        assert!(ok && log.is_empty());
        assert_float_bits(&accepted, 0.0);

        let mut rejected = not_chain(256);
        let (ok, log, texts) = optimise_node(&mut rejected, &client(13));
        assert!(!ok);
        // The node entered at depth 256 is the innermost one: the leaf, whose span is (0, 1).
        assert_eq!(log, [("E22", Severity::Error, (0, 1))]);
        assert_eq!(
            texts,
            [
                "Error: Expression could not be parsed due to stack depth overflow (too many sub-expressions)"
            ]
        );
        assert_eq!(
            rejected,
            Node::placeholder(),
            "a failed optimisation leaves the placeholder root"
        );
    }

    #[test]
    fn the_depth_limit_is_one_more_than_the_public_maximum() {
        assert_eq!(DEPTH_LIMIT, 256);
        assert_eq!(DEPTH_LIMIT, crate::compile::MAX_DEPTH + 1);
    }

    #[test]
    fn nested_prefix_operators_count_one_level_each() {
        for (count, ok) in [(254, true), (255, true), (256, false), (300, false)] {
            let src = format!("{}1", "!".repeat(count));
            let out = optimise_with(&src, &client(13));
            assert_eq!(out.ok, ok, "{count} nested !");
            assert_eq!(out.log.len(), usize::from(!ok));
        }
    }

    #[test]
    fn nested_parentheses_count_one_level_each() {
        for (count, ok) in [(255, true), (256, false)] {
            let src = format!("{}1{}", "(".repeat(count), ")".repeat(count));
            assert_eq!(
                optimise_with(&src, &client(13)).ok,
                ok,
                "{count} nested parentheses"
            );
        }
    }

    #[test]
    fn nested_argument_lists_count_one_level_each() {
        // `math.min(1, …)`: the parenthesis node is replaced by the arguments before the children
        // are entered, so a nesting costs one level.
        for (count, ok) in [(255, true), (256, false)] {
            let src = format!("{}1{}", "math.min(1,".repeat(count), ")".repeat(count));
            let out = optimise_with(&src, &client(13));
            assert_eq!(out.ok, ok, "{count} nested math.min");
            if !ok {
                assert_eq!(out.log.iter().map(|m| m.0).collect::<Vec<_>>(), ["E22"]);
            }
        }
    }

    #[test]
    fn nested_one_argument_functions_count_two_levels_each() {
        // `math.abs(…)` keeps its parenthesis node until the child is done: two levels a nesting.
        for (count, ok) in [(127, true), (128, false)] {
            let src = format!("{}1{}", "math.abs(".repeat(count), ")".repeat(count));
            assert_eq!(
                optimise_with(&src, &client(13)).ok,
                ok,
                "{count} nested math.abs"
            );
        }
    }

    #[test]
    fn the_depth_limit_is_counted_without_native_recursion() {
        // 512 KiB of stack cannot hold a recursion over 100,000 levels, in any build.
        let result = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let mut deep = not_chain(100_000);
                let (ok, log, _) = optimise_node(&mut deep, &client(13));
                assert!(!ok);
                assert_eq!(log.len(), 1);
                assert_eq!(log[0].0, "E22");
                let mut edge = not_chain(255);
                let (ok, log, _) = optimise_node(&mut edge, &client(13));
                assert!(ok && log.is_empty());
            })
            .expect("a thread spawns")
            .join();
        assert!(result.is_ok());
    }

    #[test]
    fn optimising_replaces_the_root_in_place() {
        let mut root = parent(Op::Add, vec![float(1.0), float(2.0)]);
        let (ok, log, _) = optimise_node(&mut root, &client(13));
        assert!(ok && log.is_empty());
        assert_float_bits(&root, 3.0);
    }
}
