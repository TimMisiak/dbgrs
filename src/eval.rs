use crate::command::grammar::EvalExpr;

// TODO: Expression evaluation needs an evaluation context. Possibly including memory read, register read, and symbol names
pub fn evaluate_expression(expr: EvalExpr) -> u64 {
    match expr {
        EvalExpr::Number(x) => x,
        EvalExpr::Add(x, _, y) => evaluate_expression(*x) + evaluate_expression(*y),
    }
}
