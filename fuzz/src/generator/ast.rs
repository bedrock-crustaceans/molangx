//! The tree of a generated program and the `Arbitrary` code that draws it.

use super::pools::{
    CONTEXTS, HOST_MATH, MATH, MAX_DEPTH, MAX_STATEMENTS, MEMBERS, QUERIES, STRING_QUERIES,
    STRINGS, TEMPS, VARIABLES, math_arguments,
};
use arbitrary::{Arbitrary, Result, Unstructured};
use molangx::internals::reference_catalog;
use molangx::version::RawVersion;
use std::iter;

/// A generated program: a single expression, or a statement list (`a; b; return c;`).
#[derive(Clone, Debug, PartialEq)]
pub enum Program {
    /// A simple expression.
    Simple(Ex),
    /// A complex expression: statements, each followed by `;`.
    Complex(Vec<Stmt>),
}

/// A statement of a statement list or block.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// An expression statement.
    Expr(Ex),
    /// `return x`.
    Return(Ex),
    /// `break`.
    Break,
    /// `continue`.
    Continue,
}

/// A namespace of a variable read or write.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Ns {
    /// `variable.` / `v.`
    Entity,
    /// `temp.` / `t.`
    Temp,
    /// `context.` / `c.`
    Context,
}

/// A binary operator.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `??`
    Coalesce,
}

impl BinOp {
    /// Every operator, in the order the generator draws them.
    pub(super) const ALL: [Self; 13] = [
        Self::Add,
        Self::Sub,
        Self::Mul,
        Self::Div,
        Self::Lt,
        Self::Le,
        Self::Gt,
        Self::Ge,
        Self::Eq,
        Self::Ne,
        Self::And,
        Self::Or,
        Self::Coalesce,
    ];

    pub(super) fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::And => "&&",
            Self::Or => "||",
            Self::Coalesce => "??",
        }
    }

    /// A comparison or a logical operator: versions before 6 group those in another order.
    pub(super) fn is_chained(self) -> bool {
        matches!(
            self,
            Self::Lt | Self::Le | Self::Gt | Self::Ge | Self::Eq | Self::Ne | Self::And | Self::Or
        )
    }
}

/// A variable with an optional member path: indices into the name pools.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    /// The namespace.
    pub ns: Ns,
    /// Index of the name in its pool.
    pub name: u8,
    /// Member names (indices into the member pool).
    pub members: Vec<u8>,
}

impl Ns {
    /// The pool of the names in this namespace.
    pub(super) fn names(self) -> &'static [&'static str] {
        match self {
            Self::Entity => VARIABLES,
            Self::Temp => TEMPS,
            Self::Context => CONTEXTS,
        }
    }
}

impl Var {
    /// A variable without members.
    pub(super) fn plain(ns: Ns, name: u8) -> Self {
        Self {
            ns,
            name,
            members: Vec::new(),
        }
    }
}

/// A generated expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Ex {
    /// A number literal (index into the pool; a pool miss is the integer itself).
    Num(u8),
    /// `true` / `false`.
    Bool(bool),
    /// A string literal (index into the pool).
    Str(u8),
    /// `this`.
    This,
    /// A variable read.
    Var(Var),
    /// `query.<name>(args)` (index into the query pool).
    Query(u8, Vec<Ex>),
    /// `math.<name>(args)` (index into the math pool).
    Math(u8, Vec<Ex>),
    /// Unary minus.
    Neg(Box<Ex>),
    /// `!`.
    Not(Box<Ex>),
    /// A binary operator.
    Bin(BinOp, Box<Ex>, Box<Ex>),
    /// `c ? a : b`, or `c ? a` without the else branch.
    Cond(Box<Ex>, Box<Ex>, Option<Box<Ex>>),
    /// `target = value`.
    Assign(Var, Box<Ex>),
    /// `a->b`.
    Arrow(Box<Ex>, Box<Ex>),
    /// `{ statements }`.
    Block(Vec<Stmt>),
    /// `loop(count, { statements })`.
    Loop(Box<Ex>, Vec<Stmt>),
    /// `for_each(variable, array, { statements })`.
    ForEach(Var, Box<Ex>, Vec<Stmt>),
}

impl<'a> Arbitrary<'a> for Program {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        Self::generate(u, None)
    }
}

impl Program {
    /// A program drawn from `u` for a compile at the raw version `version` (`None`: a version that
    /// resolves the standard queries).
    ///
    /// Valid by construction, except that about one program in 16 is *loose* and ignores what each
    /// grammar position accepts, so the compiler's rejection paths are generated too.
    ///
    /// # Errors
    ///
    /// Never: an exhausted buffer draws the first value of every range.
    pub fn generate(u: &mut Unstructured<'_>, version: Option<i16>) -> Result<Self> {
        // `ratio` on an exhausted buffer is true: the default is a strict program.
        let loose = !u.ratio(15, 16)?;
        let maker = Gen::new(loose, version.unwrap_or(13));
        Ok(if u.ratio(1, 3)? {
            // Mostly an expression, now and then a statement form (the printer closes it with `;`).
            if u.ratio(3, 4)? {
                Self::Simple(maker.ex(u, 0, false, Want::Val)?)
            } else {
                let pick = u.int_in_range(5..=23u8)?;
                Self::Simple(maker.form(u, 0, false, pick)?)
            }
        } else {
            Self::Complex(maker.list(u, 0, false)?)
        })
    }
}

