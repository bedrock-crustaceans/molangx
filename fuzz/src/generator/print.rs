//! Renders a generated [`Program`] as Molang text, drawing its structure and cosmetic choices
//! from two byte streams.

use super::ast::{BinOp, Ex, Ns, Program, Stmt, Var};
use super::pools::{MATH, MEMBERS, NUMBERS, QUERIES, STRINGS};
use super::style::{Choices, Style};

struct Printer<'s> {
    out: String,
    structure: Choices<'s>,
    cosmetic: Choices<'s>,
    /// Draw no choice: every structure choice is 0, one space between tokens, no case change.
    canonical: bool,
}

impl<'s> Printer<'s> {
    fn new(structure: &'s [u8], cosmetic: &'s [u8], canonical: bool) -> Self {
        Self {
            out: String::new(),
            structure: Choices::new(structure),
            cosmetic: Choices::new(cosmetic),
            canonical,
        }
    }
}

/// How tight an expression binds when it is an operand (the order of the grouping passes of the
/// parser): a larger number binds tighter.
const ATOM: u8 = 100;
/// More than anything binds: the operand is always parenthesised.
const ALWAYS: u8 = 101;
const ARROW: u8 = 90;
const UNARY: u8 = 80;
const COND: u8 = 20;
const NULL_COALESCING: u8 = 10;
const ASSIGN: u8 = 0;

/// The precedence of a binary operator.
fn binary_precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Div => 70,
        BinOp::Mul => 60,
        BinOp::Add | BinOp::Sub => 50,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 40,
        BinOp::Eq | BinOp::Ne => 35,
        BinOp::And => 30,
        BinOp::Or => 25,
        BinOp::Coalesce => NULL_COALESCING,
    }
}

fn precedence(ex: &Ex) -> u8 {
    match ex {
        Ex::Neg(_) | Ex::Not(_) => UNARY,
        Ex::Arrow(..) => ARROW,
        Ex::Bin(op, ..) => binary_precedence(*op),
        Ex::Cond(..) => COND,
        Ex::Assign(..) => ASSIGN,
        _ => ATOM,
    }
}

/// Whether the printed text has a `;` or an `=` in it: such an expression ends with `;`.
fn is_complex(ex: &Ex) -> bool {
    matches!(
        ex,
        Ex::Assign(..) | Ex::Block(_) | Ex::Loop(..) | Ex::ForEach(..)
    ) || ex.children().into_iter().any(is_complex)
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '\''
}

impl Printer<'_> {
    /// Appends a token, with the separator the cosmetic stream picks; adjacent word tokens always
    /// get at least one space. Letters outside strings take the case the stream picks.
    fn tok(&mut self, text: &str) {
        let separator = if self.canonical {
            " "
        } else {
            match self.cosmetic.next() % 16 {
                0..=5 => "",
                6..=10 => " ",
                11 => "  ",
                12 => "\t",
                13 => "\n",
                14 => "\r\n",
                _ => " \t ",
            }
        };
        let joins = self.out.chars().next_back().is_some_and(is_word)
            && text.chars().next().is_some_and(is_word);
        if !self.out.is_empty() {
            self.out.push_str(if separator.is_empty() && joins {
                " "
            } else {
                separator
            });
        }
        if text.starts_with('\'') || self.canonical {
            self.out.push_str(text);
            return;
        }
        for c in text.chars() {
            let flip = self.cosmetic.next().is_multiple_of(3);
            self.out.push(if flip && c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c
            });
        }
    }

    /// The next structure choice; 0 (every default) in the canonical form.
    fn structure(&mut self) -> u8 {
        if self.canonical {
            0
        } else {
            self.structure.next()
        }
    }

    fn number(&mut self, index: u8) {
        let value = NUMBERS
            .get(usize::from(index))
            .copied()
            .unwrap_or(f64::from(index));
        let choice = self.structure();
        let mut text = if value.fract() == 0.0 && choice % 4 == 1 {
            format!("{value:.1}")
        } else {
            format!("{value}")
        };
        if text.starts_with("0.") && choice % 8 == 2 {
            // `.5`
            text.remove(0);
        }
        if choice % 8 == 3 {
            text.push('f');
        } else if choice % 8 == 5 {
            text.push('F');
        }
        self.tok(&text);
    }

    fn var(&mut self, var: &Var) {
        let long = self.structure() % 4 == 1;
        let (short, full) = match var.ns {
            Ns::Entity => ("v.", "variable."),
            Ns::Temp => ("t.", "temp."),
            Ns::Context => ("c.", "context."),
        };
        let mut text = String::from(if long { full } else { short });
        text.push_str(
            var.ns
                .names()
                .get(usize::from(var.name))
                .copied()
                .unwrap_or("x"),
        );
        for member in &var.members {
            text.push('.');
            text.push_str(MEMBERS.get(usize::from(*member)).copied().unwrap_or("x"));
        }
        self.tok(&text);
    }

    fn call(&mut self, name: &str, args: &[Ex]) {
        self.tok(name);
        self.tok("(");
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.tok(",");
            }
            // `,` binds tighter than `=`: an assignment is parenthesised.
            self.operand(arg, ASSIGN + 1);
        }
        self.tok(")");
    }

    /// Whether a parenthesis the grammar needs is left out: now and then in a printed text (never
    /// in the canonical form), so the compiler's rejections of such texts are exercised too.
    fn omit_required(&mut self) -> bool {
        self.structure() % 64 == 63
    }

    /// Prints `ex` as an operand that binds at least as tight as `least`: in parentheses when it
    /// binds looser.
    fn operand(&mut self, ex: &Ex, least: u8) {
        if precedence(ex) < least && !self.omit_required() {
            self.tok("(");
            self.ex(ex);
            self.tok(")");
        } else {
            self.ex(ex);
        }
    }

    fn block(&mut self, statements: &[Stmt]) {
        self.tok("{");
        self.statements(statements);
        self.tok("}");
    }

    fn statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            match statement {
                Stmt::Expr(ex) => self.ex(ex),
                Stmt::Return(ex) => {
                    self.tok("return");
                    self.operand(ex, ASSIGN + 1);
                }
                Stmt::Break => self.tok("break"),
                Stmt::Continue => self.tok("continue"),
            }
            self.tok(";");
        }
    }

    fn ex(&mut self, ex: &Ex) {
        // A redundant pair of parentheses around an operand now and then.
        let wrap = !matches!(ex, Ex::Block(_)) && self.structure() % 8 == 7;
        if wrap {
            self.tok("(");
        }
        self.bare(ex);
        if wrap {
            self.tok(")");
        }
    }

    /// `ex` without the redundant parentheses [`Printer::ex`] adds around it now and then.
    fn bare(&mut self, ex: &Ex) {
        match ex {
            Ex::Num(i) => self.number(*i),
            Ex::Bool(b) => self.tok(if *b { "true" } else { "false" }),
            Ex::Str(i) => {
                let text = format!("'{}'", STRINGS.get(usize::from(*i)).copied().unwrap_or(""));
                self.tok(&text);
            }
            Ex::This => self.tok("this"),
            Ex::Var(var) => self.var(var),
            Ex::Query(q, args) => self.query(*q, args),
            Ex::Math(f, args) => self.math(*f, args),
            Ex::Neg(inner) => {
                self.tok("-");
                self.operand(inner, ARROW);
            }
            Ex::Not(inner) => {
                self.tok("!");
                self.operand(inner, ARROW);
            }
            Ex::Bin(op, a, b) => self.binary(*op, a, b),
            Ex::Cond(c, a, b) => self.conditional(c, a, b.as_deref()),
            Ex::Assign(var, value) => {
                self.var(var);
                self.tok("=");
                self.operand(value, ASSIGN + 1);
            }
            Ex::Arrow(a, b) => self.arrow(a, b),
            Ex::Block(statements) => self.block(statements),
            Ex::Loop(count, body) => self.counted_loop(count, body),
            Ex::ForEach(var, array, body) => self.for_each(var, array, body),
        }
    }

    fn query(&mut self, q: u8, args: &[Ex]) {
        let (name, _) = QUERIES[usize::from(q) % QUERIES.len()];
        let prefix = if self.structure() % 4 == 1 {
            "query."
        } else {
            "q."
        };
        let full = format!("{prefix}{name}");
        // `q.name()` is rejected: a call without arguments is written without parentheses.
        if args.is_empty() && !self.omit_required() {
            self.tok(&full);
        } else {
            self.call(&full, args);
        }
    }

    fn math(&mut self, f: u8, args: &[Ex]) {
        let (name, _) = MATH[usize::from(f) % MATH.len()];
        let full = format!("math.{name}");
        if args.is_empty() && name == "pi" {
            self.tok(&full);
        } else {
            self.call(&full, args);
        }
    }

    fn binary(&mut self, op: BinOp, a: &Ex, b: &Ex) {
        let level = binary_precedence(op);
        // Comparisons and logic are never written without parentheses inside each other: the
        // versions before 6 group them in another order (and `||` binds tighter than `&&` before
        // 6).
        let chained =
            |ex: &Ex| matches!(ex, Ex::Bin(inner, ..) if inner.is_chained()) && op.is_chained();
        let left = if op == BinOp::Coalesce {
            ATOM
        } else if chained(a) {
            ALWAYS
        } else {
            level
        };
        self.operand(a, left);
        self.tok(op.symbol());
        self.operand(b, if chained(b) { ALWAYS } else { level + 1 });
    }

    fn conditional(&mut self, c: &Ex, a: &Ex, b: Option<&Ex>) {
        self.operand(c, COND + 1);
        self.tok("?");
        self.operand(a, COND + 1);
        if let Some(b) = b {
            self.tok(":");
            self.operand(b, COND + 1);
        }
    }

    fn arrow(&mut self, a: &Ex, b: &Ex) {
        // `->` groups before the math functions: a math call beside it is written in parentheses.
        let side = |ex: &Ex| {
            if matches!(ex, Ex::Math(..)) {
                ALWAYS
            } else {
                ATOM
            }
        };
        self.operand(a, side(a));
        self.tok("->");
        self.operand(b, side(b));
    }

    fn counted_loop(&mut self, count: &Ex, body: &[Stmt]) {
        self.tok("loop");
        self.tok("(");
        self.operand(count, ASSIGN + 1);
        self.tok(",");
        self.block(body);
        self.tok(")");
    }

    fn for_each(&mut self, var: &Var, array: &Ex, body: &[Stmt]) {
        self.tok("for_each");
        self.tok("(");
        self.var(var);
        self.tok(",");
        self.operand(array, ASSIGN + 1);
        self.tok(",");
        self.block(body);
        self.tok(")");
    }
}

