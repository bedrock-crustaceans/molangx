//! The expression tree.
//!
//! The lexer produces a flat list of token nodes, the grouping passes of
//! [`parse`](crate::compile::parse) nest them, and [`sema`](crate::compile::sema) optimises the
//! result in place. A node is an [`ExpressionOp`], an affine post-op `(scale, offset)` the
//! optimiser folds `x·c`, `x + c` and `−x` into, a [`Payload`] and its children.
//!
//! # Invariants of a tree that compiled
//!
//! - Sections (`{ } [ ] ( )`), `,`, unary `-`, `math.pi` and `:` are gone.
//! - The tree is at most 256 levels deep (the root is level 0) and optimising never deepens it, so
//!   a recursive walk of a compiled tree is bounded. Passes over unbounded input use explicit
//!   stacks.
//! - `Add` has two or more children and no `Float` child (constants fold into an offset).
//!   `LogicalAnd` / `LogicalOr` are flattened n-ary: a nested node of the same op merges into its
//!   parent and its post-op is lost.
//! - `Mul` has two children, at most one of them the `Float` a literal divisor became.
//! - A constant operand folded into the parent's `value` is no longer a child: `LessThan`,
//!   `LessEqual`, `GreaterEqual`, `GreaterThan`, `Pow`, `Mod` and `Assignment` with one child carry
//!   their **second** operand in `value` ([`Payload::Float`]); `LogicalEqual`, `LogicalNotEqual`,
//!   `Min`, `Max` with one child carry the constant operand of **either** side (a float, or for
//!   `==` / `!=` a string hash). The moved value includes the moved node's post-op (`S·v + O`).
//! - `Array` has exactly one child, its index.
//! - **A `Float` may carry a post-op**: a sum whose terms all cancel
//!   (`(v.x + v.y + 1) + (-v.x - v.y)`) is the `Float` 1 with the sum's post-op `(1, +1)`. Moved or
//!   folded into its parent it is read as `S·v + O`. Where it stays (a `Conditional` branch or
//!   condition, an operand of an all-constant math function, comparison or arithmetic fold, or the
//!   whole expression) it is worth `v` alone: the `Float` instruction ignores the post-op.
//! - Other nodes keep a post-op although they have no value: a statement list used as an
//!   arithmetic operand (`v.y = {v.x = 1;} + 1;` is `[(Semicolon …)*1+1]`, the block worth 0), and
//!   below version 3 a string, resource variable, `loop`, `for_each`, `break`, `continue` or
//!   assignment used as one (`'a' + 3` at version 2 is `[hash*1+3]`).
//! - `Conditional` has `[condition, then]` or `[condition, then, else]`.
//! - `Semicolon` children are the statements; a `{…}` block is its `Semicolon` node.
//! - Calls (`QueryFunction`, math functions, `Loop`, `ForEach`) have their arguments as children.
//!   `ForEach` has three: an entity or temp variable, the collection, and the body (a statement
//!   list or any other expression). `Loop` has two: the count and a statement list.
//! - `MemberAccessor` has one child (the base) and the member name in `value`.
//! - `break` may stand outside a loop (`break;`, `v.x ? break : 1;`): it is logged, the expression
//!   is kept, and at run time it ends the expression without a message.
use std::fmt::{self, Write as _};
use std::ops::Range;

use crate::catalog::{MathCatalog, MathRef, QueryIndex};
use crate::compile::parse::TokenClasses;
use crate::diag::fixed6;
use crate::hash::HashedStr;
use crate::numeric::PostOp;
use crate::ops::ExpressionOp;

/// A byte range in the source text.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) struct Span {
    pub(super) start: u32,
    pub(super) end: u32,
}

impl Span {
    pub(super) const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// The span of the bytes `start..end`, each bound saturating at `u32::MAX` (reachable with
    /// the `source_length_limit` deviation off).
    pub(super) fn saturating(start: usize, end: usize) -> Self {
        let bound = |byte: usize| u32::try_from(byte).unwrap_or(u32::MAX);
        Self::new(bound(start), bound(end))
    }

    /// The span from the start of this one to the end of `last`.
    pub(super) fn to(self, last: Self) -> Self {
        Self::new(self.start, last.end)
    }
}

impl From<Span> for Range<u32> {
    fn from(span: Span) -> Self {
        span.start..span.end
    }
}

/// The longest term text that is built ([`Node::term_text`]).
///
/// The text doubles with every member of a `v.a.b.c…` chain; otherwise it is about as long as the
/// term's source text.
pub(super) const TERM_TEXT_LIMIT: u64 = 4_096;

/// A lower-case name with its FNV-1 hash.
///
/// For a variable it is the full canonical name with the long namespace (`variable.x`, `temp.i`,
/// `context.other`, `array.skins`, `geometry.default`); for a struct member it is the member name
/// without the leading dot (`b` of `v.a.b`). The hash is the key variables and members are stored
/// under at run time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    text: Box<str>,
    hash: HashedStr,
}

impl Name {
    /// The name of `text` (already lower-case and canonical) with its FNV-1 hash.
    pub fn new(text: impl Into<Box<str>>) -> Self {
        let text = text.into();
        let hash = HashedStr::new(&text);
        Self { text, hash }
    }

    /// A name with a chosen hash instead of the FNV-1 of its text: lets a unit test force two
    /// different names onto one hash.
    #[cfg(test)]
    pub(super) fn with_hash(text: impl Into<Box<str>>, hash: u64) -> Self {
        Self {
            text: text.into(),
            hash: HashedStr::from_u64(hash),
        }
    }

    /// The canonical text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// FNV-1 of the canonical text.
    pub fn hash(&self) -> HashedStr {
        self.hash
    }
}

/// A query resolved at lex time: its position in the compile's catalogue, and which of its
/// version ranges (implementations) the version selected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct QueryRef {
    pub(crate) index: QueryIndex,
    pub(crate) impl_idx: u8,
}

/// A node's value: what the parser and the optimiser store in a node.
#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    /// Nothing (operator nodes before a constant operand is moved in).
    None,
    /// A float: a literal, a folded constant, or a constant operand moved into its parent.
    Float(f32),
    /// The FNV-1 hash of a string literal.
    Hash(u64),
    /// `context.x`.
    Context(Name),
    /// `variable.x`.
    Entity(Name),
    /// `temp.x`.
    Temp(Name),
    /// `.name`.
    Member(Name),
    /// `array.x`.
    ArrayVariable(Name),
    /// `geometry.x`.
    Geometry(Name),
    /// `material.x`.
    Material(Name),
    /// `texture.x`.
    Texture(Name),
    /// A resolved query.
    Query(QueryRef),
    /// A function of the compile's math catalogue.
    HostMath(MathRef),
}

impl Payload {
    /// The name of a variable or resource: what the term text and the tree notation print for
    /// it. `None` for an array or a member, and for every other payload.
    fn printed_name(&self) -> Option<&str> {
        match self {
            Self::Context(n)
            | Self::Entity(n)
            | Self::Temp(n)
            | Self::Geometry(n)
            | Self::Material(n)
            | Self::Texture(n) => Some(n.as_str()),
            _ => None,
        }
    }
}