/// What an expression position accepts (from version 3 the compiler rejects the others).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Want {
    /// A number: an operand of arithmetic, of a comparison other than `==` and `!=`, of `&&`, `||`,
    /// `!` or unary minus, an argument of a `math.*` function, the count of a loop. No string,
    /// assignment, loop, block, `for_each` or string-valued query.
    Num,
    /// Any value: additionally strings, string-valued queries and (parenthesised) assignments. The
    /// value of an assignment or a `return`, an argument of a query, an operand of `==` and `!=`, a
    /// branch of `?:`.
    Val,
    /// Anything, loops and blocks included: loose programs.
    Any,
}

impl Want {
    /// What a branch of `?:` or the right side of `??` accepts where the whole accepts `self`.
    fn branch(self) -> Self {
        if self == Self::Num {
            Self::Num
        } else {
            Self::Val
        }
    }
}

/// An index into a pool of `len` entries (at most 256).
fn pool_index(u: &mut Unstructured<'_>, len: usize) -> Result<u8> {
    Ok(u8::try_from(u.choose_index(len)?).unwrap_or(0))
}

/// The state of the generation of one program.
struct Gen {
    /// Ignore what the positions accept: the rejections of the compiler are generated too.
    loose: bool,
    /// The indices into [`QUERIES`] the compile version resolves.
    queries: Vec<u8>,
}

