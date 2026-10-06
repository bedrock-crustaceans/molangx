//! The iterative driver every grouping pass runs on.
//!
//! A pass changes a list only where one of its tokens stands as a leaf. The driver runs a [`Level`]
//! only on such a list, walks other lists in place, and skips a node with nothing of the pass below
//! it ([`Node::below`], all bits set until a pass has looked). A note stays true for the rest of
//! the grouping because passes only take leaves out of a subtree; the one exception, the unary pass
//! turning `-` into `+`, is why the class of `-` includes the class of `+`.

use super::{Failed, Pass, is_math_call};
use crate::compile::{Cx, ast::Node};
use crate::ops::ExpressionOp as Op;
use std::mem::take;
use std::ops::{BitOr, BitOrAssign, Range};

/// A set of token classes ([`class`]): what may stand as a leaf below a node ([`Node::below`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(in crate::compile) struct TokenClasses(u32);

impl TokenClasses {
    pub(in crate::compile) const NONE: Self = Self(0);
    /// Every class: the note of a node no pass has looked below yet.
    pub(in crate::compile) const ALL: Self = Self(u32::MAX);

    /// Whether the two sets share a class.
    #[inline]
    fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl BitOr for TokenClasses {
    type Output = Self;

    #[inline]
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for TokenClasses {
    #[inline]
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// One pass's work on one list.
///
/// Instead of recursing, [`step`](Level::step) hands out a node's children; [`run`] works them off
/// on its own stack and gives the result back through [`resume`](Level::resume). Lists are entered
/// and messages logged in the order a recursive descent would.
pub(super) trait Level: Sized {
    /// Whether the pass walks its lists from the right.
    const RIGHT_TO_LEFT: bool = false;

    /// Works on the list until the children of some node have to be processed first (returns
    /// them), or until the list is done (returns `None`).
    fn step(&mut self, cx: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed>;

    /// Takes back the processed children handed out by the last [`step`](Level::step), and
    /// whether they still hold an operator token the pass did not fold.
    fn resume(&mut self, children: Vec<Node>, unfolded: bool);

    /// The finished list, and whether it still holds an operator token the pass did not fold.
    fn finish(self, cx: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed>;
}

/// The class of `op` for [`Node::below`]: one per group of tokens one pass folds, none for the
/// tokens no pass after the sections looks for.
pub(super) fn class(op: Op) -> TokenClasses {
    let bit = match op {
        Op::QueryFunction | Op::Loop | Op::ForEach => 0,
        Op::Semicolon => 1,
        Op::ArrayVariable => 2,
        Op::Pointer => 3,
        Op::LogicalNot => 4,
        // The unary pass turns a binary `-` into `+`.
        Op::Negate => return class(Op::LogicalNot) | class(Op::Add),
        Op::Div => 5,
        Op::Mul => 6,
        Op::Add => 7,
        Op::LessThan => 8,
        Op::LessEqual => 9,
        Op::GreaterEqual => 10,
        Op::GreaterThan => 11,
        Op::LogicalEqual => 12,
        Op::LogicalNotEqual => 13,
        Op::LogicalOr => 14,
        Op::LogicalAnd => 15,
        Op::Conditional | Op::ConditionalElse => 16,
        Op::NullCoalescing => 17,
        Op::Comma => 18,
        Op::Assignment => 19,
        Op::Return => 20,
        op if is_math_call(op) => 21,
        _ => return TokenClasses::NONE,
    };
    TokenClasses(1 << bit)
}

/// The classes that may stand as leaves at or below `node`: its own as a leaf, its note otherwise.
pub(super) fn classes_at(node: &Node) -> TokenClasses {
    if node.is_leaf() {
        class(node.op)
    } else {
        node.below
    }
}

/// The classes of the leaves of `list`, and of what may stand as a leaf below its other nodes.
pub(super) fn classes_in(list: &[Node]) -> (TokenClasses, TokenClasses) {
    let (mut leaves, mut below) = (TokenClasses::NONE, TokenClasses::NONE);
    for node in list {
        if node.is_leaf() {
            leaves |= class(node.op);
        } else {
            below |= node.below;
        }
    }
    (leaves, below)
}

/// A list with no leaf of the pass, walked in place in the pass's order without a level or a new
/// list.
struct Inert {
    list: Vec<Node>,
    /// The indices not looked at yet.
    unseen: Range<usize>,
    /// The node whose children are out.
    entered: usize,
    /// Whether a list below holds an unfolded operator token.
    unfolded: bool,
    /// The classes of the nodes looked at: their own as leaves, the ones below them otherwise.
    below: TokenClasses,
}

impl Inert {
    fn new(list: Vec<Node>) -> Self {
        Self {
            unseen: 0..list.len(),
            list,
            entered: 0,
            unfolded: false,
            below: TokenClasses::NONE,
        }
    }

    /// Takes out the children of the next node that has any of `classes` below it.
    fn enter_next<L: Level>(&mut self, classes: TokenClasses) -> Option<Vec<Node>> {
        loop {
            let index = if L::RIGHT_TO_LEFT {
                self.unseen.next_back()
            } else {
                self.unseen.next()
            }?;
            let node = self.list.get_mut(index)?;
            if node.is_leaf() {
                self.below |= class(node.op);
            } else if !node.below.intersects(classes) {
                self.below |= node.below;
            } else {
                self.entered = index;
                return Some(take(&mut node.children));
            }
        }
    }

    /// Puts back the children taken out last; `below` is what is below them, when it is known.
    fn resume(&mut self, children: Vec<Node>, unfolded: bool, below: Option<TokenClasses>) {
        if let Some(node) = self.list.get_mut(self.entered) {
            node.children = children;
            if let Some(below) = below {
                node.below = below;
            }
            self.below |= node.below;
        }
        self.unfolded |= unfolded;
    }
}

enum Frame<L> {
    Level(L),
    Inert(Inert),
}

/// A list a frame is done with, or one there was nothing to do in.
struct Finished {
    list: Vec<Node>,
    /// Whether it holds an operator token the pass did not fold.
    unfolded: bool,
    /// The classes that may stand as leaves in it, when they are known.
    below: Option<TokenClasses>,
}

/// What opening a list gives: a frame to work it off, or the list back at once.
enum Opened<L> {
    Frame(Frame<L>),
    Done(Finished),
}

/// Runs one pass, whose tokens are the ones of `classes` ([`class`]), over `list` and every
/// list below it on an explicit stack; `new` builds the level of a list holding such a token.
pub(super) fn run<L: Level>(
    cx: &mut Cx<'_>,
    list: &mut Vec<Node>,
    new: impl Fn(Vec<Node>) -> L,
    classes: TokenClasses,
) -> Pass {
    let open = |list: Vec<Node>| {
        let (leaves, below) = classes_in(&list);
        if !(leaves | below).intersects(classes) {
            Opened::Done(Finished {
                list,
                unfolded: false,
                below: Some(leaves | below),
            })
        } else if !leaves.intersects(classes) {
            Opened::Frame(Frame::Inert(Inert::new(list)))
        } else {
            Opened::Frame(Frame::Level(new(list)))
        }
    };
    let mut stack = Stack::new();
    let mut finished = match open(take(list)) {
        Opened::Frame(frame) => {
            stack.push(frame);
            None
        }
        Opened::Done(done) => Some(done),
    };
    loop {
        if let Some(done) = finished.take() {
            match stack.last_mut() {
                Some(Frame::Level(parent)) => parent.resume(done.list, done.unfolded),
                Some(Frame::Inert(parent)) => parent.resume(done.list, done.unfolded, done.below),
                None => {
                    *list = done.list;
                    return Ok(());
                }
            }
        }
        let Some(top) = stack.last_mut() else {
            return Ok(());
        };
        let children = match top {
            Frame::Level(level) => level.step(cx)?,
            Frame::Inert(walk) => walk.enter_next::<L>(classes),
        };
        if let Some(children) = children {
            match open(children) {
                Opened::Frame(frame) => stack.push(frame),
                Opened::Done(done) => finished = Some(done),
            }
            continue;
        }
        finished = match stack.pop() {
            Some(Frame::Level(level)) => {
                let (list, unfolded) = level.finish(cx)?;
                Some(Finished {
                    list,
                    unfolded,
                    below: None,
                })
            }
            Some(Frame::Inert(walk)) => Some(Finished {
                list: walk.list,
                unfolded: walk.unfolded,
                below: Some(walk.below),
            }),
            None => None,
        };
    }
}

/// How many frames a [`Stack`] keeps inline.
const INLINE: usize = 4;

/// The stack of [`run`]: a pass over lists nested no deeper than [`INLINE`] allocates nothing.
struct Stack<T> {
    inline: [Option<T>; INLINE],
    len: usize,
    spill: Vec<T>,
}

impl<T> Stack<T> {
    fn new() -> Self {
        Self {
            inline: std::array::from_fn(|_| None),
            len: 0,
            spill: Vec::new(),
        }
    }

    fn push(&mut self, frame: T) {
        if self.len < INLINE {
            self.inline[self.len] = Some(frame);
        } else {
            self.spill.push(frame);
        }
        self.len += 1;
    }

    fn pop(&mut self) -> Option<T> {
        self.len = self.len.checked_sub(1)?;
        if self.len < INLINE {
            self.inline[self.len].take()
        } else {
            self.spill.pop()
        }
    }

    fn last_mut(&mut self) -> Option<&mut T> {
        let top = self.len.checked_sub(1)?;
        if top < INLINE {
            self.inline[top].as_mut()
        } else {
            self.spill.last_mut()
        }
    }
}

/// A list a level rebuilds in place, left to right: `list[..write]` is the output,
/// `list[read..]` the unread input, and the slots between hold placeholders.
pub(super) struct Compact {
    list: Vec<Node>,
    read: usize,
    write: usize,
    /// The index in the output of the node whose children are out.
    entered: Option<usize>,
}

/// What [`Compact::next_token`] stops at.
pub(super) enum Found {
    /// A leaf the level folds, taken out of the input.
    Token(Node),
    /// The children of a node moved to the output, to be given back with [`Compact::resume`].
    Children(Vec<Node>),
}

impl Compact {
    pub(super) fn new(list: Vec<Node>) -> Self {
        Self {
            list,
            read: 0,
            write: 0,
            entered: None,
        }
    }

    /// Takes the next node of the input.
    pub(super) fn next(&mut self) -> Option<Node> {
        let slot = self.list.get_mut(self.read)?;
        self.read += 1;
        Some(std::mem::replace(slot, Node::placeholder()))
    }

    /// The next node of the input, left in place.
    pub(super) fn peek(&self) -> Option<&Node> {
        self.list.get(self.read)
    }

    /// Takes the next node of the input if `accept` accepts it.
    pub(super) fn next_if(&mut self, accept: impl FnOnce(&Node) -> bool) -> Option<Node> {
        if self.peek().is_some_and(accept) {
            self.next()
        } else {
            None
        }
    }

    /// Moves the input to the output as it is up to the next node with children, whose children
    /// it hands out, or the next leaf `is_token` accepts, which it takes.
    pub(super) fn next_token(&mut self, is_token: impl Fn(&Node) -> bool) -> Option<Found> {
        loop {
            let next = self.peek()?;
            if !next.is_leaf() {
                let at = self.keep();
                self.entered = Some(at);
                let children = take(&mut self.list.get_mut(at)?.children);
                return Some(Found::Children(children));
            }
            if is_token(next) {
                return self.next().map(Found::Token);
            }
            self.keep();
        }
    }

    /// Appends `node` to the output; returns its index there.
    pub(super) fn push(&mut self, node: Node) -> usize {
        let at = self.write;
        // `write < read`: every node pushed was taken from the input first.
        if let Some(slot) = self.list.get_mut(at) {
            *slot = node;
        }
        self.write += 1;
        at
    }

    /// Appends `node` to the output and hands out its children, to be given back with
    /// [`resume`](Compact::resume).
    pub(super) fn push_entered(&mut self, mut node: Node) -> Vec<Node> {
        let children = take(&mut node.children);
        self.entered = Some(self.push(node));
        children
    }

    /// Moves the next node of the input to the output as it is; returns its index there.
    pub(super) fn keep(&mut self) -> usize {
        let at = self.write;
        if at != self.read {
            // The slot at `write` is spent: the placeholder moves to the input's side.
            self.list.swap(at, self.read);
        }
        self.read += 1;
        self.write += 1;
        at
    }

    pub(super) fn len(&self) -> usize {
        self.write
    }

    /// Takes the last node of the output.
    pub(super) fn pop(&mut self) -> Option<Node> {
        self.write = self.write.checked_sub(1)?;
        self.list
            .get_mut(self.write)
            .map(|slot| std::mem::replace(slot, Node::placeholder()))
    }

    /// Gives the node whose children were handed out last its children back; returns its index in
    /// the output.
    pub(super) fn resume(&mut self, children: Vec<Node>) -> Option<usize> {
        let at = self.entered.take()?;
        self.list.get_mut(at)?.children = children;
        Some(at)
    }

    pub(super) fn finish(mut self) -> Vec<Node> {
        self.list.truncate(self.write);
        self.list
    }
}

/// Hands out the children of `node` and keeps the node in `hole` until they come back.
pub(super) fn enter(hole: &mut Option<Node>, mut node: Node) -> Vec<Node> {
    let children = take(&mut node.children);
    *hole = Some(node);
    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::parse::sections::sections;

    use crate::compile::{lex, parse::test_support::*};

    /// A level that records the order in which `run` drives it.
    struct Recorder<'l> {
        log: &'l std::cell::RefCell<Vec<String>>,
        id: usize,
        rest: std::vec::IntoIter<Node>,
        out: Vec<Node>,
        hole: Option<Node>,
    }

    impl Level for Recorder<'_> {
        fn step(&mut self, _: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
            for node in self.rest.by_ref() {
                if node.is(Op::Break) {
                    self.log.borrow_mut().push(format!("fail {}", self.id));
                    return Err(Failed);
                }
                if !node.is_leaf() {
                    self.log
                        .borrow_mut()
                        .push(format!("{} enters {}", self.id, label(&node)));
                    return Ok(Some(enter(&mut self.hole, node)));
                }
                self.out.push(node);
            }
            Ok(None)
        }

        fn resume(&mut self, children: Vec<Node>, unfolded: bool) {
            self.log
                .borrow_mut()
                .push(format!("{} resumes (unfolded {unfolded})", self.id));
            if let Some(mut node) = self.hole.take() {
                node.children = children;
                self.out.push(node);
            }
        }

        fn finish(self, _: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
            self.log.borrow_mut().push(format!("{} finishes", self.id));
            Ok((self.out, false))
        }
    }

    /// Drives a [`Recorder`] pass of the `,` class over the sections of `src`, every note unknown.
    fn drive(src: &str) -> (Result<(), ()>, Vec<String>, String) {
        drive_with(src, class(Op::Comma), true)
    }

    /// Drives a [`Recorder`] pass of `classes` over the sections of `src`; with `unknown`, every
    /// note is first set to all classes.
    fn drive_with(
        src: &str,
        classes: TokenClasses,
        unknown: bool,
    ) -> (Result<(), ()>, Vec<String>, String) {
        let opts = options(13);
        let mut cx = Cx::for_test(src, &opts);
        let mut list = lex::scan(&mut cx, src.as_bytes()).unwrap().nodes;
        assert!(sections(&mut cx, &mut list).is_ok());
        if unknown {
            forget_below(&mut list);
        }
        let log = std::cell::RefCell::new(Vec::new());
        let next = std::cell::Cell::new(0);
        let result = run(
            &mut cx,
            &mut list,
            |children| {
                next.set(next.get() + 1);
                Recorder {
                    log: &log,
                    id: next.get(),
                    rest: children.into_iter(),
                    out: Vec::new(),
                    hole: None,
                }
            },
            classes,
        );
        (result.map_err(|_| ()), log.into_inner(), show_list(&list))
    }

    #[test]
    fn the_driver_visits_child_lists_depth_first_in_recursion_order() {
        let (result, log, list) = drive("(1, (2,) 3), (4,)");
        assert!(result.is_ok());
        assert_eq!(
            log,
            [
                "1 enters LeftParenthesis",
                "2 enters LeftParenthesis",
                "3 finishes",
                "2 resumes (unfolded false)",
                "2 finishes",
                "1 resumes (unfolded false)",
                "1 enters LeftParenthesis",
                "4 finishes",
                "1 resumes (unfolded false)",
                "1 finishes",
            ]
        );
        assert_eq!(
            list,
            "(LeftParenthesis 1 Comma (LeftParenthesis 2 Comma) 3) | Comma | (LeftParenthesis 4 Comma)"
        );
    }

    #[test]
    fn the_driver_stops_at_the_first_failure_without_finishing_any_level() {
        let (result, log, _) = drive("(1, (break,) 3), (4,)");
        assert!(result.is_err());
        assert_eq!(
            log,
            [
                "1 enters LeftParenthesis",
                "2 enters LeftParenthesis",
                "fail 3"
            ]
        );
    }

    #[test]
    fn the_driver_replaces_the_list_with_the_result() {
        let (result, log, list) = drive("1, 2 3");
        assert!(result.is_ok());
        assert_eq!(log, ["1 finishes"]);
        assert_eq!(list, "1 | Comma | 2 | 3");
    }

    /// Every node of the tree says that anything may be below it.
    fn forget_below(list: &mut [Node]) {
        for node in list {
            node.below = TokenClasses::ALL;
            forget_below(&mut node.children);
        }
    }

    #[test]
    fn a_list_without_a_token_of_the_pass_is_walked_without_a_level_and_its_children_still_are() {
        let (result, log, list) = drive_with("(1 (2) , 3) (4 (,))", class(Op::Comma), false);
        assert!(result.is_ok());
        assert_eq!(
            log,
            [
                "1 enters LeftParenthesis",
                "1 resumes (unfolded false)",
                "1 finishes",
                "2 finishes"
            ]
        );
        assert_eq!(
            list,
            "(LeftParenthesis 1 (LeftParenthesis 2) Comma 3) | (LeftParenthesis 4 (LeftParenthesis Comma))"
        );
    }

    #[test]
    fn a_failure_below_a_list_walked_in_place_fails_the_pass() {
        let (result, log, _) = drive_with("(1 (2 (break ,)))", class(Op::Comma), false);
        assert!(result.is_err());
        assert_eq!(log, ["fail 1"]);
    }

    #[test]
    fn a_tree_with_nothing_of_the_pass_is_given_back_as_it_was() {
        let (result, log, list) = drive_with("1 (2 (4) {5}) [6]", class(Op::Comma), false);
        assert!(result.is_ok());
        assert!(log.is_empty());
        assert_eq!(
            list,
            "1 | (LeftParenthesis 2 (LeftParenthesis 4) (LeftBrace 5)) | (LeftBracket 6)"
        );
    }

    /// A pass that walks its lists from the right; only its walk in place is used.
    struct Leftward;

    impl Level for Leftward {
        const RIGHT_TO_LEFT: bool = true;

        fn step(&mut self, _: &mut Cx<'_>) -> Result<Option<Vec<Node>>, Failed> {
            Ok(None)
        }

        fn resume(&mut self, _: Vec<Node>, _: bool) {}

        fn finish(self, _: &mut Cx<'_>) -> Result<(Vec<Node>, bool), Failed> {
            Ok((Vec::new(), false))
        }
    }

    /// The first child of each node a walk over `list` enters for `classes`, and the list it gives
    /// back.
    fn walked<L: Level>(
        list: Vec<Node>,
        classes: TokenClasses,
    ) -> (Vec<String>, String, TokenClasses) {
        let mut walk = Inert::new(list);
        let mut firsts = Vec::new();
        while let Some(children) = walk.enter_next::<L>(classes) {
            firsts.push(label(&children[0]));
            walk.resume(children, false, None);
        }
        (firsts, show_list(&walk.list), walk.below)
    }

    #[test]
    fn a_walk_in_place_enters_the_nodes_with_children_in_the_pass_order() {
        let mut list = grouped("sections", "1 (2) 3 {4} [5] 6");
        forget_below(&mut list);
        let shown = "1 | (LeftParenthesis 2) | 3 | (LeftBrace 4) | (LeftBracket 5) | 6";
        let mut copy = grouped("sections", "1 (2) 3 {4} [5] 6");
        forget_below(&mut copy);
        let calls = class(Op::QueryFunction);
        assert_eq!(
            walked::<Recorder<'_>>(list, calls),
            (
                vec!["2".to_owned(), "4".to_owned(), "5".to_owned()],
                shown.to_owned(),
                TokenClasses::ALL
            )
        );
        assert_eq!(
            walked::<Leftward>(copy, calls),
            (
                vec!["5".to_owned(), "4".to_owned(), "2".to_owned()],
                shown.to_owned(),
                TokenClasses::ALL
            )
        );
        assert_eq!(
            walked::<Leftward>(Vec::new(), calls),
            (Vec::new(), String::new(), TokenClasses::NONE)
        );
    }

    #[test]
    fn a_walk_in_place_skips_the_nodes_with_nothing_of_the_pass_below() {
        let list = grouped("sections", "(1 * 2) (3 / 4) (- 5) 6 + 7");
        let classes = class(Op::Div) | class(Op::Negate);
        let (firsts, _, below) = walked::<Recorder<'_>>(list, class(Op::Div));
        assert_eq!(firsts, ["3"]);
        assert_eq!(
            below,
            class(Op::Mul) | class(Op::Div) | class(Op::Negate) | class(Op::Add),
            "the notes of the nodes passed and entered, and the leaves"
        );
        let (firsts, ..) =
            walked::<Recorder<'_>>(grouped("sections", "(1 * 2) (3 / 4) (- 5) 6 + 7"), classes);
        assert_eq!(firsts, ["3", "Negate"]);
    }

    #[test]
    fn a_walk_in_place_notes_what_is_below_a_node_it_entered() {
        let mut list = grouped("sections", "(1 * 2) 3");
        forget_below(&mut list);
        let mut walk = Inert::new(list);
        let children = walk
            .enter_next::<Recorder<'_>>(TokenClasses::ALL)
            .expect("a section");
        walk.resume(children, false, Some(class(Op::Mul)));
        assert!(walk.enter_next::<Recorder<'_>>(TokenClasses::ALL).is_none());
        assert_eq!(walk.list[0].below, class(Op::Mul));
        assert_eq!(walk.below, class(Op::Mul));
        // A list a level gave back leaves the note as it was.
        let mut list = grouped("sections", "(1 * 2) 3");
        forget_below(&mut list);
        let mut walk = Inert::new(list);
        let children = walk
            .enter_next::<Recorder<'_>>(TokenClasses::ALL)
            .expect("a section");
        walk.resume(children, false, None);
        assert_eq!(walk.list[0].below, TokenClasses::ALL);
    }

    #[test]
    fn a_walk_in_place_reports_an_unfolded_token_from_any_list_below() {
        let mut list = grouped("sections", "(1) (2) (3)");
        forget_below(&mut list);
        let mut walk = Inert::new(list);
        for unfolded in [false, true, false] {
            let children = walk
                .enter_next::<Recorder<'_>>(class(Op::QueryFunction))
                .expect("a section");
            walk.resume(children, unfolded, None);
        }
        assert!(
            walk.enter_next::<Recorder<'_>>(class(Op::QueryFunction))
                .is_none()
        );
        assert!(walk.unfolded);
    }

    #[test]
    fn a_compacted_list_reads_its_input_in_order_and_keeps_what_is_pushed() {
        let mut list = Compact::new(grouped("sections", "1 2 3 4"));
        assert_eq!(list.peek().map(label), Some("1".to_owned()));
        let one = list.next().expect("1");
        assert_eq!(list.push(one), 0);
        assert!(
            list.next_if(|node| node.float() == 5.0).is_none(),
            "a refused node stays"
        );
        let two = list.next_if(|node| node.float() == 2.0).expect("2");
        assert_eq!(list.len(), 1);
        drop(two);
        let three = list.next().expect("3");
        assert_eq!(list.push(three), 1);
        let popped = list.pop().expect("3");
        assert_eq!(label(&popped), "3");
        assert_eq!(list.push(popped), 1);
        assert!(list.next().is_some());
        assert!(list.next().is_none());
        assert!(list.peek().is_none());
        assert_eq!(show_list(&list.finish()), "1 | 3");
    }

    #[test]
    fn a_compacted_list_gives_an_entered_node_its_children_back_in_place() {
        let mut list = Compact::new(grouped("sections", "(1 2) 3"));
        let section = list.next().expect("a section");
        let children = list.push_entered(section);
        assert_eq!(show_list(&children), "1 | 2");
        let three = list.next().expect("3");
        list.push(three);
        assert_eq!(list.resume(children), Some(0));
        assert_eq!(show_list(&list.finish()), "(LeftParenthesis 1 2) | 3");
    }

    #[test]
    fn keeping_a_node_moves_it_across_the_spent_slots() {
        let mut list = Compact::new(grouped("sections", "1 2 3 (4)"));
        assert_eq!(
            list.keep(),
            0,
            "nothing spent yet: the node stays where it is"
        );
        drop(list.next());
        assert_eq!(
            list.keep(),
            1,
            "after one spent slot the next kept node moves back by one"
        );
        let Some(Found::Children(children)) = list.next_token(|_| false) else {
            panic!("the section is entered");
        };
        assert_eq!(show_list(&children), "4");
        assert_eq!(list.resume(children), Some(2));
        assert!(list.peek().is_none());
        assert_eq!(show_list(&list.finish()), "1 | 3 | (LeftParenthesis 4)");
    }

    #[test]
    fn the_stack_keeps_its_first_frames_inline_and_spills_the_rest() {
        let mut stack = Stack::new();
        assert!(stack.last_mut().is_none());
        assert!(stack.pop().is_none());
        for frame in 0..10 {
            stack.push(frame);
            assert_eq!(stack.last_mut().copied(), Some(frame));
        }
        assert_eq!(stack.spill.len(), 10 - INLINE);
        if let Some(top) = stack.last_mut() {
            *top = 90;
        }
        assert_eq!(stack.pop(), Some(90));
        for frame in (0..9).rev() {
            assert_eq!(stack.pop(), Some(frame));
        }
        assert!(stack.pop().is_none());
        assert!(stack.spill.is_empty());
    }

    #[test]
    fn popping_an_empty_compacted_list_gives_nothing() {
        let mut list = Compact::new(grouped("sections", "1"));
        assert!(list.pop().is_none());
        assert_eq!(list.len(), 0);
        assert!(list.finish().is_empty());
    }

    #[test]
    fn each_pass_has_its_own_class_and_the_rest_none() {
        let groups: [&[Op]; 21] = [
            &[Op::QueryFunction, Op::Loop, Op::ForEach],
            &[Op::Semicolon],
            &[Op::ArrayVariable],
            &[Op::Pointer],
            &[Op::LogicalNot],
            &[Op::Div],
            &[Op::Mul],
            &[Op::Add],
            &[Op::LessThan],
            &[Op::LessEqual],
            &[Op::GreaterEqual],
            &[Op::GreaterThan],
            &[Op::LogicalEqual],
            &[Op::LogicalNotEqual],
            &[Op::LogicalOr],
            &[Op::LogicalAnd],
            &[Op::Conditional, Op::ConditionalElse],
            &[Op::NullCoalescing],
            &[Op::Comma],
            &[Op::Assignment],
            &[Op::Return],
        ];
        let mut seen = TokenClasses::NONE;
        for group in groups {
            let first = class(group[0]);
            assert_eq!(first.0.count_ones(), 1, "{group:?}");
            assert!(!seen.intersects(first), "{group:?} shares a class");
            seen |= first;
            assert!(group.iter().all(|&op| class(op) == first), "{group:?}");
        }
        let math = class(Op::Sin);
        assert_eq!(math.0.count_ones(), 1);
        assert!(!seen.intersects(math));
        for &op in Op::all() {
            if is_math_call(op) {
                assert_eq!(class(op), math, "{op:?}");
            } else if op != Op::Negate && !groups.iter().any(|group| group.contains(&op)) {
                assert_eq!(class(op), TokenClasses::NONE, "{op:?}");
            }
        }
        assert_eq!(class(Op::Pi), TokenClasses::NONE);
    }

    #[test]
    fn a_minus_counts_as_a_not_and_as_a_plus() {
        assert_eq!(class(Op::Negate), class(Op::LogicalNot) | class(Op::Add));
    }

    #[test]
    fn classes_in_a_list_are_the_leaves_and_the_notes_of_the_other_nodes() {
        let list = grouped("sections", "1 + (2 * 3) - [4 / 5]");
        assert_eq!(
            classes_in(&list),
            (
                class(Op::Add) | class(Op::Negate),
                class(Op::Mul) | class(Op::Div)
            )
        );
        assert_eq!(classes_in(&[]), (TokenClasses::NONE, TokenClasses::NONE));
    }
}
