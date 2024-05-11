use crate::command::grammar::EvalExpr;
use crate::process::Process;
use crate::name_resolution::resolve_name_to_address;
use crate::registers::get_register;
use windows_sys::Win32::System::Diagnostics::Debug::CONTEXT;

pub struct EvalContext<'a> {
    pub process: &'a mut Process,
    // TODO: This should really be an abstraction on top of the context
    pub register_context: &'a CONTEXT,
}

pub fn evaluate_expression(expr: EvalExpr, context: &mut EvalContext) -> Result<u64, anyhow::Error> {
    match expr {
        EvalExpr::Number(x) => Ok(x),
        EvalExpr::Add(x, _, y) => Ok(evaluate_expression(*x, context)? + evaluate_expression(*y, context)?),
        EvalExpr::Symbol(sym) => {
            if sym.starts_with('@') {
                if let Ok(val) = get_register(context.register_context, &sym[1..]) {
                    return Ok(val);
                }
            }
            resolve_name_to_address(&sym, context.process)
        }
    }
}