impl Gen {
    fn new(loose: bool, version: i16) -> Self {
        let catalog = reference_catalog::catalog();
        let queries = QUERIES
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| {
                catalog.get_suffix(name).is_some_and(|decl| {
                    decl.resolve(
                        RawVersion(version),
                        &reference_catalog::ADMISSION,
                        reference_catalog::EXPERIMENTS,
                    )
                    .is_some()
                })
            })
            .map(|(i, _)| u8::try_from(i).unwrap_or(0))
            .collect();
        Self { loose, queries }
    }

    fn list(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool) -> Result<Vec<Stmt>> {
        let count = u.int_in_range(1..=MAX_STATEMENTS)?;
        let mut list = Vec::with_capacity(count);
        for _ in 0..count {
            if u.is_empty() {
                break;
            }
            let stmt = self.stmt(u, depth, in_loop)?;
            let ends = matches!(stmt, Stmt::Return(_) | Stmt::Break | Stmt::Continue);
            list.push(stmt);
            // A statement after one of these is unreachable, which the compiler rejects.
            if ends && !self.loose {
                break;
            }
        }
        if list.is_empty() {
            list.push(Stmt::Expr(Ex::Num(1)));
        }
        Ok(list)
    }

    fn stmt(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool) -> Result<Stmt> {
        let pick = u.int_in_range(0..=23u8)?;
        Ok(match pick {
            0..=2 => Stmt::Return(self.ex(u, depth + 1, in_loop, Want::Val)?),
            // Mostly inside loops; now and then at the root, where `break;` logs.
            3 if in_loop || u.ratio(1, 4)? => Stmt::Break,
            4 if in_loop || u.ratio(1, 4)? => Stmt::Continue,
            _ => Stmt::Expr(self.form(u, depth, in_loop, pick)?),
        })
    }

    /// The expression of a statement: an assignment, a loop, a block or a conditional of blocks, or
    /// a value.
    fn form(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool, pick: u8) -> Result<Ex> {
        Ok(match pick {
            5..=10 => self.assign(u, depth, in_loop)?,
            // Loops and blocks are statements far more often than operands.
            11..=13 if depth < MAX_DEPTH => Ex::Loop(
                Box::new(self.ex(u, depth + 1, in_loop, Want::Num)?),
                self.list(u, depth + 1, true)?,
            ),
            14 if depth < MAX_DEPTH => self.for_each(u, depth, in_loop)?,
            15 if depth < MAX_DEPTH => Ex::Block(self.list(u, depth + 1, in_loop)?),
            16 if depth < MAX_DEPTH => Ex::Cond(
                Box::new(self.ex(u, depth + 1, in_loop, Want::Num)?),
                Box::new(Ex::Block(self.list(u, depth + 1, in_loop)?)),
                if u.arbitrary()? {
                    Some(Box::new(Ex::Block(self.list(u, depth + 1, in_loop)?)))
                } else {
                    None
                },
            ),
            _ => self.ex(u, depth, in_loop, Want::Val)?,
        })
    }

    /// A variable to read.
    fn var(u: &mut Unstructured<'_>) -> Result<Var> {
        let ns = match u.int_in_range(0..=9u8)? {
            0..=4 => Ns::Entity,
            5..=7 => Ns::Temp,
            _ => Ns::Context,
        };
        Self::var_in(u, ns)
    }

    /// A variable to assign.
    fn target(&self, u: &mut Unstructured<'_>) -> Result<Var> {
        let ns = match u.int_in_range(0..=9u8)? {
            0..=4 => Ns::Entity,
            5..=7 => Ns::Temp,
            // A context target is rejected or logged: only a loose program writes one, and rarely.
            _ if !self.loose || !u.ratio(1, 8)? => Ns::Temp,
            _ => Ns::Context,
        };
        Self::var_in(u, ns)
    }

    fn var_in(u: &mut Unstructured<'_>, ns: Ns) -> Result<Var> {
        let name = pool_index(u, ns.names().len())?;
        let mut members = Vec::new();
        while members.len() < 2 && u.ratio(1, 6)? {
            members.push(pool_index(u, MEMBERS.len())?);
        }
        Ok(Var { ns, name, members })
    }

    /// A variable without members: the left side of `??`.
    fn plain_var(u: &mut Unstructured<'_>) -> Result<Var> {
        let mut var = Self::var(u)?;
        var.members.clear();
        Ok(var)
    }

    /// An entity variable without members: the right side of `->`.
    fn entity_var(u: &mut Unstructured<'_>) -> Result<Var> {
        let mut var = Self::plain_var(u)?;
        if var.ns != Ns::Entity {
            var.ns = Ns::Entity;
            var.name %= u8::try_from(VARIABLES.len()).unwrap_or(1);
        }
        Ok(var)
    }

    /// The arguments of a call: the declared count, or with `vary` now and then one more or one
    /// less.
    fn args(
        &self,
        u: &mut Unstructured<'_>,
        depth: u32,
        in_loop: bool,
        usual: u8,
        vary: bool,
        want: Want,
    ) -> Result<Vec<Ex>> {
        let count = if vary {
            match u.int_in_range(0..=15u8)? {
                0 => usual.saturating_sub(1),
                1 => usual + 1,
                _ => usual,
            }
        } else {
            usual
        };
        (0..count)
            .map(|_| self.ex(u, depth + 1, in_loop, want))
            .collect()
    }

    /// A call of a query the version resolves; a string-valued one only where a string is accepted.
    /// `None` when there is none to call.
    fn query(
        &self,
        u: &mut Unstructured<'_>,
        depth: u32,
        in_loop: bool,
        want: Want,
    ) -> Result<Option<Ex>> {
        let candidates: Vec<u8> = self
            .queries
            .iter()
            .copied()
            .filter(|q| want != Want::Num || !STRING_QUERIES.contains(&QUERIES[usize::from(*q)].0))
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        let q = candidates[u.choose_index(candidates.len())?];
        // The compiler only warns of a query called with another count of arguments.
        let vary = self.loose || u.ratio(1, 16)?;
        let usual = QUERIES[usize::from(q)].1;
        Ok(Some(Ex::Query(
            q,
            self.args(u, depth, in_loop, usual, vary, Want::Val)?,
        )))
    }

    fn leaf(u: &mut Unstructured<'_>, want: Want) -> Result<Ex> {
        Ok(match u.int_in_range(0..=9u8)? {
            0..=2 => Ex::Num(u.arbitrary()?),
            3 if want == Want::Num => Ex::Num(u.arbitrary()?),
            3 => Ex::Str(pool_index(u, STRINGS.len())?),
            4 => Ex::This,
            5 => Ex::Bool(u.arbitrary()?),
            _ => Ex::Var(Self::var(u)?),
        })
    }

    /// `for_each(v.e | t.e, array, { … })`: the array is usually an actor array.
    fn for_each(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool) -> Result<Ex> {
        let (ns, name) = *u.choose(&[(Ns::Entity, 5), (Ns::Temp, 3)])?;
        let array = if u.ratio(3, 4)? {
            let (ns, name) = *u.choose(&[(Ns::Context, 2), (Ns::Entity, 6)])?;
            Ex::Var(Var::plain(ns, name))
        } else {
            self.ex(u, depth + 1, in_loop, Want::Val)?
        };
        Ok(Ex::ForEach(
            Var::plain(ns, name),
            Box::new(array),
            self.list(u, depth + 1, true)?,
        ))
    }

    fn ex(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool, want: Want) -> Result<Ex> {
        let want = if self.loose { Want::Any } else { want };
        if Self::leaf_only(u, depth)? {
            return Self::leaf(u, want);
        }
        Ok(match u.int_in_range(0..=31u8)? {
            0..=7 => Self::leaf(u, want)?,
            8..=13 => self.binary(u, depth, in_loop, want)?,
            14..=16 => self.math(u, depth, in_loop)?,
            17 => match self.query(u, depth, in_loop, want)? {
                Some(query) => query,
                None => Self::leaf(u, want)?,
            },
            18 => Ex::Neg(Box::new(self.ex(u, depth + 1, in_loop, Want::Num)?)),
            19 => Ex::Not(Box::new(self.ex(u, depth + 1, in_loop, Want::Num)?)),
            20..=21 => self.conditional(u, depth, in_loop, want)?,
            22..=23 if want != Want::Num => self.assign(u, depth, in_loop)?,
            24..=25 => self.arrow(u, depth, in_loop, want)?,
            // Blocks and loops as operands are rare (a loop's value is not usable from version 3).
            26 if want == Want::Any && u.ratio(1, 4)? => {
                Ex::Block(self.list(u, depth + 1, false)?)
            }
            27 if want == Want::Any && u.ratio(1, 4)? => Ex::Loop(
                Box::new(self.ex(u, depth + 1, in_loop, Want::Any)?),
                self.list(u, depth + 1, true)?,
            ),
            28 if want == Want::Any && u.ratio(1, 4)? => self.for_each(u, depth, in_loop)?,
            _ => Ex::Var(Self::var(u)?),
        })
    }

    /// Whether the expression at `depth` is a leaf.
    fn leaf_only(u: &mut Unstructured<'_>, depth: u32) -> Result<bool> {
        // Deeper levels are more and more often leaves, so most programs stay short.
        Ok(depth >= MAX_DEPTH || u.is_empty() || (depth > 0 && u.ratio(depth, MAX_DEPTH + 1)?))
    }

    /// A call of a `math.*` function with numbers for arguments, as many as it takes (with `loose`,
    /// now and then one more or one less). One call in four is of a host function, so the volatile
    /// one is common.
    fn math(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool) -> Result<Ex> {
        let f = if u.ratio(1, 4)? {
            HOST_MATH + u.choose_index(MATH.len() - HOST_MATH)?
        } else {
            u.choose_index(MATH.len())?
        };
        let count = u.int_in_range(math_arguments(f))?;
        Ok(Ex::Math(
            u8::try_from(f).unwrap_or(0),
            self.args(u, depth, in_loop, count, self.loose, Want::Num)?,
        ))
    }

    /// `c ? a : b` or `c ? a`, with a number for the condition.
    fn conditional(
        &self,
        u: &mut Unstructured<'_>,
        depth: u32,
        in_loop: bool,
        want: Want,
    ) -> Result<Ex> {
        let condition = self.ex(u, depth + 1, in_loop, Want::Num)?;
        let then = self.ex(u, depth + 1, in_loop, want.branch())?;
        let otherwise = if u.ratio(3, 4)? {
            Some(Box::new(self.ex(u, depth + 1, in_loop, want.branch())?))
        } else {
            None
        };
        Ok(Ex::Cond(Box::new(condition), Box::new(then), otherwise))
    }

    fn assign(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool) -> Result<Ex> {
        let target = self.target(u)?;
        Ok(Ex::assign(
            target,
            self.ex(u, depth + 1, in_loop, Want::Val)?,
        ))
    }

    fn binary(
        &self,
        u: &mut Unstructured<'_>,
        depth: u32,
        in_loop: bool,
        want: Want,
    ) -> Result<Ex> {
        let op = *u.choose(&BinOp::ALL)?;
        let (left, right) = match op {
            // `==` and `!=` compare anything.
            BinOp::Eq | BinOp::Ne => (Want::Val, Want::Val),
            BinOp::Coalesce => {
                // A variable on the left; on the right a value, now and then a block (`v.x ?? {
                // break; }`).
                let left = if self.loose {
                    self.ex(u, depth + 1, in_loop, Want::Any)?
                } else {
                    Ex::Var(Self::plain_var(u)?)
                };
                let right = if want != Want::Num && u.ratio(1, 6)? {
                    Ex::Block(self.list(u, depth + 1, in_loop)?)
                } else {
                    self.ex(u, depth + 1, in_loop, want.branch())?
                };
                return Ok(Ex::Bin(op, Box::new(left), Box::new(right)));
            }
            _ => (Want::Num, Want::Num),
        };
        Ok(Ex::Bin(
            op,
            Box::new(self.ex(u, depth + 1, in_loop, left)?),
            Box::new(self.ex(u, depth + 1, in_loop, right)?),
        ))
    }

    fn arrow(&self, u: &mut Unstructured<'_>, depth: u32, in_loop: bool, want: Want) -> Result<Ex> {
        // The left side is usually something that can be an actor: `c.other`, `v.e`, an unset
        // context, a number. Never another `->`.
        let left = if u.ratio(2, 3)? {
            let (ns, name) = *u.choose(&[
                (Ns::Context, 0),
                (Ns::Entity, 5),
                (Ns::Context, 3),
                (Ns::Entity, 0),
            ])?;
            Ex::Var(Var::plain(ns, name))
        } else if self.loose {
            self.ex(u, depth + 1, in_loop, Want::Any)?
        } else {
            Self::leaf(u, Want::Num)?
        };
        // The right side is an entity variable or a query.
        let right = if self.loose {
            self.ex(u, depth + 1, in_loop, Want::Any)?
        } else if u.ratio(3, 4)? {
            Ex::Var(Self::entity_var(u)?)
        } else {
            match self.query(u, depth + 1, in_loop, want)? {
                Some(query) => query,
                None => Ex::Var(Self::entity_var(u)?),
            }
        };
        Ok(Ex::Arrow(Box::new(left), Box::new(right)))
    }
}

