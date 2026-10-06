//! The merge of the terms of an addition.

use super::lift_child;
use crate::compile::ast::{Name, Node, Payload, TERM_TEXT_LIMIT};
use crate::numeric::arith;
use crate::ops::ExpressionOp as Op;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::{BuildHasherDefault, Hasher};

/// Replaces each `Add` child by its terms, its scale distributed over them and its offset folded
/// into this node's.
///
/// A leading nested sum's list becomes this node's, so a left-nested chain `a + b + c + …` does not
/// copy its terms once per `+`.
pub(super) fn flatten_add(node: &mut Node) {
    if !node.children.iter().any(|c| c.is(Op::Add)) {
        return;
    }
    let old = std::mem::take(&mut node.children);
    for mut child in old {
        if !child.is(Op::Add) {
            node.children.push(child);
            continue;
        }
        // A product with 1 is the other factor bit for bit: no scale or offset is a signalling NaN.
        if child.post.scale != 1.0 {
            for term in &mut child.children {
                term.post.scale = arith::mul(term.post.scale, child.post.scale);
                term.post.offset = arith::mul(term.post.offset, child.post.scale);
            }
        }
        node.post.offset = arith::mul_add(node.post.scale, child.post.offset, node.post.offset);
        if node.children.is_empty() {
            node.children = std::mem::take(&mut child.children);
        } else {
            node.children.append(&mut child.children);
        }
    }
}

/// Which terms of a flattened sum are new to the checks and the merge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NewTerms {
    All,
    /// Only the last: the sum extends an optimised sum by one term.
    Last,
}

/// Which terms of the unflattened `Add` `node` are new: only the last when it is an optimised sum
/// followed by one non-sum term (every `+` but the innermost of a left-nested chain).
///
/// An optimised sum has two or more terms, none a `Float` or an `Add` and no two the same entity
/// variable; [`optimize_add`] relies on that for [`NewTerms::Last`], and looks at the new last term
/// only.
pub(super) fn new_terms(node: &Node) -> NewTerms {
    if matches!(node.children.as_slice(), [sum, term] if sum.is(Op::Add) && !term.is(Op::Add)) {
        NewTerms::Last
    } else {
        NewTerms::All
    }
}

/// The `Add` case, after flattening: only the `new` terms can be the constant term or merge with an
/// earlier term.
pub(super) fn optimize_add(node: &mut Node, new: NewTerms) {
    if new == NewTerms::All
        && node.children.iter().all(|c| c.is(Op::Float))
        && let [a, b, ..] = node.children.as_slice()
    {
        // Only the first two children count; the parser never builds more. The left operand's NaN
        // wins on x86-64.
        let sum = arith::add(a.float(), b.float());
        node.set_float(sum);
        return;
    }
    if let Some(constant) = take_constant(node, new) {
        if let [term] = node.children.as_mut_slice() {
            // `x + c`: folded into `x`'s post-op.
            term.post.offset = arith::add(term.post.offset, constant);
            lift_only_term(node);
            return;
        }
        node.post.offset = arith::mul_add(node.post.scale, constant, node.post.offset);
    } else if terms_print_alike(node) {
        merge_alike_terms(node);
        return;
    }

    match new {
        NewTerms::All => merge_entity_variable_terms(node),
        NewTerms::Last => merge_last_term(&mut node.children),
    }
    match node.children.len() {
        0 => {
            // Every term cancelled: the node is the constant it had accumulated, post-op kept.
            let offset = node.post.offset;
            node.set_float(offset);
        }
        1 => lift_only_term(node),
        _ => {}
    }
}

/// Removes the constant term, the *last* float child (after flattening there is at most one), and
/// returns its value with its post-op applied.
fn take_constant(node: &mut Node, new: NewTerms) -> Option<f32> {
    let constant =
        |child: &Node| arith::mul_add(child.post.scale, child.float(), child.post.offset);
    match new {
        NewTerms::Last => {
            debug_assert!(
                node.children
                    .split_last()
                    .is_some_and(|(_, earlier)| earlier.iter().all(|t| !t.is(Op::Float)))
            );
            node.children
                .pop_if(|child| child.is(Op::Float))
                .map(|child| constant(&child))
        }
        NewTerms::All => {
            let mut value = None;
            node.children.retain(|child| {
                let float = child.is(Op::Float);
                if float {
                    value = Some(constant(child));
                }
                !float
            });
            value
        }
    }
}

/// Replaces the sum by its only term, the sum's post-op applied after the term's.
fn lift_only_term(node: &mut Node) {
    let own = node.post;
    let term = lift_child(node, 0);
    term.post.scale = arith::mul(term.post.scale, own.scale);
    term.post.offset = arith::mul_add(term.post.offset, own.scale, own.offset);
}

/// `x + x + …` of identical terms: one term with the post-ops summed in two interleaved chains
/// (even and odd positions), then the odd one out.
fn merge_alike_terms(node: &mut Node) {
    let mut even = (0.0f32, 0.0f32);
    let mut odd = (0.0f32, 0.0f32);
    let (pairs, rest) = node.children.as_chunks::<2>();
    for [a, b] in pairs {
        even = (
            arith::add(a.post.scale, even.0),
            arith::add(a.post.offset, even.1),
        );
        odd = (
            arith::add(b.post.scale, odd.0),
            arith::add(b.post.offset, odd.1),
        );
    }
    let mut sum = (arith::add(odd.0, even.0), arith::add(odd.1, even.1));
    if let [last] = rest {
        sum = (
            arith::add(last.post.scale, sum.0),
            arith::add(last.post.offset, sum.1),
        );
    }
    if sum.0 == 0.0 {
        let offset = node.post.offset;
        node.set_float(offset);
    } else {
        let own = node.post;
        let term = lift_child(node, 0);
        term.post.scale = arith::mul(own.scale, sum.0);
        term.post.offset = arith::mul_add(own.scale, sum.1, own.offset);
    }
}

/// Adds `term`'s post-op to `merged`'s; whether the merged scale cancelled to exactly zero.
fn merge_into(merged: &mut Node, term: &Node) -> bool {
    merged.post.scale = arith::add(merged.post.scale, term.post.scale);
    merged.post.offset = arith::add(merged.post.offset, term.post.offset);
    merged.post.scale == 0.0
}

/// Terms that are the same entity variable merge into the first of them, their post-ops added in
/// source order. A merged term whose scale cancels to exactly zero leaves the sum with its offset
/// (`(v.x + 1) - v.x` folds to 0); a later term of that variable starts a new merged term in its
/// own position.
///
/// Same result as comparing every term with every later one, in linear time.
fn merge_entity_variable_terms(node: &mut Node) {
    let terms = &mut node.children;
    let Some(repeated) = RepeatedNames::of(terms) else {
        return;
    };
    let mut leaving: Vec<usize> = Vec::new();
    // The live merged term of each variable, keyed by the FNV-1 hash of its name; a name whose hash
    // another name held when its term arrived goes to `collided`. A live term is in exactly one map
    // until it cancels, and a collided name keeps its entry when the hash holder cancels, so
    // `collided` is checked whenever `live` does not hold the name itself.
    let mut live: HashMap<u64, usize, BuildHasherDefault<HashKey>> = HashMap::default();
    let mut collided: HashMap<Box<str>, usize> = HashMap::new();
    for index in 0..terms.len() {
        let Some(name) = entity_variable_name(&terms[index]) else {
            continue;
        };
        let key = name.hash().as_u64();
        if !repeated.may_repeat(key) {
            continue;
        }
        let existing = match live.get(&key) {
            Some(&first) if terms[first].value == terms[index].value => Some(first),
            _ => collided.get(name.as_str()).copied(),
        };
        let Some(first) = existing else {
            match live.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(index);
                }
                Entry::Occupied(_) => {
                    collided.insert(name.as_str().into(), index);
                }
            }
            continue;
        };
        // `first < index`: merge the later term into the earlier one.
        let (head, tail) = terms.split_at_mut(index);
        let merged = &mut head[first];
        leaving.push(index);
        if merge_into(merged, &tail[0]) {
            leaving.push(first);
            if live.get(&key) == Some(&first) {
                live.remove(&key);
            } else if let Payload::Entity(name) = &merged.value {
                collided.remove(name.as_str());
            }
        }
    }
    if leaving.is_empty() {
        return;
    }
    leaving.sort_unstable();
    let mut leaving = leaving.into_iter().peekable();
    for (index, term) in std::mem::take(terms).into_iter().enumerate() {
        if leaving.next_if(|&at| at == index).is_none() {
            terms.push(term);
        }
    }
}