/// One node of the expression tree.
///
/// Two nodes are equal when everything but `below`, the grouping passes' note, is.
#[derive(Debug)]
pub struct Node {
    pub(crate) op: ExpressionOp,
    pub(crate) post: PostOp,
    pub(crate) value: Payload,
    pub(crate) children: Vec<Node>,
    /// The token the node came from, as a byte range of the source.
    pub(super) span: Span,
    /// The grouping passes' note of the token classes that may stand as leaves below the node;
    /// all of them while unknown.
    pub(super) below: TokenClasses,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.op == other.op
            && self.post == other.post
            && self.value == other.value
            && self.children == other.children
            && self.span == other.span
    }
}

impl Node {
    /// A leaf token.
    pub(super) fn token(op: ExpressionOp, value: Payload, span: Span) -> Self {
        Self {
            op,
            post: PostOp::IDENTITY,
            value,
            children: Vec::new(),
            span,
            below: TokenClasses::ALL,
        }
    }

    /// A childless node that holds a slot while the node that belongs there is out.
    pub(super) fn placeholder() -> Self {
        Self::token(ExpressionOp::Float, Payload::Float(0.0), Span::new(0, 0))
    }

    /// Whether the node's op is `op`.
    #[inline]
    pub fn is(&self, op: ExpressionOp) -> bool {
        self.op == op
    }

    /// Whether the node is a token nothing has been grouped under yet.
    #[inline]
    pub(super) fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    #[inline]
    pub(super) fn has_post_op(&self) -> bool {
        !self.post.is_identity()
    }

    /// The float in the node's value; 0 when the value is not a float (a non-float payload never
    /// reaches the places that call this on a compiled tree).
    #[inline]
    pub fn float(&self) -> f32 {
        match self.value {
            Payload::Float(v) => v,
            _ => 0.0,
        }
    }

    /// The operation.
    #[cfg(feature = "fuzz")]
    pub fn op(&self) -> ExpressionOp {
        self.op
    }

    /// The post-op.
    #[cfg(feature = "fuzz")]
    pub fn post(&self) -> PostOp {
        self.post
    }

    /// The value.
    #[cfg(feature = "fuzz")]
    pub fn value(&self) -> &Payload {
        &self.value
    }

    /// The children.
    #[cfg(feature = "fuzz")]
    pub fn children(&self) -> &[Node] {
        &self.children
    }

    /// A node built outside the compiler, with an empty span.
    #[cfg(feature = "fuzz")]
    pub fn new(op: ExpressionOp, value: Payload, post: PostOp, children: Vec<Node>) -> Self {
        Self {
            op,
            post,
            value,
            children,
            span: Span::new(0, 0),
            below: TokenClasses::ALL,
        }
    }

    /// The node becomes a `Float` with this value and no children; its post-op is left as it is.
    pub(super) fn set_float(&mut self, value: f32) {
        self.op = ExpressionOp::Float;
        self.value = Payload::Float(value);
        self.children.clear();
    }