impl Ex {
    /// An assignment, never of a variable into a member of itself (`v.a.b = v.a`): repeated in a
    /// loop that nests a struct one level per iteration, an unbounded value (dropping it recurses
    /// once per level). The step budgets bound the rest.
    fn assign(target: Var, value: Self) -> Self {
        let own_root = matches!(&value, Self::Var(v) if !target.members.is_empty() && v.ns == target.ns && v.name == target.name);
        Self::Assign(
            target,
            Box::new(if own_root { Self::Num(1) } else { value }),
        )
    }

    /// The expressions directly under this one, those of its statements included, in source order.
    pub(super) fn children(&self) -> Vec<&Self> {
        match self {
            Self::Num(_) | Self::Bool(_) | Self::Str(_) | Self::This | Self::Var(_) => Vec::new(),
            Self::Query(_, args) | Self::Math(_, args) => args.iter().collect(),
            Self::Neg(a) | Self::Not(a) | Self::Assign(_, a) => vec![a],
            Self::Bin(_, a, b) | Self::Arrow(a, b) => vec![a, b],
            Self::Cond(c, a, b) => [Some(&**c), Some(&**a), b.as_deref()]
                .into_iter()
                .flatten()
                .collect(),
            Self::Block(list) => list.iter().filter_map(Stmt::expr).collect(),
            Self::Loop(count, list) | Self::ForEach(_, count, list) => iter::once(&**count)
                .chain(list.iter().filter_map(Stmt::expr))
                .collect(),
        }
    }
}