/// [`merge_entity_variable_terms`] for terms whose entity variables, all but the last term's, are
/// distinct: the last term merges into the earlier one of the same variable, if there is one.
fn merge_last_term(terms: &mut Vec<Node>) {
    let Some((term, earlier)) = terms.split_last_mut() else {
        return;
    };
    let Some(name) = entity_variable_name(term) else {
        return;
    };
    let Some(first) = earlier.iter().position(|t| {
        entity_variable_name(t)
            .is_some_and(|n| n.hash() == name.hash() && n.as_str() == name.as_str())
    }) else {
        return;
    };
    let cancelled = merge_into(&mut earlier[first], term);
    terms.pop();
    if cancelled {
        terms.remove(first);
    }
}

fn entity_variable_name(term: &Node) -> Option<&Name> {
    match (term.op, &term.value) {
        (Op::EntityVariable, Payload::Entity(name)) => Some(name),
        _ => None,
    }
}

/// A filter with no false negatives for the entity-variable names that occur more than once among
/// the terms of a `+`: a name that occurs once costs two bit tests instead of a table lookup.
struct RepeatedNames {
    mask: u64,
    twice: Vec<u64>,
}

impl RepeatedNames {
    /// `None` when no name can occur twice.
    fn of(terms: &[Node]) -> Option<Self> {
        let bits = (terms.len() * 16).next_power_of_two().max(64);
        let mask = bits as u64 - 1;
        let mut once = vec![0u64; bits / 64];
        let mut twice = vec![0u64; bits / 64];
        let mut any = false;
        for name in terms.iter().filter_map(entity_variable_name) {
            let bit = mix(name.hash().as_u64()) & mask;
            let (word, flag) = ((bit / 64) as usize, 1u64 << (bit % 64));
            if once[word] & flag != 0 {
                twice[word] |= flag;
                any = true;
            }
            once[word] |= flag;
        }
        any.then_some(Self { mask, twice })
    }

    fn may_repeat(&self, hash: u64) -> bool {
        let bit = mix(hash) & self.mask;
        self.twice[(bit / 64) as usize] & (1u64 << (bit % 64)) != 0
    }
}

/// splitmix64's finaliser: FNV-1 of names that differ only in their last bytes differs mostly in
/// its low bits, which a table or a filter would use as they are.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A hasher for keys that already are hashes (mixed, see [`mix`]).
#[derive(Default)]
struct HashKey(u64);