impl Program {
    /// The program as source text under `style`.
    pub fn source(&self, style: &Style) -> String {
        self.print(style, false)
    }

    /// The canonical source text: lower case, one space between tokens, no redundant
    /// parentheses, short namespaces.
    pub fn canonical(&self) -> String {
        self.print(&Style::default(), true)
    }

    fn print(&self, style: &Style, canonical: bool) -> String {
        let mut printer = Printer::new(&style.structure, &style.cosmetic, canonical);
        match self {
            Self::Simple(ex) => {
                printer.ex(ex);
                // A complex expression (one with an `=` or a `;`) ends with `;`.
                if is_complex(ex) && !printer.omit_required() {
                    printer.tok(";");
                }
            }
            Self::Complex(statements) => printer.statements(statements),
        }
        printer.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_support::*;

    /// The lower-cased text with the white space outside strings removed.
    fn squeezed(text: &str) -> String {
        let mut out = String::new();
        let mut in_string = false;
        for c in text.chars() {
            if c == '\'' {
                in_string = !in_string;
            }
            if in_string {
                out.push(c);
            } else if !c.is_whitespace() {
                out.push(c.to_ascii_lowercase());
            }
        }
        out
    }

    fn one_of_each_kind() -> Vec<(&'static str, Program)> {
        let block = vec![Stmt::Expr(n(1)), Stmt::Return(n(2))];
        vec![
            ("number", simple(n(1))),
            ("number from a pool miss", simple(n(200))),
            ("true", simple(Ex::Bool(true))),
            ("false", simple(Ex::Bool(false))),
            ("string", simple(Ex::Str(1))),
            ("this", simple(Ex::This)),
            ("entity variable", simple(Ex::Var(vx()))),
            (
                "temp variable with members",
                simple(Ex::Var(var(Ns::Temp, 3, &[0, 3]))),
            ),
            (
                "context variable",
                simple(Ex::Var(var(Ns::Context, 0, &[]))),
            ),
            (
                "query with arguments",
                simple(Ex::Query(0, vec![n(1), n(2)])),
            ),
            ("query without arguments", simple(Ex::Query(5, vec![]))),
            (
                "math with arguments",
                simple(Ex::Math(18, vec![n(1), n(2)])),
            ),
            ("math without arguments", simple(Ex::Math(22, vec![]))),
            ("negation", simple(Ex::Neg(b(n(1))))),
            ("not", simple(Ex::Not(b(n(1))))),
            ("binary", simple(Ex::Bin(BinOp::Mul, b(n(1)), b(n(2))))),
            (
                "conditional",
                simple(Ex::Cond(b(Ex::Var(vx())), b(n(1)), Some(b(n(2))))),
            ),
            (
                "conditional without else",
                simple(Ex::Cond(b(Ex::Var(vx())), b(n(1)), None)),
            ),
            (
                "assignment",
                Program::Complex(vec![Stmt::Expr(Ex::Assign(vx(), b(n(1))))]),
            ),
            (
                "arrow",
                simple(Ex::Arrow(
                    b(Ex::Var(var(Ns::Context, 0, &[]))),
                    b(Ex::Var(vx())),
                )),
            ),
            ("block", simple(Ex::Block(block.clone()))),
            (
                "loop",
                simple(Ex::Loop(b(n(3)), vec![Stmt::Break, Stmt::Continue])),
            ),
            (
                "for_each",
                simple(Ex::ForEach(
                    var(Ns::Temp, 3, &[]),
                    b(Ex::Var(var(Ns::Context, 2, &[]))),
                    block,
                )),
            ),
            (
                "statements",
                Program::Complex(vec![
                    Stmt::Expr(n(1)),
                    Stmt::Return(n(2)),
                    Stmt::Break,
                    Stmt::Continue,
                ]),
            ),
        ]
    }

    #[test]
    fn is_word_covers_names_numbers_and_strings_only() {
        for c in ['a', 'Z', '0', '9', '_', '.', '\''] {
            assert!(is_word(c), "{c:?}");
        }
        for c in [
            '+', '-', '(', ')', ' ', ';', ',', '?', ':', '=', '!', '<', '{', '}', '\n',
        ] {
            assert!(!is_word(c), "{c:?}");
        }
    }

    #[test]
    fn canonical_leaves() {
        assert_eq!(simple(n(1)).canonical(), "1");
        assert_eq!(simple(n(0)).canonical(), "0");
        assert_eq!(simple(n(7)).canonical(), "0.5");
        assert_eq!(
            simple(n(24)).canonical(),
            "300000000000000000000000000000000000000"
        );
        assert_eq!(
            simple(n(200)).canonical(),
            "200",
            "a pool miss is the integer itself"
        );
        assert_eq!(simple(Ex::Bool(true)).canonical(), "true");
        assert_eq!(simple(Ex::Bool(false)).canonical(), "false");
        assert_eq!(simple(Ex::Str(0)).canonical(), "'moo'");
        assert_eq!(
            simple(Ex::Str(3)).canonical(),
            "'Moo X'",
            "upper case is kept inside a string"
        );
        assert_eq!(simple(Ex::Str(2)).canonical(), "''");
        assert_eq!(
            simple(Ex::Str(200)).canonical(),
            "''",
            "a pool miss is the empty string"
        );
        assert_eq!(simple(Ex::This).canonical(), "this");
    }

    #[test]
    fn canonical_variables() {
        assert_eq!(simple(Ex::Var(vx())).canonical(), "v.x");
        assert_eq!(
            simple(Ex::Var(var(Ns::Entity, 10, &[]))).canonical(),
            "v.never"
        );
        assert_eq!(
            simple(Ex::Var(var(Ns::Temp, 3, &[0, 3]))).canonical(),
            "t.e.x.q"
        );
        assert_eq!(
            simple(Ex::Var(var(Ns::Context, 0, &[]))).canonical(),
            "c.other"
        );
        assert_eq!(
            simple(Ex::Var(var(Ns::Context, 3, &[1]))).canonical(),
            "c.missing.y"
        );
        // A pool miss falls back to `x`.
        assert_eq!(
            simple(Ex::Var(var(Ns::Entity, 200, &[200]))).canonical(),
            "v.x.x"
        );
    }

    #[test]
    fn canonical_operators_and_calls_have_one_space_between_tokens() {
        assert_eq!(
            simple(Ex::Bin(BinOp::Add, b(n(1)), b(n(2)))).canonical(),
            "1 + 2"
        );
        assert_eq!(
            simple(Ex::Bin(BinOp::Coalesce, b(Ex::Var(vx())), b(n(2)))).canonical(),
            "v.x ?? 2"
        );
        assert_eq!(simple(Ex::Neg(b(n(1)))).canonical(), "- 1");
        assert_eq!(simple(Ex::Not(b(n(1)))).canonical(), "! 1");
        assert_eq!(
            simple(Ex::Math(18, vec![n(1), n(2)])).canonical(),
            "math.max ( 1 , 2 )"
        );
        assert_eq!(simple(Ex::Math(22, vec![])).canonical(), "math.pi");
        assert_eq!(
            simple(Ex::Math(0, vec![])).canonical(),
            "math.abs ( )",
            "only pi is written without parentheses"
        );
        assert_eq!(simple(Ex::Query(5, vec![])).canonical(), "q.is_baby");
        assert_eq!(
            simple(Ex::Query(0, vec![n(1), n(2)])).canonical(),
            "q.log ( 1 , 2 )"
        );
        assert_eq!(simple(Ex::Query(0, vec![])).canonical(), "q.log");
        assert_eq!(
            simple(Ex::Cond(b(Ex::Var(vx())), b(n(1)), Some(b(n(2))))).canonical(),
            "v.x ? 1 : 2"
        );
        assert_eq!(
            simple(Ex::Cond(b(Ex::Var(vx())), b(n(1)), None)).canonical(),
            "v.x ? 1"
        );
        assert_eq!(
            simple(Ex::Assign(vx(), b(n(1)))).canonical(),
            "v.x = 1 ;",
            "an expression with an `=` ends with `;`"
        );
        assert_eq!(
            simple(Ex::Arrow(
                b(Ex::Var(var(Ns::Context, 0, &[]))),
                b(Ex::Var(vx()))
            ))
            .canonical(),
            "c.other -> v.x"
        );
    }

    #[test]
    fn canonical_blocks_loops_and_statements() {
        assert_eq!(
            simple(Ex::Block(vec![Stmt::Expr(n(1))])).canonical(),
            "{ 1 ; } ;",
            "an expression with a `;` ends with `;`"
        );
        assert_eq!(
            simple(Ex::Loop(b(n(3)), vec![Stmt::Break])).canonical(),
            "loop ( 3 , { break ; } ) ;"
        );
        assert_eq!(
            simple(Ex::ForEach(
                var(Ns::Temp, 3, &[]),
                b(Ex::Var(var(Ns::Context, 2, &[]))),
                vec![Stmt::Continue]
            ))
            .canonical(),
            "for_each ( t.e , c.arr , { continue ; } ) ;"
        );
        assert_eq!(
            Program::Complex(vec![Stmt::Return(n(1))]).canonical(),
            "return 1 ;"
        );
        assert_eq!(
            Program::Complex(vec![
                Stmt::Expr(Ex::Assign(vx(), b(n(1)))),
                Stmt::Break,
                Stmt::Continue,
                Stmt::Return(Ex::This)
            ])
            .canonical(),
            "v.x = 1 ; break ; continue ; return this ;"
        );
    }

    #[test]
    fn every_binary_operator_prints_its_own_symbol() {
        let printed: Vec<String> = BinOp::ALL
            .iter()
            .map(|&op| simple(Ex::Bin(op, b(n(1)), b(n(2)))).canonical())
            .collect();
        assert_eq!(
            printed,
            [
                "1 + 2", "1 - 2", "1 * 2", "1 / 2", "1 < 2", "1 <= 2", "1 > 2", "1 >= 2", "1 == 2",
                "1 != 2", "1 && 2", "1 || 2", "1 ?? 2",
            ]
        );
    }

    #[test]
    fn every_math_function_and_query_prints_under_its_name() {
        for (i, (name, arity)) in MATH.iter().enumerate() {
            let args: Vec<Ex> = (0..*arity).map(|_| n(1)).collect();
            let text = simple(Ex::Math(i as u8, args)).canonical();
            if *name == "pi" {
                assert_eq!(text, "math.pi");
            } else {
                let list = vec!["1"; usize::from(*arity)].join(" , ");
                assert_eq!(text, format!("math.{name} ( {list} )"));
            }
        }
        for (i, (name, _)) in QUERIES.iter().enumerate() {
            assert_eq!(
                simple(Ex::Query(i as u8, vec![])).canonical(),
                format!("q.{name}")
            );
        }
        // Indices past the pools wrap.
        assert_eq!(
            simple(Ex::Math(64, vec![n(1)])).canonical(),
            "math.abs ( 1 )"
        );
        assert_eq!(simple(Ex::Query(11, vec![])).canonical(), "q.log");
    }

    #[test]
    fn the_canonical_form_ignores_the_style() {
        for (name, program) in one_of_each_kind() {
            let text = program.canonical();
            assert_eq!(
                text,
                program.print(&style(&[1, 2, 3], &[4, 5, 6]), true),
                "{name}"
            );
            assert!(text.is_ascii() && !text.is_empty(), "{name}");
            assert_eq!(text, text.trim(), "{name}: no leading or trailing blank");
            assert!(!text.contains("  "), "{name}: one space between tokens");
        }
    }

    #[test]
    fn printing_is_deterministic() {
        for seed in 0..100 {
            let case = case_of(&buffer(seed, 80));
            let again = case.clone();
            assert_eq!(case.source(), again.source());
            assert_eq!(case.source(), case.source());
            assert_eq!(case.program.source(&case.style), case.source());
            assert_eq!(case.program.canonical(), again.program.canonical());
        }
    }

    #[test]
    fn the_default_style_prints_without_separators_in_upper_case() {
        // With empty streams every cosmetic draw is 0: no separator unless two words meet, and
        // every lower-case letter outside a string is flipped to upper case.
        assert_eq!(
            simple(Ex::Bin(BinOp::Add, b(n(1)), b(n(2)))).source(&Style::default()),
            "1+2"
        );
        assert_eq!(
            simple(Ex::Math(18, vec![n(1), n(2)])).source(&Style::default()),
            "MATH.MAX(1,2)"
        );
        assert_eq!(simple(Ex::Var(vx())).source(&Style::default()), "V.X");
        assert_eq!(
            simple(Ex::Str(3)).source(&Style::default()),
            "'Moo X'",
            "strings are never cased or spaced"
        );
        assert_eq!(
            Program::Complex(vec![Stmt::Return(n(1))]).source(&Style::default()),
            "RETURN 1;",
            "two words keep a space"
        );
        assert_eq!(simple(Ex::Bool(true)).source(&Style::default()), "TRUE");
    }

    #[test]
    fn the_structure_stream_picks_the_number_spelling() {
        // The first draw is the raw byte. An empty cosmetic stream upper-cases the suffix.
        let print = |index: u8, byte: u8| {
            let bytes = [byte];
            let mut p = Printer::new(&bytes, &[], false);
            p.number(index);
            p.out
        };
        assert_eq!(print(1, 0), "1");
        assert_eq!(
            print(1, 1),
            "1.0",
            "choice % 4 == 1 spells an integral value with a fraction"
        );
        assert_eq!(print(1, 3), "1F", "choice % 8 == 3 adds the suffix");
        assert_eq!(
            print(1, 5),
            "1.0F",
            "choice % 8 == 5 adds the upper-case suffix"
        );
        assert_eq!(print(1, 9), "1.0", "9 % 4 == 1 but 9 % 8 == 1: no suffix");
        assert_eq!(
            print(7, 2),
            ".5",
            "choice % 8 == 2 drops the leading zero of 0.5"
        );
        assert_eq!(print(7, 10), ".5");
        assert_eq!(
            print(7, 1),
            "0.5",
            "a fractional value keeps its spelling for % 4 == 1"
        );
        assert_eq!(print(7, 3), "0.5F");
        assert_eq!(print(1, 2), "1", "nothing to drop from an integer");
        assert_eq!(print(0, 2), "0", "'0' does not start with '0.'");
        assert_eq!(print(0, 1), "0.0");
        assert_eq!(
            print(200, 1),
            "200.0",
            "a pool miss is an integral value too"
        );
    }

    #[test]
    fn the_structure_stream_picks_the_namespace_spelling() {
        let print = |var: Var, byte: u8| {
            let bytes = [byte];
            let mut p = Printer::new(&bytes, &[], false);
            p.var(&var);
            p.out
        };
        assert_eq!(print(vx(), 0), "V.X");
        assert_eq!(print(vx(), 1), "VARIABLE.X");
        assert_eq!(print(vx(), 5), "VARIABLE.X");
        assert_eq!(print(vx(), 2), "V.X");
        assert_eq!(print(var(Ns::Temp, 0, &[]), 1), "TEMP.A");
        assert_eq!(print(var(Ns::Context, 0, &[1]), 1), "CONTEXT.OTHER.Y");
        assert_eq!(print(var(Ns::Context, 0, &[1]), 0), "C.OTHER.Y");
    }

    #[test]
    fn the_structure_stream_picks_the_query_prefix_and_the_call_form() {
        // Draw 1 picks `query.` (% 4 == 1); a query with no arguments takes a second draw: even is
        // bare.
        let print = |ex: Ex, structure: &[u8]| {
            let mut p = Printer::new(structure, &[], false);
            p.ex(&ex);
            p.out
        };
        // Draws: wrap (0), prefix, form (63 omits what the grammar needs: the empty call is
        // rejected).
        let baby = Ex::Query(5, vec![]);
        assert_eq!(print(baby.clone(), &stream_for(&[0, 0, 0])), "Q.IS_BABY");
        assert_eq!(
            print(baby.clone(), &stream_for(&[0, 1, 0])),
            "QUERY.IS_BABY"
        );
        assert_eq!(
            print(baby.clone(), &stream_for(&[0, 0, 1])),
            "Q.IS_BABY",
            "an ordinary draw writes the name alone"
        );
        assert_eq!(
            print(baby.clone(), &stream_for(&[0, 0, 63])),
            "Q.IS_BABY()",
            "draw 63 writes the empty call"
        );
        assert_eq!(print(baby, &stream_for(&[0, 1, 63])), "QUERY.IS_BABY()");
        // A query with arguments takes no second draw, so the next draw is the argument's wrap.
        let log = Ex::Query(0, vec![n(1)]);
        assert_eq!(print(log.clone(), &stream_for(&[0, 1])), "QUERY.LOG(1)");
        assert_eq!(print(log, &stream_for(&[0, 0, 7])), "Q.LOG((1))");
    }

    #[test]
    fn the_structure_stream_wraps_operands_in_redundant_parentheses() {
        let print = |ex: Ex, byte: u8| {
            let bytes = [byte];
            let mut p = Printer::new(&bytes, &[], false);
            p.ex(&ex);
            p.out
        };
        assert_eq!(print(n(1), 7), "(1)");
        assert_eq!(print(n(1), 15), "(1)");
        assert_eq!(print(n(1), 6), "1");
        assert_eq!(print(Ex::This, 7), "(THIS)");
        // A block is never wrapped.
        assert_eq!(print(Ex::Block(vec![Stmt::Break]), 7), "{BREAK;}");
    }

    #[test]
    fn an_operand_that_binds_looser_than_its_operator_is_parenthesised() {
        let x = || Ex::Var(vx());
        let add = |a: Ex, c: Ex| Ex::Bin(BinOp::Add, b(a), b(c));
        let mul = |a: Ex, c: Ex| Ex::Bin(BinOp::Mul, b(a), b(c));
        let assign = || Ex::Assign(vx(), b(n(1)));
        let cond = || Ex::Cond(b(x()), b(n(1)), Some(b(n(2))));
        // The left operand of the same level is not wrapped (the operators are left associative),
        // the right one is.
        assert_eq!(simple(add(add(x(), n(1)), n(2))).canonical(), "v.x + 1 + 2");
        assert_eq!(
            simple(add(x(), add(n(1), n(2)))).canonical(),
            "v.x + ( 1 + 2 )"
        );
        assert_eq!(
            simple(mul(add(x(), n(1)), n(2))).canonical(),
            "( v.x + 1 ) * 2"
        );
        assert_eq!(simple(add(x(), mul(n(1), n(2)))).canonical(), "v.x + 1 * 2");
        // Unary operators take an atom, a call or an arrow.
        assert_eq!(
            simple(Ex::Neg(b(add(x(), n(1))))).canonical(),
            "- ( v.x + 1 )"
        );
        assert_eq!(
            simple(Ex::Neg(b(Ex::Neg(b(n(1)))))).canonical(),
            "- ( - 1 )"
        );
        assert_eq!(
            simple(Ex::Not(b(Ex::Math(0, vec![n(1)])))).canonical(),
            "! math.abs ( 1 )"
        );
        assert_eq!(
            simple(Ex::Neg(b(Ex::Arrow(
                b(Ex::Var(var(Ns::Context, 0, &[]))),
                b(x())
            ))))
            .canonical(),
            "- c.other -> v.x"
        );
        // An assignment is never an operand without parentheses, nor is a conditional of an
        // operator.
        assert_eq!(simple(add(assign(), n(2))).canonical(), "( v.x = 1 ) + 2 ;");
        assert_eq!(simple(add(cond(), n(2))).canonical(), "( v.x ? 1 : 2 ) + 2");
        assert_eq!(
            simple(Ex::Cond(b(cond()), b(n(1)), None)).canonical(),
            "( v.x ? 1 : 2 ) ? 1"
        );
        assert_eq!(
            simple(Ex::Cond(
                b(x()),
                b(cond()),
                Some(b(Ex::Bin(BinOp::Coalesce, b(x()), b(n(2)))))
            ))
            .canonical(),
            "v.x ? ( v.x ? 1 : 2 ) : ( v.x ?? 2 )"
        );
        assert_eq!(
            simple(Ex::Assign(vx(), b(assign()))).canonical(),
            "v.x = ( v.x = 1 ) ;"
        );
        assert_eq!(
            simple(Ex::Assign(vx(), b(cond()))).canonical(),
            "v.x = v.x ? 1 : 2 ;",
            "an assignment is the loosest: its value needs no parentheses"
        );
        // `==` of two comparisons, and `&&` of an `||`, are written with parentheses whatever the
        // version groups.
        let lt = Ex::Bin(BinOp::Lt, b(x()), b(n(1)));
        assert_eq!(
            simple(Ex::Bin(BinOp::Eq, b(lt.clone()), b(n(1)))).canonical(),
            "( v.x < 1 ) == 1"
        );
        assert_eq!(
            simple(Ex::Bin(
                BinOp::And,
                b(Ex::Bin(BinOp::Or, b(x()), b(n(1)))),
                b(n(2))
            ))
            .canonical(),
            "( v.x || 1 ) && 2"
        );
        // `??` takes a variable on its left; whatever is on its right binds tighter than it.
        assert_eq!(
            simple(Ex::Bin(
                BinOp::Coalesce,
                b(x()),
                b(Ex::Bin(BinOp::Coalesce, b(x()), b(n(2))))
            ))
            .canonical(),
            "v.x ?? ( v.x ?? 2 )"
        );
        assert_eq!(
            simple(Ex::Bin(BinOp::Coalesce, b(x()), b(add(n(1), n(2))))).canonical(),
            "v.x ?? 1 + 2"
        );
        // The arguments of a call and the count of a loop: only an assignment is wrapped (`,` binds
        // tighter than `=`).
        assert_eq!(
            simple(Ex::Math(18, vec![assign(), cond()])).canonical(),
            "math.max ( ( v.x = 1 ) , v.x ? 1 : 2 ) ;"
        );
        assert_eq!(
            Program::Complex(vec![Stmt::Return(assign())]).canonical(),
            "return ( v.x = 1 ) ;"
        );
        assert_eq!(
            simple(Ex::Loop(b(assign()), vec![Stmt::Break])).canonical(),
            "loop ( ( v.x = 1 ) , { break ; } ) ;"
        );
    }

    #[test]
    fn the_parentheses_a_statement_needs_are_not_written_around_the_whole_statement() {
        let list = Program::Complex(vec![
            Stmt::Expr(Ex::Assign(vx(), b(Ex::Cond(b(n(1)), b(n(2)), None)))),
            Stmt::Expr(Ex::Cond(b(n(1)), b(Ex::Block(vec![Stmt::Break])), None)),
        ]);
        assert_eq!(list.canonical(), "v.x = 1 ? 2 ; 1 ? { break ; } ;");
    }

    #[test]
    fn an_expression_with_an_assignment_or_a_semicolon_ends_with_a_semicolon_and_no_other_does() {
        for (ex, ends) in [
            (Ex::Assign(vx(), b(n(1))), true),
            (Ex::Block(vec![Stmt::Break]), true),
            (Ex::Loop(b(n(1)), vec![Stmt::Break]), true),
            (
                Ex::ForEach(var(Ns::Temp, 3, &[]), b(n(1)), vec![Stmt::Break]),
                true,
            ),
            // Anywhere inside: an operand, a branch, an argument, an arrow.
            (
                Ex::Bin(BinOp::Add, b(n(1)), b(Ex::Assign(vx(), b(n(1))))),
                true,
            ),
            (Ex::Neg(b(Ex::Assign(vx(), b(n(1))))), true),
            (
                Ex::Cond(b(n(1)), b(n(1)), Some(b(Ex::Block(vec![Stmt::Break])))),
                true,
            ),
            (Ex::Math(0, vec![Ex::Assign(vx(), b(n(1)))]), true),
            (Ex::Query(0, vec![Ex::Block(vec![Stmt::Break])]), true),
            (Ex::Arrow(b(n(1)), b(Ex::Assign(vx(), b(n(1))))), true),
            (Ex::Cond(b(Ex::Assign(vx(), b(n(1)))), b(n(1)), None), true),
            (
                Ex::Bin(BinOp::Add, b(Ex::Assign(vx(), b(n(1)))), b(n(1))),
                true,
            ),
            (Ex::Not(b(Ex::Loop(b(n(1)), vec![Stmt::Break]))), true),
            (Ex::Bin(BinOp::Add, b(n(1)), b(n(2))), false),
            (Ex::Bin(BinOp::Eq, b(Ex::Var(vx())), b(n(2))), false),
            (Ex::Cond(b(n(1)), b(n(1)), Some(b(n(2)))), false),
            (Ex::Math(0, vec![n(1)]), false),
            (Ex::Query(0, vec![n(1), Ex::Str(0)]), false),
            (Ex::Arrow(b(n(1)), b(Ex::Var(vx()))), false),
        ] {
            let text = simple(ex.clone()).canonical();
            assert_eq!(text.ends_with(';'), ends, "{ex:?} printed as {text:?}");
            assert!(!text.ends_with(";;") && !text.ends_with("; ;"), "{text:?}");
        }
        // A statement list is closed by its own semicolons, and one more is not added.
        assert_eq!(
            Program::Complex(vec![Stmt::Expr(Ex::Assign(vx(), b(n(1))))]).canonical(),
            "v.x = 1 ;"
        );
    }

    #[test]
    fn a_draw_of_63_leaves_out_what_the_grammar_needs_and_the_canonical_form_never_does() {
        let omit = |program: &Program, draws: &[u8]| {
            program
                .source(&style(&stream_for(draws), &[]))
                .to_ascii_lowercase()
        };
        // `(v.x + 1) * 2`: draws are the wrap of the whole expression, the omission test of the
        // left operand.
        let grouped = simple(Ex::Bin(
            BinOp::Mul,
            b(Ex::Bin(BinOp::Add, b(Ex::Var(vx())), b(n(1)))),
            b(n(2)),
        ));
        assert_eq!(omit(&grouped, &[0, 0, 0, 0, 0, 0, 0, 0]), "(v.x+1)*2");
        assert_eq!(omit(&grouped, &[0, 63, 0, 0, 0, 0, 0, 0]), "v.x+1*2");
        // Any other value of the draw writes the parentheses; so does a draw of 63 in the canonical
        // form.
        assert_eq!(omit(&grouped, &[0, 62, 0, 0, 0, 0, 0, 0]), "(v.x+1)*2");
        assert_eq!(
            grouped.print(&style(&stream_for(&[63; 8]), &[]), true),
            "( v.x + 1 ) * 2"
        );
        // The final `;` of an expression with an `=` in it.
        let assign = simple(Ex::Assign(vx(), b(n(1))));
        assert_eq!(omit(&assign, &[0, 0, 0, 0, 0]), "v.x=1;");
        assert_eq!(omit(&assign, &[0, 0, 0, 0, 63]), "v.x=1");
    }

    #[test]
    fn the_canonical_printer_draws_no_structure_choices() {
        let mut p = Printer::new(&[7, 7, 7], &[], true);
        p.ex(&Ex::Bin(BinOp::Add, b(n(1)), b(Ex::Var(vx()))));
        assert_eq!(p.out, "1 + v.x");
        assert_eq!(p.structure.at, 0, "the stream is not read");
        assert_eq!(p.cosmetic.at, 0);
    }

    #[test]
    fn tok_separates_adjacent_words_even_when_the_stream_says_nothing() {
        let mut p = Printer::new(&[], &[], false);
        p.tok("return");
        p.tok("1");
        p.tok(";");
        p.tok("return");
        assert_eq!(p.out, "RETURN 1;RETURN");
        let mut p = Printer::new(&[], &[], false);
        p.tok("a.b");
        p.tok("'str'");
        p.tok("x");
        assert_eq!(p.out, "A.B 'str' X", "a quote is a word character too");
        let mut p = Printer::new(&[], &[], false);
        p.tok("(");
        p.tok(")");
        assert_eq!(p.out, "()", "punctuation needs no separator");
    }

    #[test]
    fn tok_draws_a_separator_per_token_and_a_case_per_letter() {
        // Two tokens "ab" and "(": the separator draw happens only when something precedes the
        // token, but it is drawn for the first token too; then one draw per letter.
        let mut p = Printer::new(&[], &[3, 3, 3, 3, 3, 3, 3], false);
        p.tok("ab");
        assert_eq!(p.cosmetic.at, 3);
        p.tok("(");
        assert_eq!(
            p.cosmetic.at, 5,
            "a separator draw and one letter draw even for punctuation"
        );
    }

    #[test]
    fn tok_never_changes_a_string_literal() {
        let mut p = Printer::new(&[], &[0, 0, 0, 0, 0, 0], false);
        p.tok("'MoO x'");
        assert_eq!(p.out, "'MoO x'");
        assert_eq!(p.cosmetic.at, 1, "only the separator was drawn");
    }

    /// A cosmetic stream whose draw `k` is `draws[k]` (the stream mixes the position into each
    /// byte, so the byte at position `k` is the wanted draw rotated back; all of it is the first
    /// lap).
    fn stream_for(draws: &[u8]) -> Vec<u8> {
        draws
            .iter()
            .enumerate()
            .map(|(k, draw)| draw.rotate_right((k % 7) as u32))
            .collect()
    }

    #[test]
    fn stream_for_reproduces_the_draws_it_was_given() {
        let draws = [0, 1, 2, 3, 100, 200, 255, 254, 17, 0, 9];
        let bytes = stream_for(&draws);
        let mut choices = Choices::new(&bytes);
        let read: Vec<u8> = (0..draws.len()).map(|_| choices.next()).collect();
        assert_eq!(read, draws);
    }

    #[test]
    fn the_cosmetic_stream_picks_the_separators() {
        // `(` then `)`: draws 0 and 1 belong to the first token (separator, unused; then its one
        // character), draw 2 is the second token's separator (mod 16), draw 3 its character.
        let cases = [
            (0u8, ""),
            (5, ""),
            (6, " "),
            (10, " "),
            (11, "  "),
            (12, "\t"),
            (13, "\n"),
            (14, "\r\n"),
            (15, " \t "),
            (16, ""),
            (22, " "),
            (255, " \t "),
        ];
        for (draw, expected) in cases {
            let bytes = stream_for(&[0, 1, draw, 1]);
            let mut p = Printer::new(&[], &bytes, false);
            p.tok("(");
            p.tok(")");
            assert_eq!(p.out, format!("({expected})"), "draw {draw}");
        }
    }

    #[test]
    fn the_cosmetic_stream_flips_a_letter_when_its_draw_is_a_multiple_of_three() {
        // `ab`: the separator draw, then one draw per letter.
        let cases = [
            ([0u8, 3, 3], "AB"),
            ([0, 1, 1], "ab"),
            ([0, 3, 1], "Ab"),
            ([0, 1, 6], "aB"),
            ([0, 0, 254], "Ab"),
            ([0, 255, 253], "Ab"),
            ([0, 255, 252], "AB"),
            ([0, 254, 252], "aB"),
        ];
        for (draws, expected) in cases {
            let bytes = stream_for(&draws);
            let mut p = Printer::new(&[], &bytes, false);
            p.tok("ab");
            assert_eq!(p.out, expected, "draws {draws:?}");
        }
        // Digits and punctuation draw too but do not change.
        let bytes = stream_for(&[0, 3, 3, 3]);
        let mut p = Printer::new(&[], &bytes, false);
        p.tok("1+");
        assert_eq!(p.out, "1+");
    }

    #[test]
    fn a_cosmetic_draw_is_taken_for_every_token_even_the_first() {
        let mut p = Printer::new(&[], &[1; 40], false);
        p.tok("ab");
        assert_eq!(p.cosmetic.at, 3, "one separator draw and two letter draws");
        p.tok("(");
        assert_eq!(p.cosmetic.at, 5);
        p.tok("'q'");
        assert_eq!(
            p.cosmetic.at, 6,
            "a string literal is not cased: only the separator is drawn"
        );
        p.tok("");
        assert_eq!(p.cosmetic.at, 7);
    }

    #[test]
    fn the_cosmetic_stream_changes_the_case_and_the_white_space_but_nothing_else() {
        for seed in 0..200 {
            let case = case_of(&buffer(seed, 100));
            let base = case.program.source(&case.style);
            let squeezed_base = squeezed(&base);
            for other in 1..4 {
                let cosmetic = buffer(seed * 7 + other, 40);
                let text = case.program.source(&case.style.with_cosmetic(cosmetic));
                assert_eq!(squeezed(&text), squeezed_base, "seed {seed}");
            }
            // And the canonical text is the same token stream written with the structure of an
            // empty stream.
            let plain = Style {
                structure: vec![],
                cosmetic: buffer(seed, 40),
            };
            assert_eq!(
                squeezed(&case.program.source(&plain)),
                squeezed(&case.program.canonical()),
                "seed {seed}"
            );
        }
    }

    #[test]
    fn a_structure_stream_changes_the_text_of_most_programs() {
        let mut changed = 0;
        for seed in 0..100 {
            let case = case_of(&buffer(seed, 100));
            let with = case.program.source(&Style {
                structure: buffer(seed, 32),
                cosmetic: vec![],
            });
            let without = case.program.source(&Style::default());
            if squeezed(&with) != squeezed(&without) {
                changed += 1;
            }
        }
        assert!(
            changed > 50,
            "{changed} of 100 programs print differently under a structure stream"
        );
    }

    #[test]
    fn the_two_streams_are_read_independently() {
        // The structure stream is read the same way whatever the cosmetic stream is.
        let ex = Ex::Bin(BinOp::Add, b(Ex::Var(vx())), b(n(1)));
        let structure = [5u8, 9, 1, 5, 3, 3, 3, 3, 3, 3];
        let mut reads = Vec::new();
        for cosmetic in [vec![], vec![1], buffer(1, 9), buffer(2, 30)] {
            let mut p = Printer::new(&structure, &cosmetic, false);
            p.ex(&ex);
            reads.push((p.structure.at, squeezed(&p.out)));
        }
        assert!(reads.windows(2).all(|w| w[0] == w[1]), "{reads:?}");
        assert!(reads[0].0 >= 5, "the stream was read: {reads:?}");
    }

    mod compiled {
        use super::*;
        use molangx::compile::{CompileOptions, compile};
        use molangx::hash::HashedStr;
        use molangx::internals::{Payload, tokens};
        use molangx::ops::ExpressionOp as Op;

        fn options() -> CompileOptions {
            molangx::internals::reference_catalog::options(13)
        }

        /// The tokens (op and payload) the lexer makes of `src`, or why it refused.
        fn lex(src: &str) -> Result<Vec<(Op, Payload)>, String> {
            tokens(src, &options())
        }

        /// The single float `src` lexes to.
        fn lexed_float(src: &str) -> f32 {
            let tokens = lex(src).unwrap_or_else(|e| panic!("{src:?}: {e:?}"));
            match tokens.as_slice() {
                [(Op::Float, Payload::Float(x))] => *x,
                other => panic!("{src:?} lexed to {other:?}"),
            }
        }

        fn disassembly(src: &str) -> Option<String> {
            compile(src, &options())
                .expr()
                .cloned()
                .map(|e| e.disassemble())
        }

        #[test]
        fn every_kind_of_node_prints_text_the_lexer_accepts() {
            for (name, program) in one_of_each_kind() {
                for text in [
                    program.canonical(),
                    program.source(&Style::default()),
                    program.source(&style(&[1, 2, 3, 4, 5, 6, 7], &[0, 9, 12, 13, 14, 15, 33])),
                ] {
                    let tokens = lex(&text)
                        .unwrap_or_else(|log| panic!("{name}: {text:?} is not lexed: {log:?}"));
                    assert!(!tokens.is_empty(), "{name}");
                }
            }
        }

        #[test]
        fn no_kind_of_node_prints_text_the_compiler_calls_an_unrecognized_token() {
            for (name, program) in one_of_each_kind() {
                for text in [
                    program.canonical(),
                    program.source(&Style::default()),
                    program.source(&style(&[1, 5, 3, 7], &[6, 11, 12, 13, 14, 15])),
                ] {
                    let compiled = compile(&text, &options());
                    assert!(
                        compiled
                            .diagnostics()
                            .iter()
                            .all(|d| !d.message().contains("unrecognized token")),
                        "{name}: {text:?}: {:?}",
                        compiled.diagnostics()
                    );
                }
            }
        }

        #[test]
        fn the_expression_kinds_compile_and_the_statement_kinds_need_a_statement_list() {
            // The statement list returns before its `break`; every other kind is accepted.
            let rejected = ["statements"];
            for (name, program) in one_of_each_kind() {
                let compiled = compile(&program.canonical(), &options());
                let accepted = compiled.is_success();
                assert_eq!(
                    accepted,
                    !rejected.contains(&name),
                    "{name}: {:?}",
                    compiled.diagnostics()
                );
            }
            // A block, a loop or a for_each is an expression only as a statement: the printer ends
            // it with `;`.
            for (name, program) in one_of_each_kind() {
                if let Program::Simple(ex @ (Ex::Block(_) | Ex::Loop(..) | Ex::ForEach(..))) =
                    program
                {
                    assert!(
                        Program::Simple(ex.clone()).canonical().ends_with(';'),
                        "{name}"
                    );
                    let wrapped = Program::Complex(vec![Stmt::Expr(ex)]);
                    let compiled = compile(&wrapped.canonical(), &options());
                    assert_eq!(
                        compiled.failure(),
                        None,
                        "{name}: {:?}",
                        compiled.diagnostics()
                    );
                }
            }
            // The statement list of `one_of_each_kind` returns before its `break`.
            let compiled = compile(
                &one_of_each_kind().pop().expect("a last kind").1.canonical(),
                &options(),
            );
            assert_eq!(
                compiled
                    .diagnostics()
                    .iter()
                    .map(|d| d.message().into_owned())
                    .collect::<Vec<_>>(),
                ["Error: unreachable statements after Return 'return'."]
            );
        }

        #[test]
        fn every_operator_nested_in_every_other_compiles_at_every_version() {
            let x = || Ex::Var(vx());
            // Everything that is a number where one is wanted.
            let mut children = vec![
                n(1),
                Ex::Bool(true),
                Ex::This,
                x(),
                Ex::Neg(b(n(2))),
                Ex::Not(b(x())),
                Ex::Cond(b(x()), b(n(1)), Some(b(n(2)))),
                Ex::Cond(b(x()), b(n(1)), None),
                Ex::Math(18, vec![x(), n(1)]),
                Ex::Math(22, vec![]),
                Ex::Arrow(b(Ex::Var(var(Ns::Context, 0, &[]))), b(x())),
                Ex::Query(0, vec![n(1), x()]),
            ];
            children.extend(BinOp::ALL.map(|op| {
                let left = if op == BinOp::Coalesce { x() } else { n(1) };
                Ex::Bin(op, b(left), b(n(2)))
            }));
            let mut parents: Vec<Box<dyn Fn(Ex) -> Ex>> = vec![
                Box::new(|e| Ex::Neg(b(e))),
                Box::new(|e| Ex::Not(b(e))),
                Box::new(|e| Ex::Cond(b(e), b(n(1)), Some(b(n(2))))),
                Box::new(|e| Ex::Cond(b(Ex::Var(vx())), b(e), Some(b(n(2))))),
                Box::new(|e| Ex::Cond(b(Ex::Var(vx())), b(n(1)), Some(b(e)))),
                Box::new(|e| Ex::Cond(b(Ex::Var(vx())), b(e), None)),
                Box::new(|e| Ex::Math(0, vec![e])),
                Box::new(|e| Ex::Math(18, vec![n(1), e])),
                Box::new(|e| Ex::Query(0, vec![e, n(1)])),
                Box::new(|e| Ex::Arrow(b(e), b(Ex::Var(vx())))),
                Box::new(|e| Ex::Loop(b(e), vec![Stmt::Break])),
                // The right side of `??`.
                Box::new(|e| Ex::Bin(BinOp::Coalesce, b(Ex::Var(vx())), b(e))),
            ];
            for op in BinOp::ALL {
                if op != BinOp::Coalesce {
                    parents.push(Box::new(move |e| Ex::Bin(op, b(e), b(n(1)))));
                }
                parents.push(Box::new(move |e| Ex::Bin(op, b(Ex::Var(vx())), b(e))));
            }
            let mut tried = 0;
            for (i, parent) in parents.iter().enumerate() {
                for (j, child) in children.iter().enumerate() {
                    // `a->b->c` is not supported (the generator never writes one).
                    if i == 9 && matches!(child, Ex::Arrow(..)) {
                        continue;
                    }
                    // A comparison or an arithmetic operator takes a number; `Not`, `Neg`, `Cond`
                    // and the others too.
                    let program = simple(parent(child.clone()));
                    for version in 0..=13 {
                        let compiled = compile(
                            &program.canonical(),
                            &molangx::internals::reference_catalog::options(version),
                        );
                        assert_eq!(
                            compiled.failure(),
                            None,
                            "parent {i} child {j} v{version}: {:?}: {:?}",
                            program.canonical(),
                            compiled.diagnostics()
                        );
                        tried += 1;
                    }
                }
            }
            assert!(tried > 10_000, "{tried}");
        }

        #[test]
        fn generated_text_never_makes_an_unrecognized_token() {
            for seed in 0..300 {
                let case = case_of(&buffer(seed, 100));
                let text = case.source();
                let compiled = compile(&text, &options());
                let unrecognized = compiled
                    .diagnostics()
                    .iter()
                    .any(|d| d.message().contains("unrecognized token"));
                assert!(
                    !unrecognized,
                    "seed {seed}: {text:?}: {:?}",
                    compiled.diagnostics()
                );
            }
        }

        /// Lowering cuts a text only at its first NUL, so it keeps generated text whole.
        #[test]
        fn generated_text_holds_no_nul() {
            for seed in 0..100 {
                let text = case_of(&buffer(seed, 100)).source();
                assert!(!text.contains('\0'), "seed {seed}: {text:?}");
            }
        }

        #[test]
        fn cosmetics_lex_to_the_same_tokens() {
            for seed in 0..300 {
                let program = program_of(&buffer(seed, 100));
                let canonical = lex(&program.canonical());
                for other in 0..3 {
                    let styled = lex(&program.source(&style(&[], &buffer(seed * 11 + other, 50))));
                    assert_eq!(canonical, styled, "seed {seed}");
                }
            }
        }

        #[test]
        fn cosmetics_compile_to_the_same_program() {
            for seed in 0..300 {
                let program = program_of(&buffer(seed, 100));
                let structure = buffer(seed + 1000, 30);
                let reference = disassembly(&program.source(&style(&structure, &[])));
                for other in 1..4 {
                    let text = program.source(&style(&structure, &buffer(seed * 13 + other, 50)));
                    assert_eq!(disassembly(&text), reference, "seed {seed}: {text:?}");
                }
                // The canonical form is the same program as the all-defaults structure.
                assert_eq!(
                    disassembly(&program.canonical()),
                    disassembly(&program.source(&style(&[], &buffer(seed, 20)))),
                    "seed {seed}"
                );
            }
        }

        #[test]
        fn structure_choices_that_cannot_change_the_parse_compile_to_the_same_program() {
            // Operands that are single tokens: redundant parentheses, the `f` suffix, the fraction
            // and the leading zero, and the long namespaces change the text but not the program.
            let programs = [
                simple(Ex::Bin(BinOp::Add, b(n(1)), b(n(7)))),
                simple(Ex::Bin(BinOp::Mul, b(Ex::Var(vx())), b(n(9)))),
                simple(Ex::Math(18, vec![Ex::Var(vx()), n(2)])),
                simple(Ex::Cond(b(Ex::Var(vx())), b(n(1)), Some(b(n(2))))),
                simple(Ex::Neg(b(n(3)))),
                Program::Complex(vec![
                    Stmt::Expr(Ex::Assign(vx(), b(n(7)))),
                    Stmt::Return(Ex::Var(var(Ns::Temp, 0, &[]))),
                ]),
            ];
            let streams: [&[u8]; 6] = [
                &[],
                &[1],
                &[3, 5, 2, 7],
                &[7, 7, 7, 7, 7],
                &[2, 10, 3, 9, 5, 11, 1],
                &[255, 254, 253, 252, 251],
            ];
            for program in &programs {
                let reference = disassembly(&program.canonical());
                assert!(reference.is_some(), "{program:?} compiles");
                for structure in streams {
                    let text = program.source(&style(structure, &[0, 6, 12]));
                    assert_eq!(
                        disassembly(&text),
                        reference,
                        "{program:?} printed as {text:?}"
                    );
                }
            }
        }

        #[test]
        fn an_operand_that_binds_looser_is_parenthesised_unless_the_structure_stream_omits_it() {
            // `(v.x + 2) * 3` needs its parentheses: the tree is `*` over `+`. A draw of 63 omits
            // them, and the text `v.x + 2 * 3` is another grouping, `v.x + 6`.
            let program = simple(Ex::Bin(
                BinOp::Mul,
                b(Ex::Bin(BinOp::Add, b(Ex::Var(vx())), b(n(2)))),
                b(n(3)),
            ));
            assert_eq!(program.canonical(), "( v.x + 2 ) * 3");
            let plain = disassembly(&program.canonical()).expect("compiles");
            assert_eq!(
                disassembly(&program.source(&Style::default())).as_deref(),
                Some(plain.as_str())
            );
            // Draws: the whole expression (no wrap), then the left operand's required parentheses
            // (omitted).
            let omitted = disassembly(&program.source(&style(
                &stream_for(&[0, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                &[],
            )))
            .expect("compiles");
            assert_ne!(plain, omitted);
            assert!(
                plain.contains("*3+6") && omitted.contains("*1+6"),
                "{plain} / {omitted}"
            );
        }

        /// What the lexer makes of the integer written `text`: wrapped to 32 bits.
        fn wrapped_integer(text: &str) -> f32 {
            text.bytes().fold(0i32, |acc, digit| {
                acc.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'))
            }) as f32
        }

        /// The float the lexer must read for pool entry `index` however it is spelled.
        fn expected_float(index: u8) -> f32 {
            match NUMBERS.get(usize::from(index)) {
                // 1e10 and 3e38, written out as integers, do not fit in 32 bits and wrap in the
                // lexer.
                Some(value) if *value >= 2_147_483_648.0 => wrapped_integer(&format!("{value}")),
                Some(value) => *value as f32,
                None => f32::from(index),
            }
        }

        #[test]
        fn the_number_pool_survives_the_lexer() {
            let mut wrapped = Vec::new();
            for (i, value) in NUMBERS.iter().enumerate() {
                let text = simple(n(i as u8)).canonical();
                let got = lexed_float(&text);
                if *value >= 2_147_483_648.0 {
                    wrapped.push(i);
                    assert_eq!(
                        got.to_bits(),
                        wrapped_integer(&text).to_bits(),
                        "NUMBERS[{i}] = {value}"
                    );
                    assert_ne!(
                        got.to_bits(),
                        (*value as f32).to_bits(),
                        "NUMBERS[{i}] = {value} is too big for the integer accumulator"
                    );
                } else {
                    assert_eq!(
                        got.to_bits(),
                        (*value as f32).to_bits(),
                        "NUMBERS[{i}] = {value} printed as {text:?} lexed to {got}"
                    );
                }
            }
            assert_eq!(wrapped, [23, 24], "the two magnitudes beyond 2^31");
        }

        #[test]
        fn the_entries_needing_eight_fraction_digits_survive() {
            for (index, text) in [
                (19u8, "0.0000001"),
                (20, "0.00000012"),
                (11, "0.1"),
                (12, "0.3"),
            ] {
                assert_eq!(simple(n(index)).canonical(), text);
                assert_eq!(
                    lexed_float(text).to_bits(),
                    (NUMBERS[usize::from(index)] as f32).to_bits(),
                    "{text}"
                );
            }
        }

        #[test]
        fn a_pool_miss_prints_an_integer_the_lexer_reads_back() {
            for i in (NUMBERS.len() as u8)..=255 {
                let text = simple(n(i)).canonical();
                assert_eq!(text, i.to_string());
                assert_eq!(lexed_float(&text), f32::from(i));
            }
        }

        #[test]
        fn every_spelling_of_a_number_reads_back_the_same_float() {
            for index in 0..=255u8 {
                let value = expected_float(index);
                for choice in 0..=255u8 {
                    let bytes = [choice];
                    let mut p = Printer::new(&bytes, &[1], false);
                    p.number(index);
                    let text = p.out;
                    let got = lexed_float(&text);
                    assert_eq!(
                        got.to_bits(),
                        value.to_bits(),
                        "index {index}, choice {choice}: {text:?} lexed to {got}"
                    );
                }
            }
        }

        proptest::proptest! {
            #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

            #[test]
            fn number_printing_round_trips_for_any_index_and_choice(index in 0u8..=255, choice in 0u8..=255) {
                let value = expected_float(index);
                let bytes = [choice];
                let mut p = Printer::new(&bytes, &[1], false);
                p.number(index);
                let got = lexed_float(&p.out);
                proptest::prop_assert_eq!(got.to_bits(), value.to_bits());
            }

            #[test]
            fn a_printed_string_has_exactly_two_quotes(index in 0u8..=255, structure in proptest::collection::vec(0u8..=255, 0..8), cosmetic in proptest::collection::vec(0u8..=255, 0..8)) {
                let text = simple(Ex::Str(index)).source(&style(&structure, &cosmetic));
                let pooled = STRINGS.get(usize::from(index)).copied().unwrap_or("");
                proptest::prop_assert_eq!(text.matches('\'').count(), 2);
                proptest::prop_assert!(text.contains(&format!("'{pooled}'")), "{text:?}");
            }
        }

        #[test]
        fn a_negated_zero_prints_a_minus_and_a_zero_and_folds_to_negative_zero() {
            let text = simple(Ex::Neg(b(n(0)))).canonical();
            assert_eq!(text, "- 0");
            let constant = compile(&text, &options())
                .expr()
                .cloned()
                .expect("compiles")
                .as_constant()
                .expect("folds to a constant");
            assert_eq!(constant.to_bits(), (-0.0f32).to_bits());
            // No number in the pool is negative: the sign always comes from the operator.
            assert!(NUMBERS.iter().all(|x| x.is_sign_positive()));
        }

        #[test]
        fn every_string_of_the_pool_lexes_to_the_hash_of_its_exact_text() {
            for (i, text) in STRINGS.iter().enumerate() {
                let printed = simple(Ex::Str(i as u8)).canonical();
                assert_eq!(printed, format!("'{text}'"));
                let tokens = lex(&printed).unwrap_or_else(|e| panic!("{printed:?}: {e:?}"));
                assert_eq!(
                    tokens,
                    [(
                        Op::StringLiteral,
                        Payload::Hash(HashedStr::new(text).as_u64())
                    )],
                    "{text:?}"
                );
            }
        }

        #[test]
        fn a_string_survives_every_style_unchanged() {
            for seed in 0..100 {
                let s = style(&buffer(seed, 10), &buffer(seed + 500, 10));
                let text = simple(Ex::Str(3)).source(&s);
                assert!(text.contains("'Moo X'"), "{text:?}");
                assert_eq!(text.matches('\'').count(), 2, "{text:?}");
            }
        }

        #[test]
        fn a_pool_name_prints_in_every_letter_case_the_lexer_folds_back() {
            // The printed `V.NEVER` and `variable.never` read the same variable.
            let canonical = lex("v.never");
            for text in ["V.NEVER", "Variable.Never", "VARIABLE.never"] {
                let got = lex(text).expect("lexes");
                let want = canonical.clone().expect("lexes");
                assert_eq!(got.len(), want.len());
                assert_eq!(got[0].1, want[0].1, "{text}");
            }
        }
    }
}