impl Stmt {
    /// The expression of an expression statement or a `return`.
    pub(super) fn expr(&self) -> Option<&Ex> {
        match self {
            Self::Expr(ex) | Self::Return(ex) => Some(ex),
            Self::Break | Self::Continue => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_support::*;
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn an_empty_buffer_generates_the_number_zero() {
        assert_eq!(program_of(&[]), simple(n(0)));
        assert_eq!(
            Gen::new(false, 13)
                .ex(&mut Unstructured::new(&[]), 0, false, Want::Val)
                .expect("total"),
            n(0)
        );
    }

    #[test]
    fn an_empty_buffer_makes_a_statement_list_of_one_expression() {
        let list = Gen::new(false, 13)
            .list(&mut Unstructured::new(&[]), 0, false)
            .expect("total");
        assert_eq!(list, [Stmt::Expr(n(1))]);
    }

    #[test]
    fn a_statement_list_is_never_empty_and_never_longer_than_the_maximum() {
        for seed in 0..300 {
            let data = buffer(seed, 1 + (seed as usize % 40));
            let list = Gen::new(seed % 4 == 0, 13)
                .list(&mut Unstructured::new(&data), 0, seed % 2 == 0)
                .expect("total");
            assert!(
                (1..=MAX_STATEMENTS).contains(&list.len()),
                "seed {seed}: {}",
                list.len()
            );
        }
        for byte in 0..=255u8 {
            let list = Gen::new(false, 13)
                .list(&mut Unstructured::new(&[byte; 64]), 0, true)
                .expect("total");
            assert!((1..=MAX_STATEMENTS).contains(&list.len()), "byte {byte}");
        }
    }

    #[test]
    fn break_and_continue_are_frequent_in_loops_and_rare_outside() {
        let (mut inside, mut outside) = (0, 0);
        for seed in 0..600 {
            let data = buffer(seed, 20);
            if matches!(
                Gen::new(false, 13).stmt(&mut Unstructured::new(&data), 0, true),
                Ok(Stmt::Break | Stmt::Continue)
            ) {
                inside += 1;
            }
            if matches!(
                Gen::new(false, 13).stmt(&mut Unstructured::new(&data), 0, false),
                Ok(Stmt::Break | Stmt::Continue)
            ) {
                outside += 1;
            }
        }
        assert!(
            inside > outside,
            "{inside} inside a loop, {outside} outside"
        );
        assert!(inside > 0 && outside > 0);
    }

    #[test]
    fn at_the_maximum_depth_an_expression_is_a_leaf() {
        for seed in 0..300 {
            let data = buffer(seed, 64);
            let ex = Gen::new(false, 13)
                .ex(&mut Unstructured::new(&data), MAX_DEPTH, false, Want::Val)
                .expect("total");
            assert!(
                matches!(
                    ex,
                    Ex::Num(_) | Ex::Str(_) | Ex::This | Ex::Bool(_) | Ex::Var(_)
                ),
                "seed {seed}: {ex:?}"
            );
        }
    }

    #[test]
    fn an_assignment_never_stores_a_variable_into_a_member_of_itself() {
        let target = var(Ns::Entity, 0, &[0]);
        assert_eq!(
            Ex::assign(target.clone(), Ex::Var(vx())),
            Ex::Assign(target.clone(), b(n(1)))
        );
        // Another name, another namespace, or no member path: the value is kept.
        let other_name = Ex::Var(var(Ns::Entity, 1, &[]));
        assert_eq!(
            Ex::assign(target.clone(), other_name.clone()),
            Ex::Assign(target.clone(), b(other_name))
        );
        let other_ns = Ex::Var(var(Ns::Temp, 0, &[]));
        assert_eq!(
            Ex::assign(target.clone(), other_ns.clone()),
            Ex::Assign(target.clone(), b(other_ns))
        );
        let plain = var(Ns::Entity, 0, &[]);
        assert_eq!(
            Ex::assign(plain.clone(), Ex::Var(vx())),
            Ex::Assign(plain, b(Ex::Var(vx()))),
            "v.x = v.x is allowed"
        );
        // The rule looks at the root only: a member path on the value side is replaced as well.
        let deep = Ex::Var(var(Ns::Entity, 0, &[1]));
        assert_eq!(
            Ex::assign(target.clone(), deep),
            Ex::Assign(target.clone(), b(n(1)))
        );
        // Any other kind of value is kept.
        assert_eq!(
            Ex::assign(target.clone(), n(5)),
            Ex::Assign(target, b(n(5)))
        );
    }

    /// What a generated program contains, for the bounds below. `deepest` is the generator's depth
    /// rebuilt from the tree: an expression statement, and a block directly under a conditional or
    /// on the right of `??`, are made at their parent's depth.
    struct Shape {
        deepest: u32,
        longest_list: usize,
        shortest_list: usize,
        most_members: usize,
        problems: Vec<String>,
        kinds: BTreeSet<&'static str>,
    }

    impl Shape {
        fn of(program: &Program) -> Self {
            let mut shape = Self {
                deepest: 0,
                longest_list: 0,
                shortest_list: usize::MAX,
                most_members: 0,
                problems: Vec::new(),
                kinds: BTreeSet::new(),
            };
            match program {
                Program::Simple(ex) => shape.ex(ex, 0),
                Program::Complex(list) => shape.list(list, 0),
            }
            shape
        }

        fn var(&mut self, var: &Var) {
            if usize::from(var.name) >= var.ns.names().len() {
                self.problems.push(format!(
                    "name {} of {:?} is out of its pool",
                    var.name, var.ns
                ));
            }
            if var.members.iter().any(|m| usize::from(*m) >= MEMBERS.len()) {
                self.problems
                    .push(format!("a member of {var:?} is out of its pool"));
            }
            self.most_members = self.most_members.max(var.members.len());
        }

        fn list(&mut self, list: &[Stmt], depth: u32) {
            self.longest_list = self.longest_list.max(list.len());
            self.shortest_list = self.shortest_list.min(list.len());
            self.deepest = self.deepest.max(depth);
            for stmt in list {
                match stmt {
                    Stmt::Expr(ex) => self.ex(ex, depth),
                    Stmt::Return(ex) => self.ex(ex, depth + 1),
                    Stmt::Break | Stmt::Continue => {}
                }
            }
        }

        fn arguments(&mut self, args: &[Ex], usual: u8, what: &str, depth: u32) {
            if !(usual.saturating_sub(1)..=usual + 1).contains(&(args.len() as u8)) {
                self.problems.push(format!(
                    "{what} has {} arguments, usually {usual}",
                    args.len()
                ));
            }
            for arg in args {
                self.ex(arg, depth + 1);
            }
        }

        fn ex(&mut self, ex: &Ex, depth: u32) {
            self.kinds.insert(kind(ex));
            self.deepest = self.deepest.max(depth);
            match ex {
                Ex::Num(_) | Ex::Bool(_) | Ex::This => {}
                Ex::Str(i) => {
                    if usize::from(*i) >= STRINGS.len() {
                        self.problems.push(format!("string {i} is out of its pool"));
                    }
                }
                Ex::Var(v) => self.var(v),
                Ex::Query(q, args) => match QUERIES.get(usize::from(*q)) {
                    Some((name, usual)) => self.arguments(args, *usual, name, depth),
                    None => self.problems.push(format!("query {q} is out of its pool")),
                },
                Ex::Math(f, args) => match MATH.get(usize::from(*f)) {
                    Some((name, _)) => {
                        let counts = math_arguments(usize::from(*f));
                        if !(counts.start().saturating_sub(1)..=counts.end() + 1)
                            .contains(&(args.len() as u8))
                        {
                            self.problems.push(format!(
                                "{name} has {} arguments, not about {counts:?}",
                                args.len()
                            ));
                        }
                        for arg in args {
                            self.ex(arg, depth + 1);
                        }
                    }
                    None => self
                        .problems
                        .push(format!("math function {f} is out of its pool")),
                },
                Ex::Neg(a) | Ex::Not(a) => self.ex(a, depth + 1),
                Ex::Bin(_, a, c) => {
                    // A block on the right of `??` is made at the depth of its parent, like one
                    // under a conditional.
                    self.ex(a, depth + 1);
                    self.ex(c, depth + u32::from(!matches!(**c, Ex::Block(_))));
                }
                Ex::Cond(c, a, e) => {
                    self.ex(c, depth + 1);
                    for branch in iter::once(a).chain(e.as_ref()) {
                        // A block directly under a conditional is made at the conditional's depth.
                        self.ex(branch, depth + u32::from(!matches!(**branch, Ex::Block(_))));
                    }
                }
                Ex::Assign(target, value) => {
                    self.var(target);
                    self.ex(value, depth + 1);
                }
                Ex::Arrow(a, c) => {
                    self.ex(a, depth + 1);
                    self.ex(c, depth + 1);
                }
                Ex::Block(list) => self.list(list, depth + 1),
                Ex::Loop(count, list) => {
                    self.ex(count, depth + 1);
                    self.list(list, depth + 1);
                }
                Ex::ForEach(v, array, list) => {
                    self.var(v);
                    self.ex(array, depth + 1);
                    self.list(list, depth + 1);
                }
            }
        }
    }

    fn kind(ex: &Ex) -> &'static str {
        match ex {
            Ex::Num(_) => "num",
            Ex::Bool(_) => "bool",
            Ex::Str(_) => "str",
            Ex::This => "this",
            Ex::Var(_) => "var",
            Ex::Query(..) => "query",
            Ex::Math(..) => "math",
            Ex::Neg(_) => "neg",
            Ex::Not(_) => "not",
            Ex::Bin(..) => "bin",
            Ex::Cond(..) => "cond",
            Ex::Assign(..) => "assign",
            Ex::Arrow(..) => "arrow",
            Ex::Block(_) => "block",
            Ex::Loop(..) => "loop",
            Ex::ForEach(..) => "for_each",
        }
    }

    #[test]
    fn shape_measures_what_it_claims_to() {
        // A guard for the helper the bounds below rely on.
        assert_eq!(Shape::of(&simple(n(1))).deepest, 0);
        assert_eq!(Shape::of(&simple(Ex::Neg(b(n(1))))).deepest, 1);
        assert_eq!(
            Shape::of(&simple(Ex::Bin(BinOp::Add, b(Ex::Neg(b(n(1)))), b(n(2))))).deepest,
            2
        );
        assert_eq!(Shape::of(&simple(Ex::Math(22, vec![]))).deepest, 0);
        assert_eq!(Shape::of(&simple(Ex::Math(0, vec![n(1)]))).deepest, 1);
        let nested = simple(Ex::Block(vec![Stmt::Expr(Ex::Loop(
            b(n(1)),
            vec![Stmt::Break],
        ))]));
        assert_eq!(
            Shape::of(&nested).deepest,
            2,
            "the block's list is one deep, the loop's body two"
        );
        let cond = simple(Ex::Cond(b(n(1)), b(Ex::Block(vec![Stmt::Break])), None));
        assert_eq!(
            Shape::of(&cond).deepest,
            1,
            "a block under a conditional shares its depth"
        );
        assert_eq!(
            Shape::of(&Program::Complex(vec![Stmt::Return(n(1))])).deepest,
            1
        );
        let bad = Shape::of(&simple(Ex::Var(var(Ns::Temp, 9, &[7]))));
        assert_eq!(bad.problems.len(), 2);
        assert_eq!(
            Shape::of(&Program::Complex(vec![Stmt::Break; 4])).longest_list,
            4
        );
        assert_eq!(
            Shape::of(&simple(Ex::Query(0, vec![n(1), n(2), n(3), n(4)])))
                .problems
                .len(),
            1,
            "too many arguments"
        );
    }

    #[test]
    fn generated_programs_stay_inside_the_pools_lists_and_depth() {
        let mut deepest = 0;
        let mut kinds = BTreeSet::new();
        for seed in 0..600 {
            let data = buffer(seed, 16 + (seed as usize % 7) * 48);
            let program = program_of(&data);
            let shape = Shape::of(&program);
            assert!(
                shape.problems.is_empty(),
                "seed {seed}: {:?}",
                shape.problems
            );
            // No expression is generated at MAX_DEPTH or deeper; the leaves under a statement at
            // MAX_DEPTH are one more.
            assert!(
                shape.deepest <= MAX_DEPTH + 1,
                "seed {seed}: depth {} {program:?}",
                shape.deepest
            );
            assert!(
                shape.longest_list <= MAX_STATEMENTS,
                "seed {seed}: a list of {}",
                shape.longest_list
            );
            assert!(shape.shortest_list >= 1, "seed {seed}");
            assert!(shape.most_members <= 2, "seed {seed}");
            let text = program.canonical();
            assert!(text.is_ascii() && !text.is_empty(), "seed {seed}");
            deepest = deepest.max(shape.deepest);
            kinds.extend(shape.kinds);
        }
        assert!(deepest >= 4, "generation hardly nests: {deepest}");
        let all = [
            "num", "bool", "str", "this", "var", "query", "math", "neg", "not", "bin", "cond",
            "assign", "arrow", "block", "loop", "for_each",
        ];
        for expected in all {
            assert!(
                kinds.contains(expected),
                "{expected} is never generated: {kinds:?}"
            );
        }
    }

    #[test]
    fn generation_is_total_and_deterministic_for_every_byte_value_and_length() {
        for length in [0usize, 1, 2, 3, 8, 64, 512] {
            for byte in 0..=255u8 {
                let data = vec![byte; length];
                let first = Program::arbitrary(&mut Unstructured::new(&data))
                    .unwrap_or_else(|e| panic!("byte {byte} x {length}: {e}"));
                let again = Program::arbitrary(&mut Unstructured::new(&data)).expect("total");
                assert_eq!(first, again, "byte {byte} x {length}");
                let shape = Shape::of(&first);
                assert!(
                    shape.problems.is_empty(),
                    "byte {byte} x {length}: {:?}",
                    shape.problems
                );
                assert!(
                    shape.deepest <= MAX_DEPTH + 1,
                    "byte {byte} x {length}: depth {}",
                    shape.deepest
                );
                assert!(
                    shape.longest_list <= MAX_STATEMENTS,
                    "byte {byte} x {length}"
                );
            }
        }
    }

    /// The positions of a strict program that need a number hold one: the operands of arithmetic,
    /// comparisons other than `==` and `!=`, `&&`, `||`, `!` and unary minus, and the arguments of
    /// `math.*` and loop counts.
    fn check_numeric_positions(ex: &Ex, problems: &mut Vec<String>) {
        fn number(ex: &Ex, what: &str, problems: &mut Vec<String>) {
            let bad = match ex {
                Ex::Str(_) | Ex::Assign(..) | Ex::Loop(..) | Ex::ForEach(..) | Ex::Block(_) => true,
                Ex::Query(q, _) => STRING_QUERIES.contains(&QUERIES[usize::from(*q)].0),
                _ => false,
            };
            if bad {
                problems.push(format!("{ex:?} is an operand of {what}"));
            }
        }
        match ex {
            Ex::Neg(a) | Ex::Not(a) => number(a, "a unary operator", problems),
            Ex::Bin(op, a, c) => {
                if !matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Coalesce) {
                    number(a, op.symbol(), problems);
                    number(c, op.symbol(), problems);
                }
                if *op == BinOp::Coalesce && !matches!(&**a, Ex::Var(v) if v.members.is_empty()) {
                    problems.push(format!("the left side of ?? is {a:?}"));
                }
            }
            Ex::Math(f, args) => {
                for arg in args {
                    number(arg, MATH[usize::from(*f)].0, problems);
                }
            }
            Ex::Cond(c, ..) => number(c, "a condition", problems),
            Ex::Arrow(a, c) => {
                if matches!(**a, Ex::Arrow(..)) {
                    problems.push("an arrow on the left of an arrow".to_owned());
                }
                if !matches!(&**c, Ex::Query(..))
                    && !matches!(&**c, Ex::Var(v) if v.ns == Ns::Entity && v.members.is_empty())
                {
                    problems.push(format!("the right side of -> is {c:?}"));
                }
            }
            Ex::Loop(count, _) | Ex::ForEach(_, count, _) => number(count, "a loop", problems),
            Ex::Num(_)
            | Ex::Bool(_)
            | Ex::Str(_)
            | Ex::This
            | Ex::Var(_)
            | Ex::Query(..)
            | Ex::Assign(..)
            | Ex::Block(_) => {}
        }
        for child in ex.children() {
            check_numeric_positions(child, problems);
        }
    }

