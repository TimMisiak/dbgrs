use crate::command::grammar::EvalExpr;
use crate::process::Process;
use crate::name_resolution::resolve_name_to_address;

pub struct EvalContext<'a> {
    pub process: &'a mut Process,
}

pub fn evaluate_expression(expr: EvalExpr, context: &mut EvalContext) -> Result<u64, String> {
    match expr {
        EvalExpr::Number(x) => Ok(x),
        EvalExpr::Add(x, _, y) => Ok(evaluate_expression(*x, context)? + evaluate_expression(*y, context)?),
        EvalExpr::Symbol(sym) => {
            resolve_name_to_address(&sym, context.process)
        }
    }
}