impl Hasher for HashKey {
    fn finish(&self) -> u64 {
        mix(self.0)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 << 8) ^ u64::from(byte);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

/// Whether every child has the term text of the first, and the first has no side effects (random
/// function, query, assignment).
fn terms_print_alike(node: &Node) -> bool {
    let Some((first, rest)) = node.children.split_first() else {
        return true;
    };
    if rest.is_empty() {
        return true;
    }
    // Texts that begin differently settle it without building either: the usual sum of different
    // variables or calls.
    if first
        .term_text_lead()
        .differs_from(rest[0].term_text_lead())
    {
        return false;
    }
    // An empty term text never makes terms equal.
    let Some(len) = first.term_text_len().filter(|&len| len != 0) else {
        return false;
    };
    if len > TERM_TEXT_LIMIT {
        // A text this long (a member chain doubles it with every member) is not built; terms of
        // the same shape are the ones with the same text.
        return rest
            .iter()
            .all(|child| child.term_text_len() == Some(len) && first.same_term_text_shape(child));
    }
    // The string of the first term is built only once a term of its length needs it.
    let mut reference = None;
    rest.iter().all(|child| {
        child.term_text_len() == Some(len)
            && child.term_text() == *reference.get_or_insert_with(|| first.term_text())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ast::Span;
    use crate::compile::sema::test_support::*;
    use crate::numeric::{
        arch::{arm64, x86_64},
        test_support::per_arch,
    };

    fn add(children: Vec<Node>) -> Node {
        parent(Op::Add, children)
    }

    /// `v.x.a.a…` with `depth` members: its term text doubles with every member.
    fn member_chain(depth: usize, last: &str) -> Node {
        let mut node = entity("x");
        for index in 0..depth {
            node = member(if index + 1 == depth { last } else { "a" }, node);
        }
        node
    }

    fn terms(names: &[&str]) -> Node {
        add(names.iter().map(|name| entity(name)).collect())
    }

    /// Two names with one hash.
    fn colliding(first: &str, second: &str) -> (Node, Node) {
        let node = |name: &str| {
            Node::token(
                Op::EntityVariable,
                Payload::Entity(Name::with_hash(format!("variable.{name}"), 0x00C0_FFEE)),
                Span::new(0, 1),
            )
        };
        (node(first), node(second))
    }

    #[test]
    fn flatten_add_leaves_a_sum_without_nested_sums_alone() {
        let mut node = add(vec![entity("a"), entity("b"), float(1.0)]);
        flatten_add(&mut node);
        assert_eq!(node.tree_notation(9), "(Add v.a v.b 1)");
        assert_post_bits(&node, 1.0, 0.0);
    }

    #[test]
    fn flatten_add_replaces_a_nested_sum_by_its_terms_in_place() {
        let mut node = add(vec![
            entity("a"),
            add(vec![entity("b"), entity("c")]),
            entity("d"),
            add(vec![entity("e"), entity("f")]),
        ]);
        flatten_add(&mut node);
        assert_eq!(node.tree_notation(9), "(Add v.a v.b v.c v.d v.e v.f)");
    }

    #[test]
    fn flatten_add_distributes_the_nested_scale_and_adds_the_nested_offset() {
        let inner = with_post(
            add(vec![with_post(entity("b"), 1.0, 5.0), entity("c")]),
            2.0,
            1.0,
        );
        let mut node = with_post(add(vec![inner, entity("a")]), 3.0, 0.5);
        flatten_add(&mut node);
        assert_eq!(node.children.len(), 3);
        // Each term: scale × 2, offset × 2.
        assert_post_bits(&node.children[0], 2.0, 10.0);
        assert_post_bits(&node.children[1], 2.0, 0.0);
        assert_post_bits(&node.children[2], 1.0, 0.0);
        // The outer offset gains the nested offset, scaled by the outer scale: 3·1 + 0.5.
        assert_post_bits(&node, 3.0, 3.5);
    }

    #[test]
    fn flatten_add_rounds_the_outer_offset_as_the_architecture_does() {
        let (scale, nested, offset) = (0.3f32, 0.1f32, 0.1f32);
        assert_ne!(
            x86_64::mul_add(scale, nested, offset).to_bits(),
            arm64::mul_add(scale, nested, offset).to_bits()
        );
        let mut node = with_post(
            add(vec![
                with_post(add(vec![entity("b"), entity("c")]), 1.0, nested),
                entity("a"),
            ]),
            scale,
            offset,
        );
        flatten_add(&mut node);
        assert_eq!(
            node.post.offset.to_bits(),
            arith::mul_add(scale, nested, offset).to_bits()
        );
    }

    #[test]
    fn sums_of_sums_flatten_through_the_optimiser() {
        assert_eq!(shown("(v.x+v.y)+v.z"), "(Add v.x v.y v.z)");
        assert_eq!(shown("v.x+(v.y+v.z)"), "(Add v.x v.y v.z)");
        assert_eq!(shown("v.x+(v.y+(v.z+1))"), "(Add v.x v.y [v.z*1+1])");
        assert_eq!(shown("v.x+v.y+1+(v.z+2)"), "[(Add v.x v.y [v.z*1+2])*1+1]");
    }

    #[test]
    fn a_scaled_sum_distributes_its_scale_over_the_terms_of_an_enclosing_sum() {
        assert_eq!(shown("2*(v.x+v.y)"), "[(Add v.x v.y)*2+0]");
        assert_eq!(shown("2*(v.x+v.y)+1"), "[(Add [v.x*2+0] [v.y*2+0])*1+1]");
        assert_eq!(shown("1+(v.y+v.z)*3"), "[(Add [v.y*3+0] [v.z*3+0])*1+1]");
        assert_eq!(shown("(v.x+v.y)*2*3"), "[(Add v.x v.y)*6+0]");
    }

    #[test]
    fn a_sum_of_constants_adds_the_second_and_the_first() {
        assert_float_bits(&folded(add(vec![float(1.5), float(2.0)])), 3.5);
        assert_float_bits(&folded(add(vec![float(0.1), float(0.2)])), 0.1f32 + 0.2f32);
        assert_eq!(shown("1+2+3+4"), "10");
    }

    #[test]
    fn a_sum_of_three_constants_reads_only_the_first_two() {
        // An all-constant sum adds its first two children; the parser never builds three.
        assert_float_bits(&folded(add(vec![float(1.0), float(2.0), float(4.0)])), 3.0);
    }

    #[test]
    fn a_constant_term_next_to_one_variable_folds_into_its_offset() {
        assert_eq!(shown("v.x+2"), "[v.x*1+2]");
        assert_eq!(shown("2+v.x"), "[v.x*1+2]");
        assert_eq!(shown("v.x*2+1"), "[v.x*2+1]");
        assert_eq!(shown("1+2+v.x+3"), "[v.x*1+6]");
        assert_eq!(shown("v.x+1+v.x+2"), "[v.x*2+3]");
    }

    #[test]
    fn the_constant_term_is_the_last_float_child_not_the_sum() {
        let root = folded(add(vec![entity("x"), float(1.0), float(2.0)]));
        assert!(root.is(Op::EntityVariable));
        assert_post_bits(&root, 1.0, 2.0);
    }

    #[test]
    fn the_constant_term_includes_its_own_post_op() {
        let root = folded(add(vec![entity("x"), with_post(float(3.0), 2.0, 1.0)]));
        assert_post_bits(&root, 1.0, 7.0);
    }

    #[test]
    fn a_constant_next_to_a_term_with_a_post_op_adds_to_its_offset() {
        let root = folded(add(vec![with_post(entity("x"), 2.0, 3.0), float(4.0)]));
        assert_post_bits(&root, 2.0, 7.0);
    }

    #[test]
    fn the_own_post_op_of_the_sum_scales_the_folded_term() {
        // x + c under (S, O): the term becomes (term.scale·S, (term.offset + c)·S + O).
        let root = folded(with_post(add(vec![entity("x"), float(3.0)]), 2.0, 1.0));
        assert_post_bits(&root, 2.0, 7.0);
    }

    #[test]
    fn the_term_offset_rounds_as_the_architecture_does() {
        let (constant, scale, offset) = (0.3f32, 0.1f32, 0.1f32);
        assert_ne!(
            x86_64::mul_add(constant, scale, offset).to_bits(),
            arm64::mul_add(constant, scale, offset).to_bits()
        );
        let root = folded(with_post(
            add(vec![entity("x"), float(constant)]),
            scale,
            offset,
        ));
        assert_post_bits(&root, scale, arith::mul_add(constant, scale, offset));
    }

    #[test]
    fn the_folded_term_chooses_its_nans_as_the_architecture_does() {
        let (q1, q2) = (f32::from_bits(0x7fc0_0001), f32::from_bits(0xffc0_0002));
        let root = folded(add(vec![with_post(entity("x"), q2, q1), float(q2)]));
        assert_post_bits(&root, q2, q1);
        let root = folded(add(vec![
            with_post(entity("x"), 1.0, f32::INFINITY),
            float(f32::NEG_INFINITY),
        ]));
        assert_post_bits(
            &root,
            1.0,
            f32::from_bits(per_arch(0xffc0_0000, 0x7fc0_0000)),
        );
    }

    #[test]
    fn a_constant_next_to_several_terms_goes_into_the_sums_own_offset() {
        assert_eq!(shown("v.x+v.y+1"), "[(Add v.x v.y)*1+1]");
        let root = folded(with_post(
            add(vec![entity("x"), entity("y"), float(3.0)]),
            2.0,
            1.0,
        ));
        assert_eq!(root.tree_notation(9), "[(Add v.x v.y)*2+7]");
    }

    #[test]
    fn a_zero_constant_leaves_no_post_op() {
        assert_eq!(shown("v.x+0"), "v.x");
        assert_eq!(shown("v.x+v.y+0"), "(Add v.x v.y)");
    }

    #[test]
    fn equal_terms_become_one_term_with_the_summed_scale() {
        assert_eq!(shown("v.x+v.x"), "[v.x*2+0]");
        assert_eq!(shown("v.x+v.x+v.x"), "[v.x*3+0]");
        assert_eq!(shown("v.x+v.x+v.x+v.x"), "[v.x*4+0]");
        assert_eq!(shown("t.x+t.x"), "[t.x*2+0]");
        assert_eq!(shown("c.a+c.a"), "[c.a*2+0]");
        assert_eq!(shown("math.sin(v.x)+math.sin(v.x)"), "[(Sin v.x)*2+0]");
        assert_eq!(shown("(v.x*2)+(v.x*2)"), "[v.x*4+0]");
        assert_eq!(shown("(v.x+1)+(v.x+1)"), "[v.x*2+2]");
    }

    #[test]
    fn equal_terms_sum_their_post_ops() {
        let root = folded(add(vec![
            with_post(entity("x"), 2.0, 1.0),
            with_post(entity("x"), 2.0, 1.0),
        ]));
        assert_post_bits(&root, 4.0, 2.0);
    }

    #[test]
    fn equal_terms_are_summed_in_two_interleaved_chains() {
        // Six terms of scale 0.3: ((0.3+0.3)+0.3) + ((0.3+0.3)+0.3) is 1.8000001, one ulp above the
        // left-to-right sum 1.8.
        let terms = (0..6).map(|_| with_post(entity("x"), 0.3, 0.0)).collect();
        let root = folded(add(terms));
        assert_post_bits(&root, 1.800_000_1, 0.0);
        // An odd count adds the odd one out last.
        let seven = (0..7).map(|_| with_post(entity("x"), 0.3, 0.0)).collect();
        assert_post_bits(&folded(add(seven)), 2.100_000_1, 0.0);
    }

    #[test]
    fn an_odd_count_of_equal_terms_adds_the_offset_of_the_odd_one_out() {
        assert_eq!(shown("(v.x+1)+(v.x+1)+(v.x+1)"), "[v.x*3+3]");
        assert_eq!(
            shown("(v.x+2)+(v.x+2)+(v.x+2)+(v.x+2)+(v.x+2)"),
            "[v.x*5+10]"
        );
        let odd = (0..3).map(|_| with_post(entity("x"), 2.0, 0.5)).collect();
        assert_post_bits(&folded(add(odd)), 6.0, 1.5);
    }

    #[test]
    fn equal_terms_that_print_alike_are_summed_with_their_own_scales() {
        // The term text prints a post-op with six decimals, so scales two ulps apart are "equal"
        // terms; the sum takes the scale of each term, not the first one twice.
        let (first, second) = (0.3f32, f32::from_bits(0.3f32.to_bits() + 2));
        assert_ne!(first.to_bits(), second.to_bits());
        assert_ne!((second + first).to_bits(), (first + first).to_bits());
        let root = folded(add(vec![
            with_post(entity("x"), first, 0.0),
            with_post(entity("x"), second, 0.0),
        ]));
        assert_post_bits(&root, second + first, 0.0);
        // Four terms: the second pair is read from its own positions.
        let (third, fourth) = (
            f32::from_bits(0.3f32.to_bits() + 4),
            f32::from_bits(0.3f32.to_bits() + 6),
        );
        let terms = [first, second, third, fourth]
            .into_iter()
            .map(|scale| with_post(entity("x"), scale, 0.0))
            .collect();
        let even = third + (first + 0.0);
        let odd = fourth + (second + 0.0);
        assert_post_bits(&folded(add(terms)), odd + even, 0.0);
    }

    #[test]
    fn equal_terms_under_a_post_op_scale_by_it_and_keep_its_offset() {
        let node = with_post(add(vec![entity("x"), entity("x")]), 2.0, 1.0);
        assert_post_bits(&folded(node), 4.0, 1.0);
    }

    #[test]
    fn equal_terms_whose_scale_sums_to_zero_become_the_sums_offset() {
        assert_float_bits(&tree("v.x*0+v.x*0"), 0.0);
        let node = with_post(
            add(vec![
                with_post(entity("x"), 0.0, 0.0),
                with_post(entity("x"), 0.0, 0.0),
            ]),
            1.0,
            5.0,
        );
        let root = folded(node);
        assert_float_bits(&root, 5.0);
        assert_post_bits(&root, 1.0, 5.0);
    }

    #[test]
    fn equal_terms_of_scale_zero_do_not_add_their_own_offsets() {
        // `v.x*0+3` twice: the scales sum to zero, so the node is the constant of its own offset;
        // the terms' offsets (3 + 3) are not part of it.
        let term = || with_post(entity("x"), 0.0, 3.0);
        let root = folded(with_post(add(vec![term(), term()]), 1.0, 5.0));
        assert_float_bits(&root, 5.0);
        assert_post_bits(&root, 1.0, 5.0);
        assert_float_bits(&tree("v.x*0+3+(v.x*0+3)"), 0.0);
    }

    #[test]
    fn terms_that_differ_in_name_post_op_or_shape_are_not_equal() {
        assert_eq!(shown("v.x+v.y"), "(Add v.x v.y)");
        assert_eq!(shown("v.x+v.x*2"), "[v.x*3+0]");
        assert_eq!(
            shown("v.x.a+v.x.b"),
            "(Add (MemberAccessor v.x) (MemberAccessor v.x))"
        );
        assert_eq!(
            shown("math.sin(v.x)+math.cos(v.x)"),
            "(Add (Sin v.x) (Cos v.x))"
        );
        assert_eq!(shown("v.a+c.a+c.a"), "(Add v.a c.a c.a)");
    }

    #[test]
    fn terms_with_side_effects_are_never_equal() {
        assert_eq!(
            shown("math.random(0,1)+math.random(0,1)"),
            "(Add (Random 0 1) (Random 0 1))"
        );
        assert_eq!(shown("q.is_baby+q.is_baby"), "(Add 40 40)");
        assert_eq!(shown("v.x+math.random(0,1)"), "(Add v.x (Random 0 1))");
        assert!(!terms_print_alike(&add(vec![
            parent(Op::RandomInt, vec![float(0.0)]),
            parent(Op::RandomInt, vec![float(0.0)])
        ])));
        assert!(!terms_print_alike(&add(vec![
            parent(Op::Assignment, vec![entity("a"), float(1.0)]),
            parent(Op::Assignment, vec![entity("a"), float(1.0)])
        ])));
        assert!(!terms_print_alike(&add(vec![
            parent(Op::QueryFunction, vec![]),
            parent(Op::QueryFunction, vec![])
        ])));
    }

    #[test]
    fn a_moved_constant_is_not_part_of_the_term_comparison() {
        assert_eq!(shown("(v.x==1)+(v.x==2)"), "[(LogicalEqual v.x)*2+0]");
        assert_eq!(shown("(v.x<3)+(v.x<4)"), "[(LessThan v.x)*2+0]");
    }

    #[test]
    fn terms_print_alike_is_true_for_zero_or_one_children() {
        assert!(terms_print_alike(&add(vec![])));
        assert!(terms_print_alike(&add(vec![entity("x")])));
        assert!(terms_print_alike(&add(vec![parent(
            Op::Random,
            vec![float(0.0)]
        )])));
    }

    #[test]
    fn terms_print_alike_compares_the_term_texts() {
        assert!(terms_print_alike(&add(vec![
            entity("x"),
            entity("x"),
            entity("x")
        ])));
        assert!(!terms_print_alike(&add(vec![
            entity("x"),
            entity("x"),
            entity("y")
        ])));
        assert!(!terms_print_alike(&add(vec![
            entity("x"),
            with_post(entity("x"), 2.0, 0.0)
        ])));
        assert!(terms_print_alike(&add(vec![float(1.0), float(1.0)])));
        assert!(!terms_print_alike(&add(vec![float(1.0), float(2.0)])));
        assert!(terms_print_alike(&add(vec![string("a"), string("a")])));
    }

    #[test]
    fn terms_print_alike_treats_an_empty_term_text_as_unequal() {
        let nameless = || {
            Node::token(
                Op::EntityVariable,
                Payload::Entity(Name::new("")),
                Span::new(0, 1),
            )
        };
        assert_eq!(nameless().term_text().as_deref(), Some(""));
        assert!(!terms_print_alike(&add(vec![nameless(), nameless()])));
    }

    #[test]
    fn terms_print_alike_compares_long_chains_by_shape() {
        // Past the term-text limit nothing is built; equal shapes are equal, one member apart is
        // not.
        assert!(
            member_chain(14, "a")
                .term_text_len()
                .is_some_and(|len| len > TERM_TEXT_LIMIT)
        );
        assert!(terms_print_alike(&add(vec![
            member_chain(14, "a"),
            member_chain(14, "a")
        ])));
        assert!(!terms_print_alike(&add(vec![
            member_chain(14, "a"),
            member_chain(14, "b")
        ])));
        assert!(!terms_print_alike(&add(vec![
            member_chain(14, "a"),
            member_chain(15, "a")
        ])));
        assert!(!terms_print_alike(&add(vec![
            member_chain(14, "a"),
            with_post(member_chain(14, "a"), 2.0, 0.0)
        ])));
    }

    /// A `Max` of floats padded so that, with `last`, its term text is exactly `TERM_TEXT_LIMIT`
    /// long.
    fn max_of_limit_length(last: Node) -> Node {
        let head = Op::Max.ordinal().to_string().len() as u64;
        let last_len = last.term_text_len().expect("no side effects");
        // `1` prints as eight characters, `10` as nine: find a mix that makes up the rest.
        let rest = TERM_TEXT_LIMIT - head - last_len;
        let ninths = (0..=rest / 9)
            .find(|ninths| (rest - ninths * 9).is_multiple_of(8))
            .expect("a mix of 8 and 9 reaches the length");
        let eighths = (rest - ninths * 9) / 8;
        let mut children: Vec<Node> = (0..eighths).map(|_| float(1.0)).collect();
        children.extend((0..ninths).map(|_| float(10.0)));
        children.push(last);
        let node = parent(Op::Max, children);
        assert_eq!(node.term_text_len(), Some(TERM_TEXT_LIMIT));
        node
    }

    #[test]
    fn a_term_text_of_exactly_the_limit_is_still_built_and_compared_as_text() {
        // `This` prints its op number, a string literal prints its hash: two shapes with one text.
        // At the limit the strings are built and compared, so the terms are equal; one character
        // more would compare shapes instead.
        let this = Node::token(Op::This, Payload::None, Span::new(0, 1));
        let lookalike = Node::token(
            Op::StringLiteral,
            Payload::Hash(u64::from(Op::This.ordinal())),
            Span::new(0, 1),
        );
        let (first, second) = (max_of_limit_length(this), max_of_limit_length(lookalike));
        assert_eq!(first.term_text(), second.term_text());
        assert!(!first.same_term_text_shape(&second));
        assert!(terms_print_alike(&add(vec![first, second])));
    }

    #[test]
    fn long_member_chains_merge_through_the_optimiser() {
        let chain = format!("v.x{}", ".a".repeat(14));
        assert_eq!(
            shown(&format!("{chain}+{chain}"))
                .matches("MemberAccessor")
                .count(),
            14
        );
        assert!(shown(&format!("{chain}+{chain}")).ends_with("*2+0]"));
        let other = format!("v.x{}.b", ".a".repeat(13));
        assert!(shown(&format!("{chain}+{other}")).starts_with("(Add "));
    }

    #[test]
    fn repeated_variables_merge_into_the_first_occurrence() {
        assert_eq!(shown("v.x+v.y+v.x"), "(Add [v.x*2+0] v.y)");
        assert_eq!(shown("v.a+v.b+v.a+v.b"), "(Add [v.a*2+0] [v.b*2+0])");
        assert_eq!(shown("v.a+v.b+v.b+v.a"), "(Add [v.a*2+0] [v.b*2+0])");
        assert_eq!(shown("v.a+v.a+v.b+v.b+v.a"), "(Add [v.a*3+0] [v.b*2+0])");
        assert_eq!(shown("v.x*2+v.x*3"), "[v.x*5+0]");
    }

    #[test]
    fn merging_adds_the_scales_and_the_offsets() {
        assert_eq!(shown("v.a+(v.b+1)+(v.a+2)"), "(Add [v.a*2+2] [v.b*1+1])");
        assert_eq!(shown("(v.x+1)+(v.x+2)"), "[v.x*2+3]");
    }

    #[test]
    fn only_entity_variables_merge_when_the_terms_are_not_all_equal() {
        assert_eq!(shown("v.a+c.a+c.a"), "(Add v.a c.a c.a)");
        assert_eq!(shown("t.a+v.b+t.a"), "(Add t.a v.b t.a)");
        assert_eq!(
            shown("v.a+v.x.b+v.x.b"),
            "(Add v.a (MemberAccessor v.x) (MemberAccessor v.x))"
        );
    }

    #[test]
    fn variable_names_are_case_insensitive() {
        assert_eq!(shown("v.A+V.a"), "[v.a*2+0]");
        assert_eq!(shown("variable.a+v.a"), "[v.a*2+0]");
    }

    #[test]
    fn a_term_that_cancels_leaves_the_sum_with_its_offset() {
        // `(v.x + 1) - v.x` is 0, not 1: the cancelled term takes its constant with it.
        assert_float_bits(&tree("(v.x+1)-v.x"), 0.0);
        assert_float_bits(&tree("v.x-v.x"), 0.0);
        assert_float_bits(&tree("(v.x+1)+(-v.x)"), 0.0);
        assert_float_bits(&tree("(v.x+1)+(-v.x-1)"), 0.0);
        assert_float_bits(&tree("v.x*2+v.x*-2"), 0.0);
        assert_eq!(shown("v.x+v.y-v.x"), "v.y");
        assert_eq!(shown("v.x+v.y+v.x-v.x-v.x"), "v.y");
    }

    #[test]
    fn a_sum_that_cancels_completely_is_a_float_with_the_sums_post_op() {
        let root = tree("(v.x+v.y+1)+(-v.x-v.y)");
        assert_float_bits(&root, 1.0);
        assert_post_bits(&root, 1.0, 1.0);
        assert_eq!(root.tree_notation(9), "[1*1+1]");
    }

    #[test]
    fn a_later_term_of_a_cancelled_variable_starts_a_new_merged_term() {
        assert_eq!(shown("v.x+v.y-v.x+v.x"), "(Add v.y v.x)");
        assert_eq!(shown("v.a-v.a+v.a"), "v.a");
        assert_eq!(shown("v.a+v.b-v.a+v.c+v.a"), "(Add v.b v.c v.a)");
    }

    #[test]
    fn the_merge_leaves_the_terms_of_other_names_in_order() {
        assert_eq!(shown("v.a+v.b+v.c+v.d+v.b"), "(Add v.a [v.b*2+0] v.c v.d)");
    }

    #[test]
    fn merge_without_a_repeated_name_does_nothing() {
        let mut node = terms(&["a", "b", "c"]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.tree_notation(9), "(Add v.a v.b v.c)");
        let mut single = terms(&["a"]);
        merge_entity_variable_terms(&mut single);
        assert_eq!(single.children.len(), 1);
        let mut empty = add(vec![]);
        merge_entity_variable_terms(&mut empty);
        assert!(empty.children.is_empty());
    }

    #[test]
    fn merge_adds_post_ops_in_source_order() {
        let mut node = add(vec![
            with_post(entity("a"), 1.5, 0.25),
            entity("b"),
            with_post(entity("a"), 2.0, 0.5),
        ]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 2);
        assert_post_bits(&node.children[0], 3.5, 0.75);
        assert!(node.children[1].is(Op::EntityVariable));
        assert_post_bits(&node.children[1], 1.0, 0.0);
    }

    #[test]
    fn merge_removes_a_term_whose_scale_cancels_exactly() {
        let mut node = add(vec![
            with_post(entity("a"), 1.0, 1.0),
            entity("b"),
            with_post(entity("a"), -1.0, 0.0),
        ]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.tree_notation(9), "(Add v.b)");
        let mut only = add(vec![
            with_post(entity("a"), 1.0, 7.0),
            with_post(entity("a"), -1.0, 9.0),
        ]);
        merge_entity_variable_terms(&mut only);
        assert!(only.children.is_empty());
    }

    #[test]
    fn merge_keeps_a_term_whose_scale_is_close_to_zero() {
        let mut node = add(vec![
            entity("a"),
            with_post(entity("a"), -0.999_999_94, 0.0),
        ]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 1);
        assert_eq!(
            node.children[0].post.scale.to_bits(),
            (1.0f32 + -0.999_999_94f32).to_bits()
        );
    }

    #[test]
    fn merge_starts_a_new_term_after_a_cancellation() {
        let mut node = add(vec![
            entity("a"),
            entity("b"),
            with_post(entity("a"), -1.0, 0.0),
            with_post(entity("a"), 5.0, 1.0),
        ]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 2);
        assert!(node.children[0].is(Op::EntityVariable));
        assert_post_bits(&node.children[0], 1.0, 0.0);
        assert_post_bits(&node.children[1], 5.0, 1.0);
        assert_eq!(node.tree_notation(9), "(Add v.b [v.a*5+1])");
    }

    #[test]
    fn merge_ignores_terms_that_are_not_entity_variables() {
        let mut node = add(vec![temp("a"), temp("a"), string("a"), string("a")]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 4);
    }

    #[test]
    fn names_with_the_same_hash_merge_only_with_their_own_twin() {
        let (a1, b1) = colliding("a", "b");
        let (a2, b2) = colliding("a", "b");
        assert_eq!(a1.value, a2.value);
        assert_ne!(a1.value, b1.value);
        assert_eq!(
            entity_variable_name(&a1).map(Name::hash),
            entity_variable_name(&b1).map(Name::hash)
        );
        let mut node = add(vec![a1, b1, a2, b2]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 2);
        assert_eq!(node.tree_notation(9), "(Add [v.a*2+0] [v.b*2+0])");
    }

    #[test]
    fn a_colliding_name_that_occurs_once_is_left_alone() {
        let (a1, b1) = colliding("a", "b");
        let (a2, _) = colliding("a", "b");
        let mut node = add(vec![a1, b1, a2]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 2);
        assert_post_bits(&node.children[0], 2.0, 0.0);
        assert_post_bits(&node.children[1], 1.0, 0.0);
        assert_eq!(
            entity_variable_name(&node.children[1]).map(Name::as_str),
            Some("variable.b")
        );
    }

    #[test]
    fn a_cancelled_colliding_term_starts_anew() {
        let (a1, b1) = colliding("a", "b");
        let (_, b2) = colliding("a", "b");
        let (_, b3) = colliding("a", "b");
        let mut cancelling = b2;
        cancelling.post.scale = -1.0;
        let mut node = add(vec![a1, b1, cancelling, b3]);
        merge_entity_variable_terms(&mut node);
        // `b` (first) and `-b` cancel; the later `b` stands alone again after the first `a`.
        assert_eq!(node.children.len(), 2);
        assert_eq!(
            entity_variable_name(&node.children[0]).map(Name::as_str),
            Some("variable.a")
        );
        assert_eq!(
            entity_variable_name(&node.children[1]).map(Name::as_str),
            Some("variable.b")
        );
        assert_post_bits(&node.children[1], 1.0, 0.0);
    }

    #[test]
    fn the_first_of_two_colliding_names_cancels_through_the_main_map() {
        let (a1, b1) = colliding("a", "b");
        let (mut a2, _) = colliding("a", "b");
        a2.post.scale = -1.0;
        let mut node = add(vec![a1, b1, a2]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 1);
        assert_eq!(
            entity_variable_name(&node.children[0]).map(Name::as_str),
            Some("variable.b")
        );
    }

    #[test]
    fn a_colliding_name_still_merges_after_the_name_holding_the_hash_cancelled() {
        // `a`, `b`, `-a`, `b` with one hash. `a` holds the hash and cancels; `b` was filed
        // under its name and still merges.
        let (a1, b1) = colliding("a", "b");
        let (mut a2, b2) = colliding("a", "b");
        a2.post.scale = -1.0;
        let mut node = add(vec![a1, b1, a2, b2]);
        merge_entity_variable_terms(&mut node);
        assert_eq!(node.children.len(), 1);
        assert_eq!(
            entity_variable_name(&node.children[0]).map(Name::as_str),
            Some("variable.b")
        );
        assert_post_bits(&node.children[0], 2.0, 0.0);
    }

    #[test]
    fn three_colliding_names_merge_each_with_its_own_twin_after_the_holder_cancels() {
        // `a`, `b`, `c`, `-a`, `c`, `b`, `a`: `a` holds the hash and cancels; `b` and `c` merge
        // through their names, and the last `a` holds the freed hash anew in its own position.
        let node =
            |name: &str, scale: f32, offset: f32| with_post(colliding(name, name).0, scale, offset);
        let mut sum = add(vec![
            node("a", 1.0, 0.5),
            node("b", 1.0, 0.25),
            node("c", 1.0, 0.125),
            node("a", -1.0, 0.0),
            node("c", 3.0, 1.0),
            node("b", 2.0, 2.0),
            node("a", 4.0, 4.0),
        ]);
        merge_entity_variable_terms(&mut sum);
        assert_eq!(
            sum.tree_notation(9),
            "(Add [v.b*3+2.25] [v.c*4+1.125] [v.a*4+4])"
        );
    }

    #[test]
    fn a_colliding_name_cancels_and_reappears_while_the_holder_stays() {
        // `a`, `b`, `-b`, `b`, `a`: `b` is collided, cancels, and its next term starts anew
        // behind `a`, which keeps the hash throughout.
        let node = |name: &str, scale: f32| with_post(colliding(name, name).0, scale, 0.0);
        let mut sum = add(vec![
            node("a", 1.0),
            node("b", 1.0),
            node("b", -1.0),
            node("b", 5.0),
            node("a", 1.0),
        ]);
        merge_entity_variable_terms(&mut sum);
        assert_eq!(sum.tree_notation(9), "(Add [v.a*2+0] [v.b*5+0])");
    }

    #[test]
    fn the_holder_cancels_and_reappears_while_a_collided_name_stays_live() {
        // `a`, `b`, `-a`, `a`, `b`, `a`: the second `a` holds the hash again; the collided `b`
        // keeps merging under its name, and the later `a` terms merge into the new holder.
        let node = |name: &str, scale: f32| with_post(colliding(name, name).0, scale, 0.0);
        let mut sum = add(vec![
            node("a", 1.0),
            node("b", 1.0),
            node("a", -1.0),
            node("a", 2.0),
            node("b", 1.0),
            node("a", 3.0),
        ]);
        merge_entity_variable_terms(&mut sum);
        assert_eq!(sum.tree_notation(9), "(Add [v.b*2+0] [v.a*5+0])");
    }

    #[test]
    fn a_collided_name_that_cancels_first_frees_only_its_own_entry() {
        // `a`, `b`, `-b`, `a`, `b`: `b` cancels before the holder `a`; `a` still merges through the
        // hash and the later `b` starts anew.
        let node = |name: &str, scale: f32| with_post(colliding(name, name).0, scale, 0.0);
        let mut sum = add(vec![
            node("a", 1.0),
            node("b", 1.0),
            node("b", -1.0),
            node("a", 1.0),
            node("b", 1.0),
        ]);
        merge_entity_variable_terms(&mut sum);
        assert_eq!(sum.tree_notation(9), "(Add [v.a*2+0] v.b)");
    }

    /// The quadratic merge `merge_entity_variable_terms` must match: every term against every later
    /// one, a term whose scale cancels to exactly zero leaving the list as soon as it does.
    fn pairwise_merge(terms: &mut Vec<Node>) {
        let mut i = 0;
        while i < terms.len() {
            let mut j = i + 1;
            let mut cancelled = false;
            while j < terms.len() {
                if entity_variable_name(&terms[i]).map(Name::as_str)
                    != entity_variable_name(&terms[j]).map(Name::as_str)
                {
                    j += 1;
                    continue;
                }
                let later = terms.remove(j);
                terms[i].post.scale = x86_64::add(terms[i].post.scale, later.post.scale);
                terms[i].post.offset = x86_64::add(terms[i].post.offset, later.post.offset);
                if terms[i].post.scale == 0.0 {
                    terms.remove(i);
                    cancelled = true;
                    break;
                }
            }
            if !cancelled {
                i += 1;
            }
        }
    }

    fn merged_terms(node: &Node) -> Vec<(String, u32, u32)> {
        node.children
            .iter()
            .map(|term| {
                (
                    entity_variable_name(term)
                        .map_or_else(String::new, |name| name.as_str().to_owned()),
                    term.post.scale.to_bits(),
                    term.post.offset.to_bits(),
                )
            })
            .collect()
    }

    #[test]
    fn colliding_names_merge_as_the_pairwise_scan_does_in_every_interleaving() {
        // Every sequence of up to five terms over three names with scales +1 and -1 (offsets
        // rounding-sensitive, so the order of the additions shows in the bits), once with the three
        // names on one hash and once with their own hashes: both give the pairwise scan's terms.
        const LETTERS: [&str; 3] = ["a", "b", "c"];
        for len in 1..=5u32 {
            for code in 0..6usize.pow(len) {
                let picks: Vec<(usize, f32)> = (0..len as usize)
                    .map(|position| {
                        let digit = code / 6usize.pow(position as u32) % 6;
                        (digit / 2, if digit.is_multiple_of(2) { 1.0 } else { -1.0 })
                    })
                    .collect();
                let build = |collide: bool| {
                    let children = picks
                        .iter()
                        .enumerate()
                        .map(|(position, &(letter, scale))| {
                            let term = if collide {
                                colliding(LETTERS[letter], LETTERS[letter]).0
                            } else {
                                entity(LETTERS[letter])
                            };
                            with_post(term, scale, 0.1 * (position + 1) as f32)
                        })
                        .collect();
                    add(children)
                };
                let mut expected = build(false);
                pairwise_merge(&mut expected.children);
                for collide in [false, true] {
                    let mut node = build(collide);
                    merge_entity_variable_terms(&mut node);
                    assert_eq!(
                        merged_terms(&node),
                        merged_terms(&expected),
                        "{picks:?}, one hash: {collide}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_wide_sum_merges_a_repeat_at_the_far_end() {
        // Hand-built: written out, a sum this wide would nest past the depth limit.
        let count = 5_000;
        let names: Vec<String> = (0..count).map(|index| format!("n{index}")).collect();
        let mut wide = add(names
            .iter()
            .map(|name| entity(name))
            .chain([entity("n0"), entity("n1"), entity("n4999")])
            .collect());
        merge_entity_variable_terms(&mut wide);
        assert_eq!(wide.children.len(), count);
        assert_post_bits(&wide.children[0], 2.0, 0.0);
        assert_post_bits(&wide.children[1], 2.0, 0.0);
        assert_post_bits(&wide.children[2], 1.0, 0.0);
        assert_post_bits(&wide.children[count - 1], 2.0, 0.0);
        let mut distinct = add(names.iter().map(|name| entity(name)).collect());
        merge_entity_variable_terms(&mut distinct);
        assert_eq!(distinct.children.len(), count);
        assert!(
            distinct
                .children
                .iter()
                .all(|term| term.post.scale == 1.0 && term.post.offset == 0.0)
        );
    }

    #[test]
    fn the_repeated_name_filter_has_no_false_negatives() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        for round in 0..200 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let len = 2 + (seed >> 60) as usize + round % 40;
            let mut names = Vec::new();
            for _ in 0..len {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                names.push(format!("n{}", (seed >> 33) % (len as u64 * 2)));
            }
            let node = add(names.iter().map(|name| entity(name)).collect());
            let mut counts = std::collections::HashMap::new();
            for name in &names {
                *counts.entry(name.clone()).or_insert(0u32) += 1;
            }
            let repeated = counts.values().any(|&count| count > 1);
            let filter = RepeatedNames::of(&node.children);
            if repeated {
                let filter = filter
                    .unwrap_or_else(|| panic!("round {round}: a repeated name sets a bit twice"));
                for (name, count) in &counts {
                    if *count > 1 {
                        assert!(
                            filter
                                .may_repeat(Name::new(format!("variable.{name}")).hash().as_u64()),
                            "{name}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_repeated_name_filter_has_sixteen_bits_per_term() {
        // 100 terms: 1600 bits, rounded up to 2048; a handful of terms: 64 bits at least.
        let names = vec!["a"; 100];
        let filter = RepeatedNames::of(&terms(&names).children).expect("a repeat");
        assert_eq!(filter.mask, 2047);
        let filter = RepeatedNames::of(&terms(&["a", "a"]).children).expect("a repeat");
        assert_eq!(filter.mask, 63);
    }

    #[test]
    fn the_repeated_name_filter_is_empty_without_a_repeat() {
        assert!(RepeatedNames::of(&[]).is_none());
        assert!(RepeatedNames::of(&terms(&["a"]).children).is_none());
        assert!(RepeatedNames::of(&terms(&["a", "b", "c"]).children).is_none());
        assert!(RepeatedNames::of(&[temp("a"), temp("a")]).is_none());
        let filter = RepeatedNames::of(&terms(&["a", "b", "a"]).children).expect("a repeat");
        assert!(filter.may_repeat(Name::new("variable.a").hash().as_u64()));
    }

    #[test]
    fn entity_variable_name_reads_only_entity_variables() {
        assert_eq!(
            entity_variable_name(&entity("a")).map(Name::as_str),
            Some("variable.a")
        );
        assert_eq!(entity_variable_name(&temp("a")), None);
        assert_eq!(entity_variable_name(&float(1.0)), None);
        let mismatched = Node::token(Op::EntityVariable, Payload::None, Span::new(0, 1));
        assert_eq!(entity_variable_name(&mismatched), None);
    }

    #[test]
    fn mix_is_splitmix64s_finaliser() {
        assert_eq!(mix(0), 0);
        assert_eq!(mix(1), 0x5692_161d_100b_05e5);
        assert_eq!(mix(0x0102), 0xa791_4009_ec98_240a);
        assert_eq!(mix(0xdead_beef), 0x4e06_2702_ec92_9eea);
    }

    #[test]
    fn mix_does_not_collide_on_sequential_inputs() {
        let outputs: std::collections::HashSet<u64> = (0..10_000u64).map(mix).collect();
        assert_eq!(outputs.len(), 10_000);
    }

    #[test]
    fn the_key_hasher_finishes_with_mix() {
        assert_eq!(HashKey::default().finish(), mix(0));
        let mut hasher = HashKey::default();
        hasher.write_u64(0xfeed);
        assert_eq!(hasher.finish(), mix(0xfeed));
    }

    #[test]
    fn the_key_hasher_folds_bytes_for_other_writes() {
        let mut hasher = HashKey::default();
        hasher.write(&[1, 2]);
        assert_eq!(hasher.finish(), mix(0x0102));
        let mut again = HashKey::default();
        again.write(&[1]);
        again.write(&[2]);
        assert_eq!(again.finish(), hasher.finish());
    }

    #[test]
    fn a_map_with_the_key_hasher_round_trips() {
        let mut map: HashMap<u64, usize, BuildHasherDefault<HashKey>> = HashMap::default();
        for key in 0..1_000u64 {
            map.insert(key.wrapping_mul(0x9e37_79b9_7f4a_7c15), key as usize);
        }
        assert_eq!(map.len(), 1_000);
        for key in 0..1_000u64 {
            assert_eq!(
                map.get(&key.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
                Some(&(key as usize))
            );
        }
    }

    #[test]
    fn only_the_last_term_is_new_after_a_sum_followed_by_one_term_that_is_not_a_sum() {
        let extends = |node: Node| new_terms(&node) == NewTerms::Last;
        assert!(extends(add(vec![terms(&["a", "b"]), entity("c")])));
        assert!(extends(add(vec![terms(&["a", "b"]), float(1.0)])));
        assert!(
            !extends(add(vec![entity("c"), terms(&["a", "b"])])),
            "the sum on the right"
        );
        assert!(
            !extends(add(vec![terms(&["a", "b"]), terms(&["c", "d"])])),
            "two sums"
        );
        assert!(!extends(add(vec![entity("a"), entity("b")])));
        assert!(
            !extends(add(vec![terms(&["a", "b"]), entity("c"), entity("d")])),
            "three children"
        );
        assert!(!extends(add(vec![terms(&["a", "b"])])));
    }

    #[test]
    fn flatten_add_keeps_the_terms_of_a_leading_sum_and_appends_the_rest() {
        let mut node = add(vec![terms(&["a", "b"]), entity("c")]);
        flatten_add(&mut node);
        assert_eq!(node.tree_notation(9), "(Add v.a v.b v.c)");
        let mut scaled = add(vec![
            with_post(terms(&["a", "b"]), 1.0, 2.0),
            with_post(terms(&["c", "d"]), -1.0, 0.0),
        ]);
        flatten_add(&mut scaled);
        assert_eq!(
            scaled.tree_notation(9),
            "[(Add v.a v.b [v.c*-1+-0] [v.d*-1+-0])*1+2]"
        );
    }

    #[test]
    fn merge_last_term_merges_into_the_earlier_term_of_the_same_variable() {
        let mut terms = vec![
            entity("a"),
            with_post(entity("b"), 2.0, 1.0),
            entity("c"),
            with_post(entity("b"), 3.0, 0.5),
        ];
        merge_last_term(&mut terms);
        assert_eq!(terms.len(), 3);
        assert_post_bits(&terms[1], 5.0, 1.5);
    }

    #[test]
    fn merge_last_term_removes_both_terms_when_the_scale_cancels() {
        let mut terms = vec![
            entity("a"),
            entity("b"),
            entity("c"),
            with_post(entity("b"), -1.0, 4.0),
        ];
        merge_last_term(&mut terms);
        assert_eq!(
            terms.iter().map(|t| t.tree_notation(9)).collect::<Vec<_>>(),
            ["v.a", "v.c"]
        );
    }

    #[test]
    fn merge_last_term_leaves_terms_without_an_earlier_twin_alone() {
        for last in [
            entity("d"),
            temp("a"),
            float(1.0),
            parent(Op::Sin, vec![entity("a")]),
        ] {
            let mut terms = vec![entity("a"), entity("b"), last];
            merge_last_term(&mut terms);
            assert_eq!(terms.len(), 3);
            assert!(terms.iter().all(|t| !t.has_post_op()));
        }
        let mut one = vec![entity("a")];
        merge_last_term(&mut one);
        assert_eq!(one.len(), 1);
        merge_last_term(&mut Vec::new());
    }

    #[test]
    fn merge_last_term_compares_names_not_hashes() {
        let (first, second) = colliding("a", "b");
        let mut terms = vec![first, entity("c"), second];
        merge_last_term(&mut terms);
        assert_eq!(terms.len(), 3, "one hash, two names");
        let (first, again) = colliding("a", "a");
        let mut terms = vec![first, entity("c"), again];
        merge_last_term(&mut terms);
        assert_eq!(terms.len(), 2);
        assert_post_bits(&terms[0], 2.0, 0.0);
    }

    #[test]
    fn merge_last_term_is_the_general_merge_when_the_earlier_names_are_distinct() {
        // Earlier terms: distinct names among four, each with a post-op; the last term: any of five
        // names (one new) with a scale that may cancel.
        let names = ["a", "b", "c", "d", "e"];
        for code in 0..16 * 5 * 3 {
            let (present, pick, scale) = (code % 16, code / 16 % 5, [1.0, -1.0, -2.0][code / 80]);
            let build = || {
                let mut terms: Vec<Node> = (0..4)
                    .filter(|position| (present >> position) & 1 == 1)
                    .map(|position| with_post(entity(names[position]), 1.0 + position as f32, 0.25))
                    .collect();
                terms.push(with_post(entity(names[pick]), scale, 0.125));
                terms
            };
            let mut ours = build();
            let mut general = add(build());
            merge_last_term(&mut ours);
            merge_entity_variable_terms(&mut general);
            assert_eq!(
                merged_terms(&add(ours)),
                merged_terms(&general),
                "code {code}"
            );
        }
    }

    /// `optimize_add` on `sum + term` flattened, told and not told that the node extends a sum.
    fn both_ways(sum: &str, term: Node) -> (String, String) {
        let run = |new: NewTerms, term: Node| {
            let mut node = add(vec![tree(sum), term]);
            assert!(node.children[0].is(Op::Add), "{sum} is not a sum");
            assert_eq!(new_terms(&node), NewTerms::Last);
            flatten_add(&mut node);
            optimize_add(&mut node, new);
            node.tree_notation(9)
        };
        let copy = Node::token(term.op, term.value.clone(), term.span);
        let copy = with_post(copy, term.post.scale, term.post.offset);
        (run(NewTerms::Last, term), run(NewTerms::All, copy))
    }

    #[test]
    fn an_extended_sum_optimises_as_the_general_case_does() {
        for sum in [
            "v.a+v.b",
            "v.a+v.b+v.c",
            "v.a*2+v.b-1",
            "v.a-v.b+v.c",
            "math.sin(v.a)+v.b",
            "v.a+v.b+v.a+v.c",
        ] {
            for term in [
                entity("a"),
                entity("b"),
                entity("z"),
                with_post(entity("a"), -1.0, 0.0),
                with_post(entity("b"), -1.0, 3.0),
                with_post(entity("a"), -2.0, 0.0),
                float(1.5),
                with_post(float(2.0), 3.0, 1.0),
                temp("a"),
            ] {
                let label = term.tree_notation(9);
                let (extended, general) = both_ways(sum, term);
                assert_eq!(extended, general, "{sum} + {label}");
            }
        }
    }

    #[test]
    fn a_sum_whose_terms_all_print_alike_is_still_compared_whole() {
        // `v.x - v.x` cancels and leaves two equal calls; a third one makes all three equal.
        assert_eq!(
            shown("math.sin(v.y) + v.x + math.sin(v.y) - v.x + math.sin(v.y)"),
            "[(Sin v.y)*3+0]"
        );
        assert_eq!(
            shown("math.sin(v.y) + v.x + math.sin(v.y) - v.x + math.cos(v.y)"),
            "(Add (Sin v.y) (Sin v.y) (Cos v.y))"
        );
    }

    #[test]
    fn terms_whose_strings_begin_differently_are_not_equal_without_being_printed() {
        assert!(!terms_print_alike(&add(vec![entity("a"), entity("b")])));
        assert!(!terms_print_alike(&add(vec![
            entity("a"),
            parent(Op::Sin, vec![entity("a")])
        ])));
        assert!(terms_print_alike(&add(vec![
            entity("a"),
            entity("a"),
            entity("a")
        ])));
        assert!(!terms_print_alike(&add(vec![entity("a"), entity("ab")])));
        assert!(!terms_print_alike(&add(vec![
            entity("a"),
            with_post(entity("a"), 2.0, 0.0)
        ])));
        assert!(terms_print_alike(&add(vec![float(1.0), float(1.0)])));
    }

    mod tree_shapes {
        use crate::compile::test_support::pipeline::{at, tree};

        #[test]
        fn merging_of_equal_terms() {
            assert_eq!(tree("v.x + v.x + 3"), "[v.x*2+3]");
            assert_eq!(tree("v.x + v.y + v.x"), "(Add [v.x*2+0] v.y)");
            assert_eq!(tree("v.x * 2 + v.x * 3"), "[v.x*5+0]");
            assert_eq!(tree(&vec!["v.x"; 255].join("+")), "[v.x*255+0]");
            assert_eq!(tree("v.x - v.x"), "0");
            assert_eq!(tree("v.x + v.y - v.x"), "v.y");
            assert_eq!(tree("v.x + v.y - v.x - v.y"), "0");
            assert_eq!(tree("v.x - v.x + 1"), "1");
            // Only entity variables merge pairwise; other terms merge when *all* terms are equal.
            assert_eq!(
                tree("math.floor(v.x) + math.floor(v.x)"),
                "[(Floor v.x)*2+0]"
            );
            assert_eq!(
                tree("math.abs(v.x) + math.abs(v.x) + math.abs(v.x)"),
                "(Add [(Abs v.x)*2+0] (Abs v.x))"
            );
            assert_eq!(tree("t.x + t.x"), "[t.x*2+0]");
            assert_eq!(tree("v.x.y + v.x.y"), "[(MemberAccessor v.x)*2+0]");
            assert_eq!(
                tree("v.x.y + v.x.z"),
                "(Add (MemberAccessor v.x) (MemberAccessor v.x))"
            );
            assert_eq!(
                tree("v.x * v.y + v.y * v.x"),
                "(Add (Mul v.x v.y) (Mul v.y v.x))"
            );
            // Random functions are never merged: each is its own draw.
            assert_eq!(
                tree("math.random(0,1) + math.random(0,1)"),
                "(Add (Random 0 1) (Random 0 1))"
            );
        }

        /// Long member chains merge as short ones do, without their term text being built.
        #[test]
        fn merging_of_long_member_chains() {
            let chain = |members: usize, last: &str| format!("v.a{}{last}", ".b".repeat(members));
            // Up to 8 members the string is within the limit it is built to; from 9 it is not.
            for members in [3, 8, 9, 10, 40, 200] {
                let same = at(&format!("{0} + {0}", chain(members, ".c")), 13);
                let merged = same.tree_notation(9).expect("tree");
                assert!(
                    merged.starts_with("[(MemberAccessor ") && merged.ends_with("*2+0]"),
                    "{members} members: {merged}"
                );

                let different = at(
                    &format!("{} + {}", chain(members, ".c"), chain(members, ".d")),
                    13,
                );
                assert!(
                    different
                        .tree_notation(9)
                        .expect("tree")
                        .starts_with("(Add "),
                    "{members} members"
                );

                let scaled = at(&format!("{0} + {0} * 2", chain(members, ".c")), 13);
                assert!(
                    scaled.tree_notation(9).expect("tree").starts_with("(Add "),
                    "{members} members: a post-op is part of the string"
                );
            }
        }

        #[test]
        fn addition_quirks() {
            // A constant moved into a node's value is not part of the comparison of terms: the
            // terms merge into twice the first.
            assert_eq!(tree("(v.x == 1) + (v.x == 2)"), "[(LogicalEqual v.x)*2+0]");
            assert_eq!(
                tree("math.max(v.x, 1) + math.max(v.x, 5)"),
                "[(Max v.x)*2+0]"
            );
            assert_eq!(
                tree("math.pow(v.x, 2) + math.pow(v.x, 3)"),
                "[(Pow v.x)*2+0]"
            );
            assert_eq!(tree("(v.x < 1) + (v.x < 3)"), "[(LessThan v.x)*2+0]");
            assert_eq!(tree("(v.x + 1) + (v.x + 2)"), "[v.x*2+3]");
            assert_eq!(tree("(v.x + 1) - v.x"), "0");
            assert_eq!(tree("(v.x + v.y + 1) - v.x - v.y"), "0");
            // A nested `&&` / `||` is flattened into its parent and loses its post-op.
            assert_eq!(
                tree("v.a && ((v.b && v.c) - 1)"),
                "(LogicalAnd v.a v.b v.c)"
            );
            assert_eq!(
                tree("v.a && (1 - (v.b && v.c))"),
                "(LogicalAnd v.a v.b v.c)"
            );
            assert_eq!(tree("v.z || ((v.b || v.c) - 1)"), "(LogicalOr v.z v.b v.c)");
        }

        /// A sum whose terms all cancel is a `Float` that keeps the sum's post-op. Moved into its
        /// parent it is worth `S·v + O` (here 2); as a conditional branch or condition, a whole
        /// expression, or an operand of an all-constant fold it is worth `v` (here 1).
        #[test]
        fn a_cancelled_sum_is_a_float_with_a_post_op() {
            let x = "((v.x + v.y + 1) + (-v.x - v.y))";
            assert_eq!(tree(x), "[1*1+1]");
            assert_eq!(tree(&format!("v.z * {x}")), "[v.z*2+0]", "10 with z = 5");
            assert_eq!(tree(&format!("v.z + {x}")), "[v.z*1+2]", "7 with z = 5");
            // Assigned or moved into `math.max`, it is the moved value (2).
            let assignment = at(&format!("v.r = {x};"), 13);
            assert_eq!(
                assignment.tree_notation(9).as_deref(),
                Some("(Semicolon (Assignment v.r))")
            );
            assert_eq!(tree(&format!("math.max(v.w, {x})")), "(Max v.w)");
            // The bytecode gives a conditional branch its value alone: `v.c ? X : 7` is 1 with c
            // = 1.
            assert_eq!(
                tree(&format!("v.c ? {x} : 7")),
                "(Conditional v.c [1*1+1] 7)"
            );
        }
    }
}