    /// A statement list ends at its first `return`, `break` or `continue`.
    fn check_reachable(list: &[Stmt], problems: &mut Vec<String>) {
        if let Some((_, init)) = list.split_last() {
            for stmt in init {
                if matches!(stmt, Stmt::Return(_) | Stmt::Break | Stmt::Continue) {
                    problems.push(format!("{stmt:?} is followed by another statement"));
                }
            }
        }
    }

    /// Every statement list under `ex` ends at its first `return`, `break` or `continue`.
    fn check_nested_lists(ex: &Ex, problems: &mut Vec<String>) {
        if let Ex::Block(list) | Ex::Loop(_, list) | Ex::ForEach(_, _, list) = ex {
            check_reachable(list, problems);
        }
        for child in ex.children() {
            check_nested_lists(child, problems);
        }
    }

    /// The rules of a strict program `program` breaks: a number where one is needed, and in a
    /// statement list, no statement after the end of a list.
    fn rule_breaks(program: &Program) -> Vec<String> {
        let mut problems = Vec::new();
        match program {
            Program::Simple(ex) => check_numeric_positions(ex, &mut problems),
            Program::Complex(list) => {
                check_reachable(list, &mut problems);
                for ex in list.iter().filter_map(Stmt::expr) {
                    check_numeric_positions(ex, &mut problems);
                    check_nested_lists(ex, &mut problems);
                }
            }
        }
        problems
    }

