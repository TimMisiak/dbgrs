use windows_sys::{
    Win32::System::{Diagnostics::Debug::*},
};

pub fn display_all(context: &CONTEXT) {
    println!("rax={:#018x} rbx={:#018x} rcx={:#018x}", context.Rax, context.Rbx, context.Rcx);
    println!("rdx={:#018x} rsi={:#018x} rdi={:#018x}", context.Rdx, context.Rsi, context.Rdi);
    println!("rip={:#018x} rsp={:#018x} rbp={:#018x}", context.Rip, context.Rsp, context.Rbp);
    println!(" r8={:#018x}  r9={:#018x} r10={:#018x}", context.R8, context.R9, context.R10);
    println!("r11={:#018x} r12={:#018x} r13={:#018x}", context.R11, context.R12, context.R13);
    println!("r14={:#018x} r15={:#018x} eflags={:#010x}", context.R14, context.R15, context.EFlags);
}

pub fn display_named(context: &CONTEXT, reg_name: &str) {
    if let Ok(val) = get_register(context, reg_name) {
        println!("{}={:#018x}", reg_name.to_lowercase(), val);
    } else {
        println!("Unrecognized register name: {}", reg_name);
    }
}

pub fn get_register(context: &CONTEXT, reg_name: &str) -> Result<u64, String> {
    let val = match reg_name.to_lowercase().as_str() {
        "rax" => context.Rax,
        "rbx" => context.Rbx,
        "rcx" => context.Rcx,
        "rdx" => context.Rdx,
        "rsi" => context.Rsi,
        "rdi" => context.Rdi,
        "rip" => context.Rip,
        "rsp" => context.Rsp,
        "rbp" => context.Rbp,
        "r8" => context.R8,
        "r9" => context.R9,
        "r10" => context.R10,
        "r11" => context.R11,
        "r12" => context.R12,
        "r13" => context.R13,
        "r14" => context.R14,
        "r15" => context.R15,
        "eflags" => context.EFlags as u64,
        _ => return Err("Unrecognized register".to_string())
    };
    Ok(val)
}