    /// The node as messages name it: its op's friendly name, and a host math function's own name
    /// when `math` declares it.
    pub(super) fn friendly<'m>(&self, math: Option<&'m MathCatalog>) -> Friendly<'m> {
        Friendly {
            op: self.op,
            function: self.host_math_name(math),
        }
    }

    /// The name of the host math function the node calls, when `math` declares it.
    fn host_math_name<'m>(&self, math: Option<&'m MathCatalog>) -> Option<&'m str> {
        match (&self.value, math) {
            (Payload::HostMath(r), Some(math)) => Some(math.decl(*r).name()),
            _ => None,
        }
    }

    /// The smallest span covering the node and everything under it.
    pub(super) fn full_span(&self) -> Span {
        let mut span = self.span;
        let mut stack: Vec<&Node> = self.children.iter().collect();
        while let Some(node) = stack.pop() {
            span.start = span.start.min(node.span.start);
            span.end = span.end.max(node.span.end);
            stack.extend(node.children.iter());
        }
        span
    }

    /// The term text: the printed form two sub-trees are compared by when `+` merges equal
    /// terms. `None` when the sub-tree contains a random function, a volatile host math function,
    /// a query or an assignment, which is then unequal to everything.
    ///
    /// A constant operand that was moved into a node's value is **not** part of the string, so
    /// `(v.x == 1) + (v.x == 2)` merges into `(v.x == 1) * 2`.
    ///
    /// A member accessor prints its base twice, so the string doubles with every `.name` of a
    /// chain: callers ask [`term_text_len`](Self::term_text_len) first and never build a
    /// string longer than [`TERM_TEXT_LIMIT`].
    pub(super) fn term_text(&self) -> Option<String> {
        let mut out = String::new();
        self.write_term_text(&mut out).then_some(out)
    }

    fn write_term_text(&self, out: &mut String) -> bool {
        if self.term_text_has_side_effects() {
            return false;
        }
        if self.is(ExpressionOp::MemberAccessor)
            && let Some(base) = self.children.first()
        {
            base.write_term_text(out);
        }
        self.write_term_text_head(out);
        for child in &self.children {
            if !child.write_term_text(out) {
                return false;
            }
        }
        self.write_term_text_tail(out);
        true
    }

    /// The ops whose presence makes a sub-tree unequal to everything.
    fn term_text_has_side_effects(&self) -> bool {
        use ExpressionOp as Op;
        matches!(
            self.op,
            Op::Random | Op::RandomInt | Op::HostMathVolatile | Op::QueryFunction | Op::Assignment
        )
    }

    /// The node's own text in the term text, printed before its children.
    fn term_text_head(&self) -> Head<'_> {
        use ExpressionOp as Op;
        match (self.op, &self.value) {
            (Op::Float, _) => Head::Float(self.float()),
            (Op::StringLiteral, Payload::Hash(h)) => Head::Number(*h),
            (Op::StringLiteral, _) => Head::Number(0),
            (Op::MemberAccessor, Payload::Member(n)) => Head::Member(n.as_str()),
            (Op::HostMath, Payload::HostMath(r)) => Head::HostMath(r.index()),
            (Op::HostMath, _) => Head::HostMath(0),
            (
                Op::ContextVariable
                | Op::EntityVariable
                | Op::TempVariable
                | Op::GeometryVariable
                | Op::MaterialVariable
                | Op::TextureVariable
                | Op::Geometry
                | Op::Material
                | Op::Texture,
                value,
            ) => Head::Text(value.printed_name().unwrap_or("")),
            (Op::MemberAccessor, _) => Head::Text(""),
            (op, _) => Head::Number(u64::from(op.ordinal())),
        }
    }

    fn write_term_text_head(&self, out: &mut String) {
        match self.term_text_head() {
            Head::Float(v) => out.push_str(&fixed6(v)),
            Head::Number(n) => {
                let _ = write!(out, "{n}");
            }
            Head::Text(text) => out.push_str(text),
            Head::Member(name) => {
                out.push('.');
                out.push_str(name);
            }
            // The function's position, delimited, so that two functions never print alike.
            Head::HostMath(index) => {
                let _ = write!(out, "{}#{index}#", ExpressionOp::HostMath.ordinal());
            }
        }
    }

    /// The post-op in the term text, printed after the children.
    fn write_term_text_tail(&self, out: &mut String) {
        if self.has_post_op() {
            let _ = write!(
                out,
                "*{}+{}",
                fixed6(self.post.scale),
                fixed6(self.post.offset)
            );
        }
    }

    /// The text [`term_text`](Self::term_text) begins with, when the sub-tree has no side effects:
    /// the head of the node, or of the innermost base of a member accessor chain (which prints its
    /// base first). [`Lead::Unknown`] where that text takes formatting a float or is empty.
    pub(super) fn term_text_lead(&self) -> Lead<'_> {
        let mut node = self;
        while node.is(ExpressionOp::MemberAccessor)
            && let Some(base) = node.children.first()
        {
            node = base;
        }
        match node.term_text_head() {
            // An empty head leaves the lead to what follows.
            Head::Float(_) | Head::Text("") => Lead::Unknown,
            Head::Number(n) => Lead::Number(n),
            Head::Text(text) => Lead::Text(text),
            Head::Member(_) => Lead::Text("."),
            Head::HostMath(_) => Lead::Number(u64::from(ExpressionOp::HostMath.ordinal())),
        }
    }

    /// The length of [`term_text`](Self::term_text) without building it, saturating; `None`
    /// when the sub-tree contains a random function, a query or an assignment. Linear in the
    /// number of nodes, whatever the length is.
    pub(super) fn term_text_len(&self) -> Option<u64> {
        self.term_text_len_with(&mut String::new())
    }

    fn term_text_len_with(&self, scratch: &mut String) -> Option<u64> {
        if self.term_text_has_side_effects() {
            return None;
        }
        scratch.clear();
        self.write_term_text_head(scratch);
        self.write_term_text_tail(scratch);
        let mut len = scratch.len() as u64;
        for (index, child) in self.children.iter().enumerate() {
            let child_len = child.term_text_len_with(scratch)?;
            len = len.saturating_add(child_len);
            if index == 0 && self.is(ExpressionOp::MemberAccessor) {
                len = len.saturating_add(child_len);
            }
        }
        Some(len)
    }

    /// Whether two sub-trees print the same text node for node: the same ops, names, constants
    /// and post-ops in the same shape. Trees for which this holds have the same term text; it
    /// is the comparison used for strings too long to build.
    pub(super) fn same_term_text_shape(&self, other: &Self) -> bool {
        self.same_term_text_shape_with(other, &mut String::new(), &mut String::new())
    }

    fn same_term_text_shape_with(
        &self,
        other: &Self,
        ours: &mut String,
        theirs: &mut String,
    ) -> bool {
        if self.op != other.op || self.children.len() != other.children.len() {
            return false;
        }
        ours.clear();
        theirs.clear();
        self.write_term_text_head(ours);
        other.write_term_text_head(theirs);
        // The separator keeps `head | tail` splits apart.
        ours.push('\n');
        theirs.push('\n');
        self.write_term_text_tail(ours);
        other.write_term_text_tail(theirs);
        ours == theirs
            && self
                .children
                .iter()
                .zip(&other.children)
                .all(|(a, b)| a.same_term_text_shape_with(b, ours, theirs))
    }

    /// The tree notation: `(Op child…)` with `ExpressionOp` names, `[x*s+o]` for a post-op,
    /// constants with `float_digits` significant digits, string literals as their hash, and the
    /// namespaces abbreviated (`v.`/`t.`/`c.`).
    #[cfg(test)]
    pub(super) fn tree_notation(&self, float_digits: usize) -> String {
        self.tree_notation_in(float_digits, None)
    }

    /// [`tree_notation`](Self::tree_notation) with a host math function named as in `math`.
    pub(super) fn tree_notation_in(
        &self,
        float_digits: usize,
        math: Option<&MathCatalog>,
    ) -> String {
        let mut out = String::new();
        self.write_tree_notation(&mut out, float_digits, math);
        out.replace("variable.", "v.")
            .replace("temp.", "t.")
            .replace("context.", "c.")
    }

    fn write_tree_notation(
        &self,
        out: &mut String,
        float_digits: usize,
        math: Option<&MathCatalog>,
    ) {
        use ExpressionOp as Op;
        let post_op = self.has_post_op();
        if post_op {
            out.push('[');
        }
        // A host math function's own name when `math` is known.
        let name = self.host_math_name(math).unwrap_or(self.op.meta().name);
        if self.is(Op::Float) {
            out.push_str(&format_g(f64::from(self.float()), float_digits));
        } else if self.children.is_empty() {
            let op = self.op;
            if matches!(op, Op::Pi | Op::Break | Op::Continue | Op::This) {
                out.push_str(name);
            } else if let Some(text) = self.value.printed_name() {
                out.push_str(text);
            } else if let (Op::StringLiteral, Payload::Hash(h)) = (op, &self.value) {
                let _ = write!(out, "{h}");
            } else {
                let _ = write!(out, "{}", op.ordinal());
            }
        } else {
            out.push('(');
            out.push_str(name);
            out.push(' ');
            for (i, child) in self.children.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                child.write_tree_notation(out, float_digits, math);
            }
            out.push(')');
        }
        if post_op {
            let _ = write!(
                out,
                "*{}+{}]",
                format_g(f64::from(self.post.scale), 6),
                format_g(f64::from(self.post.offset), 6)
            );
        }
    }
}

/// A node's own text in the term text, printed before its children.
#[derive(Copy, Clone)]
enum Head<'a> {
    /// A constant, with six decimals.
    Float(f32),
    /// This number in decimal: a string's hash or an op's ordinal.
    Number(u64),
    /// A name; empty when the node holds none.
    Text(&'a str),
    /// `.name`.
    Member(&'a str),
    /// A host math function's position in its catalogue.
    HostMath(usize),
}

/// The text a term text begins with ([`Node::term_text_lead`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Lead<'a> {
    /// This text.
    Text(&'a str),
    /// This number in decimal.
    Number(u64),
    /// Not known without formatting a float.
    Unknown,
}

impl Lead<'_> {
    /// Whether two strings beginning with these leads differ: the leads differ before the shorter
    /// one ends. `false` when that is not known.
    pub(super) fn differs_from(self, other: Self) -> bool {
        let (mut ours, mut theirs) = ([0u8; 20], [0u8; 20]);
        let (Some(a), Some(b)) = (self.bytes(&mut ours), other.bytes(&mut theirs)) else {
            return false;
        };
        a.iter().zip(b).any(|(x, y)| x != y)
    }

    fn bytes<'s>(&'s self, buffer: &'s mut [u8; 20]) -> Option<&'s [u8]> {
        match *self {
            Self::Text(text) => Some(text.as_bytes()),
            Self::Number(mut n) => {
                let mut at = buffer.len();
                loop {
                    at -= 1;
                    buffer[at] = b'0' + (n % 10) as u8;
                    n /= 10;
                    if n == 0 {
                        break;
                    }
                }
                Some(&buffer[at..])
            }
            Self::Unknown => None,
        }
    }
}