    #[test]
    fn a_strict_program_puts_a_number_where_the_compiler_wants_one() {
        for version in [-2, 0, 13, 15] {
            let maker = Gen::new(false, version);
            let mut kinds = BTreeSet::new();
            for seed in 0..600 {
                let data = buffer(seed, 16 + (seed as usize % 7) * 48);
                let mut u = Unstructured::new(&data);
                let program = if seed % 2 == 0 {
                    Program::Simple(maker.ex(&mut u, 0, false, Want::Val).expect("total"))
                } else {
                    Program::Complex(maker.list(&mut u, 0, false).expect("total"))
                };
                let problems = rule_breaks(&program);
                assert!(
                    problems.is_empty(),
                    "version {version} seed {seed}: {problems:?} in {}",
                    program.canonical()
                );
                kinds.insert(Shape::of(&program).kinds);
            }
            // Strict does not mean plain: strings, assignments and the rest are still written where
            // allowed.
            let all: BTreeSet<&str> = kinds.into_iter().flatten().collect();
            for expected in [
                "str", "assign", "arrow", "cond", "loop", "block", "for_each", "bin", "math",
            ] {
                assert!(
                    all.contains(expected),
                    "version {version}: {expected} is never generated: {all:?}"
                );
            }
            assert_eq!(
                all.contains("query"),
                version == 0 || version == 13,
                "version {version}: queries only where the version resolves them"
            );
        }
    }

