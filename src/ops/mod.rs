//! The operator table: [`ExpressionOp`], the node kinds of a parsed expression, one [`OpMeta`]
//! row each in [`OP_META`], and [`OpSet`], the operations a compilation allows.

mod op;
mod set;
mod table;

pub use op::ExpressionOp;
pub use set::OpSet;
pub use table::{OP_META, OpFlags, OpMeta};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_re_exports_are_the_table_items() {
        let _: &'static [OpMeta; ExpressionOp::COUNT] = &OP_META;
        assert_eq!(OP_META[0].op, ExpressionOp::LeftBrace);
    }

    #[test]
    fn row_counts_agree_across_the_tables() {
        assert_eq!(ExpressionOp::COUNT, OP_META.len());
        assert_eq!(ExpressionOp::COUNT, ExpressionOp::all().len());
        assert_eq!(OpSet::all().len(), ExpressionOp::COUNT);
    }
}