/// Dropping a deep tree must not recurse: a 60,000-token chain of `!` nests that deep.
impl Drop for Node {
    fn drop(&mut self) {
        if self.children.is_empty() {
            return;
        }
        let mut stack = std::mem::take(&mut self.children);
        while let Some(mut node) = stack.pop() {
            stack.append(&mut node.children);
        }
    }
}

/// How a message names a node ([`Node::friendly`]): `Add '+'`, `Host Math Function 'math.f'`.
#[derive(Copy, Clone, Debug)]
pub(super) struct Friendly<'m> {
    op: ExpressionOp,
    function: Option<&'m str>,
}

impl fmt::Display for Friendly<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.op.friendly_name())?;
        match self.function {
            Some(name) => write!(f, " '{name}'"),
            None => Ok(()),
        }
    }
}

/// `value` with `digits` significant digits, in exponent form outside 1e-4..1e`digits`.
pub(super) fn format_g(value: f64, digits: usize) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }
    let digits = digits.max(1);
    // Round to `digits` significant digits in scientific form to learn the decimal exponent.
    let sci = format!("{:.*e}", digits - 1, value);
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if exponent < -4 || exponent >= digits as i32 {
        let mantissa = strip_trailing_zeros(mantissa);
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exponent.abs())
    } else {
        let decimals = (digits as i32 - 1 - exponent).max(0) as usize;
        strip_trailing_zeros(&format!("{value:.decimals$}")).to_owned()
    }
}