    /// Counts the `math.*` calls of an expression, checking the argument count of each against the
    /// pool.
    fn count_math_calls(ex: &Ex, seen: &mut u32) {
        if let Ex::Math(f, args) = ex {
            assert!(
                math_arguments(usize::from(*f)).contains(&(args.len() as u8)),
                "{}",
                MATH[usize::from(*f)].0
            );
            *seen += 1;
        }
        for child in ex.children() {
            count_math_calls(child, seen);
        }
    }

    #[test]
    fn a_strict_generator_writes_every_math_call_with_its_arity() {
        let maker = Gen::new(false, 13);
        let mut seen = 0;
        for seed in 0..400 {
            let data = buffer(seed, 200);
            let list = maker
                .list(&mut Unstructured::new(&data), 0, false)
                .expect("total");
            for ex in list.iter().filter_map(Stmt::expr) {
                count_math_calls(ex, &mut seen);
            }
        }
        assert!(seen > 100, "{seen} math calls");
    }

    #[test]
    fn a_loose_program_is_generated_now_and_then_and_breaks_the_rules() {
        // The one program in 16 that ignores what the positions accept: found among the arbitrary
        // programs by its unreachable statements, strings under arithmetic and arrows of arrows.
        let (mut loose, mut total) = (0, 0);
        for seed in 0..1600 {
            let data = buffer(seed, 200);
            total += 1;
            loose += usize::from(!rule_breaks(&program_of(&data)).is_empty());
        }
        // Not every loose program breaks a rule, and no strict one does.
        assert!(
            (20..=200).contains(&loose),
            "{loose} of {total} programs break a rule"
        );
    }

    #[test]
    fn the_depth_bound_is_reached_exactly() {
        // The bound is tight: some program reaches the maximum depth and none passes it.
        let reached = (0..2000u64)
            .map(|seed| Shape::of(&program_of(&buffer(seed, 200))).deepest)
            .max();
        assert_eq!(reached, Some(MAX_DEPTH + 1));
    }

    #[test]
    fn generation_is_deterministic_and_depends_on_the_bytes() {
        let mut distinct = HashSet::new();
        for seed in 0..100 {
            let data = buffer(seed, 80);
            assert_eq!(program_of(&data), program_of(&data));
            assert_eq!(case_of(&data), case_of(&data));
            distinct.insert(program_of(&data).canonical());
        }
        assert!(
            distinct.len() > 60,
            "{} different programs of 100",
            distinct.len()
        );
    }

    #[test]
    fn a_short_buffer_runs_out_into_leaves_rather_than_failing() {
        for length in 0..12 {
            for seed in 0..50 {
                let data = buffer(seed, length);
                let program = program_of(&data);
                assert!(!program.canonical().is_empty());
            }
        }
    }
}