fn strip_trailing_zeros(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Side;
    use crate::compile::test_support::host_math;
    use crate::stdlib::query;
    use proptest::prelude::*;

    type Op = ExpressionOp;

    fn float(v: f32) -> Node {
        Node::token(Op::Float, Payload::Float(v), Span::new(0, 1))
    }

    fn entity(name: &str) -> Node {
        Node::token(
            Op::EntityVariable,
            Payload::Entity(Name::new(format!("variable.{name}"))),
            Span::new(0, 1),
        )
    }

    fn parent(op: Op, children: Vec<Node>) -> Node {
        let mut node = Node::token(op, Payload::None, Span::new(0, 1));
        node.children = children;
        node
    }

    fn member(name: &str, base: Node) -> Node {
        let mut node = Node::token(
            Op::MemberAccessor,
            Payload::Member(Name::new(name)),
            Span::new(0, 1),
        );
        node.children.push(base);
        node
    }

    fn with_post(mut node: Node, scale: f32, offset: f32) -> Node {
        node.post = PostOp::new(scale, offset);
        node
    }

    /// `v.x.a.b…`: a chain of `depth` member accessors on a variable.
    fn chain(depth: usize) -> Node {
        let mut node = entity("x");
        for _ in 0..depth {
            node = member("a", node);
        }
        node
    }

    /// A call of the test catalogue's `name` on `children`.
    fn host_call(name: &str, children: Vec<Node>) -> Node {
        let math = host_math();
        let function = math.find(name).unwrap();
        let op = if math.decl(function).is_volatile() {
            Op::HostMathVolatile
        } else {
            Op::HostMath
        };
        let mut node = Node::token(op, Payload::HostMath(function), Span::new(0, 1));
        node.children = children;
        node
    }

    #[test]
    fn two_host_functions_never_print_alike() {
        let (twice, double) = (
            host_call("math.twice", vec![entity("x")]),
            host_call("math.double", vec![entity("x")]),
        );
        assert_ne!(twice.term_text(), double.term_text());
        assert!(!twice.same_term_text_shape(&double));
        assert_eq!(
            twice.term_text(),
            host_call("math.twice", vec![entity("x")]).term_text()
        );
        assert!(twice.same_term_text_shape(&host_call("math.twice", vec![entity("x")])));
        assert_eq!(
            twice.term_text().map(|t| t.len() as u64),
            twice.term_text_len()
        );
        let math = host_math();
        assert_eq!(
            twice.friendly(Some(math)).to_string(),
            "Host Math Function 'math.twice'"
        );
        assert_eq!(twice.friendly(None).to_string(), "Host Math Function");
        // The delimiters keep an index apart from the text of the first argument.
        assert_eq!(
            twice.term_text().as_deref(),
            Some(
                format!(
                    "109#{}#variable.x",
                    math.find("math.twice").unwrap().index()
                )
                .as_str()
            )
        );
    }

    #[test]
    fn a_volatile_host_function_is_unequal_to_everything() {
        let noise = host_call("math.noise", vec![entity("x")]);
        assert_eq!(noise.term_text(), None);
        assert_eq!(noise.term_text_len(), None);
        assert_eq!(parent(Op::Add, vec![noise, float(1.0)]).term_text(), None);
    }

    #[test]
    fn the_tree_notation_names_a_host_function_when_the_catalogue_is_known() {
        let math = host_math();
        let call = host_call("math.sum", vec![entity("x"), float(2.0)]);
        assert_eq!(call.tree_notation_in(9, Some(math)), "(math.sum v.x 2)");
        assert_eq!(call.tree_notation(9), "(HostMath v.x 2)");
        let noise = with_post(host_call("math.noise", vec![float(1.0)]), 2.0, 0.0);
        assert_eq!(
            noise.tree_notation_in(9, Some(math)),
            "[(math.noise 1)*2+0]"
        );
    }

    #[test]
    fn a_name_keeps_its_text() {
        assert_eq!(Name::new("variable.x").as_str(), "variable.x");
        assert_eq!(Name::new(String::from("temp.i")).as_str(), "temp.i");
        assert_eq!(Name::new("").as_str(), "");
    }

    #[test]
    fn a_name_hashes_its_canonical_text() {
        assert_eq!(Name::new("variable.x").hash(), HashedStr::new("variable.x"));
        assert_eq!(Name::new("y").hash(), HashedStr::new("y"));
        assert_eq!(Name::new("").hash(), HashedStr::EMPTY);
        assert_ne!(
            Name::new("variable.x").hash(),
            Name::new("variable.y").hash()
        );
    }

    #[test]
    fn names_compare_by_text() {
        assert_eq!(Name::new("a"), Name::new(String::from("a")));
        assert_ne!(Name::new("a"), Name::new("b"));
        assert_eq!(Name::new("a").clone(), Name::new("a"));
    }

    #[test]
    fn a_token_is_a_leaf_with_the_identity_post_op() {
        let node = Node::token(Op::Add, Payload::None, Span::new(3, 7));
        assert_eq!(node.op, Op::Add);
        assert_eq!(node.post, PostOp::IDENTITY);
        assert_eq!(node.value, Payload::None);
        assert!(node.children.is_empty());
        assert_eq!(node.span, Span::new(3, 7));
        assert!(node.is_leaf());
    }

    #[test]
    fn is_compares_the_op() {
        let node = float(1.0);
        assert!(node.is(Op::Float));
        assert!(!node.is(Op::Add));
    }

    #[test]
    fn a_node_with_a_child_is_not_a_leaf() {
        let node = parent(Op::Add, vec![float(1.0)]);
        assert!(!node.is_leaf());
        assert!(parent(Op::Add, vec![]).is_leaf());
    }

    #[test]
    fn post_returns_the_scale_and_offset() {
        let node = with_post(float(1.0), 2.0, 3.0);
        assert_eq!(node.post, PostOp::new(2.0, 3.0));
        assert_eq!(float(1.0).post, PostOp::IDENTITY);
    }

    #[test]
    fn has_post_op_is_false_only_for_scale_one_and_offset_zero() {
        assert!(!float(1.0).has_post_op());
        assert!(!with_post(float(1.0), 1.0, -0.0).has_post_op());
        assert!(with_post(float(1.0), 1.0, 1e-30).has_post_op());
        assert!(with_post(float(1.0), 0.0, 0.0).has_post_op());
        assert!(with_post(float(1.0), -1.0, 0.0).has_post_op());
        assert!(with_post(float(1.0), 1.0, 1.0).has_post_op());
        assert!(with_post(float(1.0), f32::NAN, 0.0).has_post_op());
    }

    #[test]
    fn float_reads_a_float_payload_and_is_zero_otherwise() {
        assert_eq!(float(2.5).float(), 2.5);
        assert_eq!(float(-0.0).float().to_bits(), (-0.0f32).to_bits());
        assert_eq!(entity("x").float(), 0.0);
        assert_eq!(
            Node::token(Op::Add, Payload::None, Span::new(0, 0)).float(),
            0.0
        );
        assert_eq!(
            Node::token(Op::StringLiteral, Payload::Hash(7), Span::new(0, 0)).float(),
            0.0
        );
    }

    #[test]
    fn set_float_turns_the_node_into_a_childless_float_and_keeps_its_post_op() {
        let mut node = with_post(parent(Op::Add, vec![float(1.0), float(2.0)]), 2.0, 3.0);
        node.set_float(3.0);
        assert!(node.is(Op::Float));
        assert_eq!(node.value, Payload::Float(3.0));
        assert!(node.children.is_empty());
        assert_eq!(node.post, PostOp::new(2.0, 3.0));
    }

    #[test]
    fn set_float_keeps_the_span() {
        let mut node = Node::token(Op::Add, Payload::None, Span::new(1, 2));
        node.set_float(-4.0);
        assert_eq!(node.op, Op::Float);
        assert_eq!(node.span, Span::new(1, 2));
    }

    #[test]
    fn friendly_names_are_the_op_table_names() {
        assert_eq!(float(1.0).friendly(None).to_string(), "Float");
        assert_eq!(
            Node::token(Op::Add, Payload::None, Span::new(0, 0))
                .friendly(None)
                .to_string(),
            "Add '+'"
        );
        assert_eq!(
            Node::token(Op::Pointer, Payload::None, Span::new(0, 0))
                .friendly(None)
                .to_string(),
            "Pointer '->'"
        );
        assert_eq!(
            Node::token(Op::Mul, Payload::None, Span::new(0, 0))
                .friendly(None)
                .to_string(),
            "Multiply '*'"
        );
        assert_eq!(
            Node::token(Op::Abs, Payload::None, Span::new(0, 0))
                .friendly(None)
                .to_string(),
            Op::Abs.friendly_name()
        );
    }

    #[test]
    fn byte_offsets_saturate() {
        assert_eq!(Span::saturating(0, 65_536), Span::new(0, 65_536));
        assert_eq!(
            Span::saturating(u32::MAX as usize, u32::MAX as usize),
            Span::new(u32::MAX, u32::MAX)
        );
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            Span::saturating(u32::MAX as usize + 1, usize::MAX),
            Span::new(u32::MAX, u32::MAX)
        );
    }

    #[test]
    fn a_span_runs_to_the_end_of_another_and_converts_to_a_range() {
        assert_eq!(Span::new(2, 3).to(Span::new(7, 9)), Span::new(2, 9));
        assert_eq!(Range::from(Span::new(4, 6)), 4..6);
    }

    #[test]
    fn a_leaf_s_full_span_is_its_own() {
        assert_eq!(
            Node::token(Op::Add, Payload::None, Span::new(5, 6)).full_span(),
            Span::new(5, 6)
        );
    }

    #[test]
    fn full_span_covers_every_descendant() {
        let mut inner = Node::token(Op::Mul, Payload::None, Span::new(9, 10));
        inner
            .children
            .push(Node::token(Op::Float, Payload::Float(1.0), Span::new(0, 1)));
        let mut node = Node::token(Op::Add, Payload::None, Span::new(5, 6));
        node.children
            .push(Node::token(Op::Float, Payload::Float(1.0), Span::new(1, 2)));
        node.children.push(inner);
        node.children.push(Node::token(
            Op::Float,
            Payload::Float(1.0),
            Span::new(9, 12),
        ));
        assert_eq!(node.full_span(), Span::new(0, 12));
    }

    #[test]
    fn full_span_of_a_deep_chain_is_iterative() {
        let mut node = Node::token(Op::Float, Payload::Float(1.0), Span::new(3, 4));
        for i in 0..10_000u32 {
            let mut wrapper = Node::token(Op::LogicalNot, Payload::None, Span::new(10 + i, 11 + i));
            wrapper.children.push(node);
            node = wrapper;
        }
        assert_eq!(node.full_span(), Span::new(3, 10_010));
    }

    #[test]
    fn the_longest_term_text_built_is_4096_bytes() {
        assert_eq!(TERM_TEXT_LIMIT, 4_096);
    }

    #[test]
    fn a_float_prints_six_decimals() {
        assert_eq!(float(2.0).term_text().as_deref(), Some("2.000000"));
        assert_eq!(float(-0.5).term_text().as_deref(), Some("-0.500000"));
        assert_eq!(float(f32::INFINITY).term_text().as_deref(), Some("inf"));
    }

    #[test]
    fn a_post_op_is_printed_after_the_node() {
        assert_eq!(
            with_post(float(2.0), 2.0, 1.0).term_text().as_deref(),
            Some("2.000000*2.000000+1.000000")
        );
        let node = with_post(parent(Op::Add, vec![float(1.0), float(2.0)]), 3.0, 0.0);
        assert_eq!(
            node.term_text().as_deref(),
            Some("91.0000002.000000*3.000000+0.000000")
        );
    }

    #[test]
    fn a_hash_prints_its_decimal_value() {
        assert_eq!(
            Node::token(Op::StringLiteral, Payload::Hash(77), Span::new(0, 0))
                .term_text()
                .as_deref(),
            Some("77")
        );
        assert_eq!(
            Node::token(Op::StringLiteral, Payload::None, Span::new(0, 0))
                .term_text()
                .as_deref(),
            Some("0")
        );
    }

    #[test]
    fn a_variable_prints_its_canonical_name() {
        assert_eq!(entity("x").term_text().as_deref(), Some("variable.x"));
        let temp = Node::token(
            Op::TempVariable,
            Payload::Temp(Name::new("temp.i")),
            Span::new(0, 0),
        );
        assert_eq!(temp.term_text().as_deref(), Some("temp.i"));
        let geometry = Node::token(
            Op::GeometryVariable,
            Payload::Geometry(Name::new("geometry.a.b")),
            Span::new(0, 0),
        );
        assert_eq!(geometry.term_text().as_deref(), Some("geometry.a.b"));
    }

    #[test]
    fn any_other_node_prints_its_op_ordinal_then_its_children() {
        let node = parent(Op::Add, vec![float(1.0), float(2.0)]);
        assert_eq!(node.term_text().as_deref(), Some("91.0000002.000000"));
        assert_eq!(
            parent(Op::Mul, vec![entity("a"), entity("b")])
                .term_text()
                .as_deref(),
            Some("31variable.avariable.b")
        );
        assert_eq!(
            Node::token(Op::DieRoll, Payload::None, Span::new(0, 0))
                .term_text()
                .as_deref(),
            Some("18")
        );
    }

    #[test]
    fn side_effects_anywhere_make_the_string_none() {
        for op in [Op::Random, Op::RandomInt, Op::QueryFunction, Op::Assignment] {
            let leaf = Node::token(op, Payload::None, Span::new(0, 0));
            assert_eq!(leaf.term_text(), None, "{op:?}");
            assert_eq!(leaf.term_text_len(), None, "{op:?}");
            let nested = parent(
                Op::Add,
                vec![
                    float(1.0),
                    parent(
                        Op::Mul,
                        vec![float(2.0), Node::token(op, Payload::None, Span::new(0, 0))],
                    ),
                ],
            );
            assert_eq!(nested.term_text(), None, "{op:?}");
            assert_eq!(nested.term_text_len(), None, "{op:?}");
        }
    }

    #[test]
    fn a_die_roll_has_no_side_effect() {
        let node = parent(Op::DieRoll, vec![float(1.0)]);
        assert!(node.term_text().is_some());
        assert!(node.term_text_len().is_some());
    }

    #[test]
    fn a_member_accessor_prints_its_base_twice() {
        let node = member("y", entity("x"));
        assert_eq!(node.term_text().as_deref(), Some("variable.x.yvariable.x"));
        assert_eq!(node.term_text_len(), Some(22));
    }

    #[test]
    fn a_member_chain_doubles_its_length_with_every_accessor() {
        let lengths: Vec<u64> = (0..6).map(|d| chain(d).term_text_len().unwrap()).collect();
        assert_eq!(lengths, [10, 22, 46, 94, 190, 382]);
        for (d, length) in lengths.iter().enumerate() {
            assert_eq!(chain(d).term_text().unwrap().len() as u64, *length);
        }
    }

    #[test]
    fn a_long_member_chain_exceeds_the_limit_without_being_built() {
        let length = chain(12).term_text_len().unwrap();
        assert_eq!(length, 49_150);
        assert!(length > TERM_TEXT_LIMIT);
        assert!(chain(9).term_text_len().unwrap() > TERM_TEXT_LIMIT);
        assert!(chain(8).term_text_len().unwrap() < TERM_TEXT_LIMIT);
    }

    #[test]
    fn the_length_saturates_instead_of_overflowing() {
        assert_eq!(chain(80).term_text_len(), Some(u64::MAX));
        assert_eq!(chain(300).term_text_len(), Some(u64::MAX));
    }

    #[test]
    fn the_length_counts_the_post_ops_too() {
        let node = with_post(parent(Op::Add, vec![float(1.0), float(2.0)]), 3.0, 0.0);
        assert_eq!(
            node.term_text_len(),
            Some(node.term_text().unwrap().len() as u64)
        );
    }

    #[test]
    fn equal_trees_have_the_same_shape() {
        let a = parent(Op::Add, vec![entity("x"), float(2.0)]);
        let b = parent(Op::Add, vec![entity("x"), float(2.0)]);
        assert!(a.same_term_text_shape(&b));
        assert!(a.same_term_text_shape(&a));
    }

    #[test]
    fn trees_of_a_different_shape_differ() {
        let a = parent(Op::Add, vec![entity("x"), float(2.0)]);
        assert!(
            !a.same_term_text_shape(&parent(Op::Add, vec![entity("x"), float(3.0)])),
            "a constant"
        );
        assert!(
            !a.same_term_text_shape(&parent(Op::Add, vec![entity("y"), float(2.0)])),
            "a name"
        );
        assert!(
            !a.same_term_text_shape(&parent(Op::Mul, vec![entity("x"), float(2.0)])),
            "the op"
        );
        assert!(
            !a.same_term_text_shape(&parent(Op::Add, vec![entity("x"), float(2.0), float(2.0)])),
            "the child count"
        );
        assert!(
            !a.same_term_text_shape(&with_post(
                parent(Op::Add, vec![entity("x"), float(2.0)]),
                2.0,
                0.0
            )),
            "the post-op"
        );
    }

    #[test]
    fn the_head_and_the_post_op_are_compared_apart() {
        // `7` with post-op text `*1.000000+0.000000` could otherwise look like a head ending the
        // same way.
        let a = with_post(
            Node::token(Op::StringLiteral, Payload::Hash(7), Span::new(0, 0)),
            2.0,
            0.0,
        );
        let b = Node::token(Op::StringLiteral, Payload::Hash(7), Span::new(0, 0));
        assert!(!a.same_term_text_shape(&b));
    }

    #[test]
    fn shapes_are_compared_for_trees_too_long_to_print() {
        let a = chain(40);
        let b = chain(40);
        assert!(a.same_term_text_shape(&b));
        let mut other = chain(40);
        other.children[0].value = Payload::Member(Name::new("b"));
        assert!(!a.same_term_text_shape(&other));
        assert!(!chain(40).same_term_text_shape(&chain(41)));
    }

    fn small_tree() -> impl Strategy<Value = Node> {
        let leaf = prop_oneof![
            (0u8..4).prop_map(|v| float(f32::from(v))),
            prop_oneof![Just("x"), Just("y")].prop_map(entity),
            (0u64..3).prop_map(|h| Node::token(
                Op::StringLiteral,
                Payload::Hash(h),
                Span::new(0, 0)
            )),
        ];
        leaf.prop_recursive(4, 24, 3, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 2..4).prop_map(|c| parent(Op::Add, c)),
                proptest::collection::vec(inner.clone(), 2..3).prop_map(|c| parent(Op::Mul, c)),
                inner.clone().prop_map(|c| member("m", c)),
                (inner, 1u8..3).prop_map(|(c, s)| with_post(
                    parent(Op::LogicalNot, vec![c]),
                    f32::from(s),
                    0.0
                )),
            ]
        })
    }

    fn config() -> ProptestConfig {
        ProptestConfig {
            cases: 128,
            failure_persistence: None,
            ..ProptestConfig::default()
        }
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn the_length_is_the_length_of_the_string(tree in small_tree()) {
            let string = tree.term_text().unwrap();
            prop_assert_eq!(tree.term_text_len(), Some(string.len() as u64));
        }

        #[test]
        fn a_tree_has_the_same_shape_as_itself_and_as_an_identical_copy(tree in small_tree()) {
            prop_assert!(tree.same_term_text_shape(&tree));
        }

        #[test]
        fn the_same_shape_means_the_same_string(a in small_tree(), b in small_tree()) {
            if a.same_term_text_shape(&b) {
                prop_assert_eq!(a.term_text(), b.term_text());
            }
        }

        #[test]
        fn the_lead_begins_the_string(tree in small_tree()) {
            let string = tree.term_text().unwrap();
            let mut buffer = [0u8; 20];
            if let Some(lead) = tree.term_text_lead().bytes(&mut buffer) {
                prop_assert!(string.as_bytes().starts_with(lead), "{string:?}");
            }
        }

        #[test]
        fn leads_that_differ_mean_strings_that_differ(a in small_tree(), b in small_tree()) {
            if a.term_text_lead().differs_from(b.term_text_lead()) {
                prop_assert_ne!(a.term_text(), b.term_text());
            }
        }
    }

    #[test]
    fn the_lead_of_a_variable_is_its_name() {
        assert_eq!(entity("x").term_text_lead(), Lead::Text("variable.x"));
        assert_eq!(
            with_post(entity("x"), 2.0, 1.0).term_text_lead(),
            Lead::Text("variable.x")
        );
        let unnamed = Node::token(Op::EntityVariable, Payload::None, Span::new(0, 1));
        assert_eq!(
            unnamed.term_text_lead(),
            Lead::Unknown,
            "prints nothing: the lead is what follows"
        );
        let empty = Node::token(
            Op::TempVariable,
            Payload::Temp(Name::new("")),
            Span::new(0, 1),
        );
        assert_eq!(empty.term_text_lead(), Lead::Unknown);
    }

    #[test]
    fn the_lead_of_a_float_is_unknown() {
        assert_eq!(float(1.0).term_text_lead(), Lead::Unknown);
    }

    #[test]
    fn the_lead_of_a_string_is_its_hash_and_of_another_node_its_ordinal() {
        assert_eq!(
            Node::token(Op::StringLiteral, Payload::Hash(77), Span::new(0, 1)).term_text_lead(),
            Lead::Number(77)
        );
        assert_eq!(
            Node::token(Op::StringLiteral, Payload::None, Span::new(0, 1)).term_text_lead(),
            Lead::Number(0)
        );
        assert_eq!(
            parent(Op::Mul, vec![entity("x"), float(2.0)]).term_text_lead(),
            Lead::Number(u64::from(Op::Mul.ordinal()))
        );
    }

    #[test]
    fn the_lead_of_a_member_accessor_is_the_lead_of_its_innermost_base() {
        assert_eq!(chain(3).term_text_lead(), Lead::Text("variable.x"));
        let bare = Node::token(
            Op::MemberAccessor,
            Payload::Member(Name::new("m")),
            Span::new(0, 1),
        );
        assert_eq!(bare.term_text_lead(), Lead::Text("."));
        let nameless = Node::token(Op::MemberAccessor, Payload::None, Span::new(0, 1));
        assert_eq!(nameless.term_text_lead(), Lead::Unknown);
    }

    #[test]
    fn leads_differ_only_before_the_shorter_one_ends() {
        assert!(Lead::Text("variable.a").differs_from(Lead::Text("variable.b")));
        assert!(
            !Lead::Text("variable.a").differs_from(Lead::Text("variable.ab")),
            "a prefix: the rest decides"
        );
        assert!(!Lead::Text("variable.a").differs_from(Lead::Text("variable.a")));
        assert!(Lead::Number(12).differs_from(Lead::Number(13)));
        assert!(!Lead::Number(1).differs_from(Lead::Number(12)));
        assert!(!Lead::Number(12).differs_from(Lead::Text("12.5")));
        assert!(Lead::Number(0).differs_from(Lead::Text("-1")));
        assert!(!Lead::Unknown.differs_from(Lead::Text("x")));
        assert!(!Lead::Text("x").differs_from(Lead::Unknown));
    }

    #[test]
    fn a_number_lead_is_printed_in_decimal() {
        let mut buffer = [0u8; 20];
        for (n, text) in [
            (0, "0"),
            (7, "7"),
            (10, "10"),
            (255, "255"),
            (u64::MAX, "18446744073709551615"),
        ] {
            assert_eq!(Lead::Number(n).bytes(&mut buffer), Some(text.as_bytes()));
        }
        assert_eq!(Lead::Unknown.bytes(&mut buffer), None);
    }

    #[test]
    fn a_float_prints_with_the_significant_digits_asked_for() {
        assert_eq!(float(1.5).tree_notation(9), "1.5");
        assert_eq!(float(0.1).tree_notation(9), "0.100000001");
        assert_eq!(float(0.1).tree_notation(6), "0.1");
        assert_eq!(float(std::f32::consts::PI).tree_notation(6), "3.14159");
        assert_eq!(float(-0.0).tree_notation(9), "-0");
        assert_eq!(float(1e20).tree_notation(6), "1e+20");
    }

    #[test]
    fn a_post_op_wraps_the_node_in_brackets() {
        assert_eq!(
            with_post(float(1.5), 2.0, 1.0).tree_notation(9),
            "[1.5*2+1]"
        );
        assert_eq!(
            with_post(float(1.5), 2.0, -0.5).tree_notation(9),
            "[1.5*2+-0.5]"
        );
    }

    #[test]
    fn the_post_op_always_has_six_digits() {
        let node = with_post(float(1.0), 1.234_567_9, 0.0);
        assert_eq!(node.tree_notation(9), "[1*1.23457+0]");
    }

    #[test]
    fn variables_are_printed_with_abbreviated_namespaces() {
        assert_eq!(entity("x").tree_notation(9), "v.x");
        assert_eq!(
            Node::token(
                Op::TempVariable,
                Payload::Temp(Name::new("temp.i")),
                Span::new(0, 0)
            )
            .tree_notation(9),
            "t.i"
        );
        assert_eq!(
            Node::token(
                Op::ContextVariable,
                Payload::Context(Name::new("context.x")),
                Span::new(0, 0)
            )
            .tree_notation(9),
            "c.x"
        );
        assert_eq!(
            Node::token(
                Op::GeometryVariable,
                Payload::Geometry(Name::new("geometry.a")),
                Span::new(0, 0)
            )
            .tree_notation(9),
            "geometry.a"
        );
    }

    #[test]
    fn the_constants_and_jumps_print_their_op_names() {
        for (op, text) in [
            (Op::Pi, "Pi"),
            (Op::Break, "Break"),
            (Op::Continue, "Continue"),
            (Op::This, "This"),
        ] {
            assert_eq!(
                Node::token(op, Payload::None, Span::new(0, 0)).tree_notation(9),
                text
            );
        }
    }

    #[test]
    fn another_tokenless_leaf_prints_its_ordinal() {
        assert_eq!(
            Node::token(Op::Add, Payload::None, Span::new(0, 0)).tree_notation(9),
            "9"
        );
        assert_eq!(
            Node::token(Op::LeftParenthesis, Payload::None, Span::new(0, 0)).tree_notation(9),
            "4"
        );
    }

    #[test]
    fn a_string_prints_its_hash() {
        assert_eq!(
            Node::token(Op::StringLiteral, Payload::Hash(12_345), Span::new(0, 0)).tree_notation(9),
            "12345"
        );
    }

    #[test]
    fn a_node_with_children_prints_its_name_and_its_children() {
        let node = parent(Op::Add, vec![float(1.0), entity("a")]);
        assert_eq!(node.tree_notation(9), "(Add 1 v.a)");
        assert_eq!(
            parent(
                Op::Mul,
                vec![parent(Op::Add, vec![float(1.0), float(2.0)]), float(3.0)]
            )
            .tree_notation(9),
            "(Mul (Add 1 2) 3)"
        );
    }

    #[test]
    fn a_post_op_on_a_call_wraps_the_whole_call() {
        let mut pointer = parent(
            Op::Pointer,
            vec![
                Node::token(
                    Op::ContextVariable,
                    Payload::Context(Name::new("context.x")),
                    Span::new(0, 0),
                ),
                entity("y"),
            ],
        );
        pointer.post.offset = 1.0;
        assert_eq!(pointer.tree_notation(9), "[(Pointer c.x v.y)*1+1]");
    }

    #[test]
    fn a_member_accessor_prints_as_a_call_with_its_base() {
        assert_eq!(
            member("y", entity("x")).tree_notation(9),
            "(MemberAccessor v.x)"
        );
    }

    #[test]
    fn a_query_leaf_prints_its_ordinal() {
        let index = crate::stdlib::queries(Side::Client)
            .index_of(query::IS_BABY)
            .expect("a standard query");
        let node = Node::token(
            Op::QueryFunction,
            Payload::Query(QueryRef { index, impl_idx: 0 }),
            Span::new(0, 0),
        );
        assert_eq!(node.tree_notation(9), "40");
    }

    #[test]
    fn format_g_examples() {
        assert_eq!(format_g(3.141_592_741_012_573, 6), "3.14159");
        assert_eq!(format_g(3.141_592_741_012_573, 9), "3.14159274");
        assert_eq!(format_g(1000.0, 6), "1000");
        assert_eq!(format_g(0.001, 6), "0.001");
        assert_eq!(format_g(1e-5, 6), "1e-05");
        assert_eq!(format_g(1e30, 6), "1e+30");
        assert_eq!(format_g(2.099_999_904_632_568, 9), "2.0999999");
        assert_eq!(format_g(-0.0, 6), "-0");
        assert_eq!(format_g(123_456_789.0, 6), "1.23457e+08");
        assert_eq!(format_g(0.5, 6), "0.5");
        assert_eq!(format_g(f64::INFINITY, 6), "inf");
    }

    #[test]
    fn format_g_special_values() {
        assert_eq!(format_g(f64::NAN, 6), "nan");
        assert_eq!(format_g(f64::NEG_INFINITY, 6), "-inf");
        assert_eq!(format_g(0.0, 6), "0");
        assert_eq!(format_g(-1.5, 6), "-1.5");
    }

    #[test]
    fn format_g_switches_to_an_exponent_at_the_digit_count() {
        assert_eq!(format_g(100_000.0, 6), "100000");
        assert_eq!(format_g(123_456.0, 6), "123456");
        assert_eq!(format_g(1_000_000.0, 6), "1e+06");
        assert_eq!(format_g(1_234_567.0, 6), "1.23457e+06");
        assert_eq!(format_g(1_000_000.0, 9), "1000000");
    }

    #[test]
    fn format_g_switches_to_an_exponent_below_one_ten_thousandth() {
        assert_eq!(format_g(0.0001, 6), "0.0001");
        assert_eq!(format_g(0.000_123_456, 6), "0.000123456");
        assert_eq!(format_g(0.00001, 6), "1e-05");
        assert_eq!(format_g(0.000_012_345_6, 6), "1.23456e-05");
    }

    #[test]
    fn format_g_rounds_before_it_picks_the_notation() {
        assert_eq!(format_g(999_999.5, 6), "1e+06");
        assert_eq!(format_g(99_999.97, 6), "100000");
        assert_eq!(format_g(0.000_099_999_99, 6), "0.0001");
    }

    #[test]
    fn format_g_with_no_digits_uses_one() {
        assert_eq!(format_g(15.0, 0), "2e+01");
        assert_eq!(format_g(0.5, 0), "0.5");
        assert_eq!(format_g(7.0, 1), "7");
    }

    #[test]
    fn format_g_prints_f32_values_exactly() {
        assert_eq!(format_g(f64::from(0.1f32), 9), "0.100000001");
        assert_eq!(format_g(f64::from(0.1f32), 6), "0.1");
        assert_eq!(format_g(f64::from(f32::MIN_POSITIVE), 6), "1.17549e-38");
        assert_eq!(format_g(f64::from(f32::MAX), 9), "3.40282347e+38");
    }

    #[test]
    fn format_g_keeps_trailing_zeros_off() {
        assert_eq!(format_g(1.5, 9), "1.5");
        assert_eq!(format_g(100.0, 9), "100");
        assert_eq!(format_g(1e10, 6), "1e+10");
        assert_eq!(format_g(1.5e10, 6), "1.5e+10");
    }

    #[test]
    fn trailing_zeros_are_stripped_only_after_a_decimal_point() {
        assert_eq!(strip_trailing_zeros("1.500"), "1.5");
        assert_eq!(strip_trailing_zeros("1.000"), "1");
        assert_eq!(strip_trailing_zeros("0.0"), "0");
        assert_eq!(strip_trailing_zeros("100"), "100");
        assert_eq!(strip_trailing_zeros("10.10"), "10.1");
        assert_eq!(strip_trailing_zeros(""), "");
    }

    fn on_small_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn dropping_a_very_deep_chain_does_not_overflow_the_stack() {
        on_small_stack(|| {
            let mut node = float(1.0);
            for _ in 0..200_000 {
                node = parent(Op::LogicalNot, vec![node]);
            }
            drop(node);
        });
    }

    #[test]
    fn dropping_a_very_deep_chain_of_wide_nodes_does_not_overflow_the_stack() {
        on_small_stack(|| {
            let mut node = float(1.0);
            for _ in 0..50_000 {
                node = parent(Op::Add, vec![float(2.0), node, float(3.0)]);
            }
            drop(node);
        });
    }

    #[test]
    fn dropping_a_very_wide_node_works() {
        let node = parent(Op::Add, (0..100_000).map(|_| float(1.0)).collect());
        assert_eq!(node.children.len(), 100_000);
        drop(node);
    }

    #[test]
    fn the_grouping_note_fits_in_the_padding_of_a_node() {
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<Node>(), 80);
        assert_eq!(
            Node::token(Op::Add, Payload::None, Span::new(0, 1)).below,
            TokenClasses::ALL,
            "unknown until a pass looks"
        );
    }

    #[test]
    fn nodes_that_differ_only_in_their_grouping_note_are_equal() {
        let mut noted = entity("x");
        noted.below = TokenClasses::NONE;
        assert_eq!(noted, entity("x"));
        let mut other = entity("x");
        other.span = Span::new(1, 2);
        assert_ne!(other, entity("x"));
    }

    #[test]
    fn dropping_a_node_after_taking_its_children_is_fine() {
        let mut node = parent(Op::Add, vec![float(1.0), float(2.0)]);
        let children = std::mem::take(&mut node.children);
        drop(node);
        assert_eq!(children.len(), 2);
    }
}
