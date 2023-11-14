use windows::Win32::System::Diagnostics::Debug::IMAGE_DIRECTORY_ENTRY_EXCEPTION;
use windows_sys::Win32::System::Diagnostics::Debug::CONTEXT;
use crate::{process::Process, memory::{MemorySource, read_memory_full_array, read_memory_data}};

#[repr(C)]
#[derive(Default, Clone)]
#[allow(non_snake_case, non_camel_case_types)]
pub struct RUNTIME_FUNCTION {
    pub BeginAddress: u32,
    pub EndAddress: u32,
    pub UnwindInfo: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
#[allow(non_snake_case, non_camel_case_types)]
pub struct UNWIND_INFO {
    pub version_flags: u8,
    pub size_of_prolog: u8,
    pub count_of_codes: u8,
    pub frame_register_offset: u8,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
#[allow(non_snake_case, non_camel_case_types)]
pub struct UNWIND_CODE {
    pub code_offset: u8,
    pub unwind_op_info: u8,
}

const UWOP_PUSH_NONVOL: u8 = 0;     /* info == register number */
const UWOP_ALLOC_LARGE: u8 = 1;     /* no info, alloc size in next 2 slots */
const UWOP_ALLOC_SMALL: u8 = 2;     /* info == size of allocation / 8 - 1 */
const UWOP_SET_FPREG: u8 = 3;       /* no info, FP = RSP + UNWIND_INFO.FPRegOffset*16 */
const UWOP_SAVE_NONVOL: u8 = 4;     /* info == register number, offset in next slot */
const UWOP_SAVE_NONVOL_FAR: u8 = 5; /* info == register number, offset in next 2 slots */
const UWOP_SAVE_XMM128: u8 = 8;     /* info == XMM reg number, offset in next slot */
const UWOP_SAVE_XMM128_FAR: u8 = 9; /* info == XMM reg number, offset in next 2 slots */
const UWOP_PUSH_MACHFRAME: u8 = 10; /* info == 0: no error-code, 1: error-code */

const UNW_FLAG_NHANDLER: u8 = 0x0;
const UNW_FLAG_EHANDLER: u8 = 0x1;
const UNW_FLAG_UHANDLER: u8 = 0x2;
const UNW_FLAG_CHAININFO: u8 = 0x4;

// These represent the logical operations, so large/small and far/near are merged
#[derive(Debug)]
enum UnwindOp {
    PushNonVolatile { reg: u8 },
    Alloc { size: u32 },
    SetFpreg,
    SaveNonVolatile { reg: u8, offset: u32 },
    SaveXmm128 { reg: u8, offset: u32 },
    PushMachFrame { error_code: bool }
}

// Does not directly correspond to UNWIND_CODE
#[derive(Debug)]
struct UnwindCode {
    code_offset: u8,
    op: UnwindOp,
}

fn find_runtime_function(addr: u32, function_list: &[RUNTIME_FUNCTION]) -> Option<&RUNTIME_FUNCTION> {
    let index = function_list.binary_search_by(|func| func.BeginAddress.cmp(&addr));

    match index {
        // Exact match
        Ok(pos) => function_list.get(pos),
        // Inexact match
        Err(pos) => {
            if pos > 0 && function_list.get(pos - 1).map_or(false, |func| func.BeginAddress <= addr && addr < func.EndAddress) {
                function_list.get(pos - 1)
            } else if pos < function_list.len() && function_list.get(pos).map_or(false, |func| func.BeginAddress <= addr && addr < func.EndAddress) {
                function_list.get(pos)
            } else {
                None
            }
        }
    }
}

// Splits an integer up that represents bitfields so that each field can be stored in a tuple. Specify the
// size of the fields from low bits to high bits. For instance, let (x, y, z) = split_up!(q => 3, 6, 7) will put the low 3 bits into x
macro_rules! split_up {
    ($value:expr => $($len:expr),+) => {
        {
            let mut value = $value;
            // Use a tuple to collect the fields
            ( $(
                {
                    let field = value & ((1 << $len) - 1); // Mask the value to get the field
                    value >>= $len; // Shift the value for the next field
                    field
                }
            ),+ ) // The '+' sign indicates one or more repetitions
        }
    };
}

fn get_unwind_ops(code_slots: &[u16]) -> Result<Vec<UnwindCode>, &'static str> {
    let mut ops = Vec::<UnwindCode>::new();

    let mut i = 0;
    while i < code_slots.len() {
        let (code_offset, unwind_op, op_info) = split_up!(code_slots[i] => 8, 4, 4);
        let code_offset = code_offset as u8;
        let unwind_op = unwind_op as u8;
        let op_info = op_info as u8;
        match unwind_op {
            UWOP_PUSH_NONVOL => {
                ops.push(UnwindCode { code_offset, op: UnwindOp::PushNonVolatile { reg: op_info } });
            }
            UWOP_ALLOC_LARGE if op_info == 0 => {
                if i + 1 >= code_slots.len() {
                    return Err("UWOP_ALLOC_LARGE was incomplete");
                }
                let size = (code_slots[i + 1] as u32) * 8;
                ops.push(UnwindCode { code_offset, op: UnwindOp::Alloc { size } });
                i += 1;
            }
            UWOP_ALLOC_LARGE if op_info == 1 => {
                if i + 2 >= code_slots.len() {
                    return Err("UWOP_ALLOC_LARGE was incomplete");
                }
                let size = code_slots[i + 1] as u32 + ((code_slots[i + 2] as u32) << 16);
                ops.push(UnwindCode { code_offset, op: UnwindOp::Alloc { size } });
                i += 2;
            }
            UWOP_ALLOC_SMALL => {
                let size = (op_info as u32) * 8 + 8;
                ops.push(UnwindCode { code_offset, op: UnwindOp::Alloc { size } });
            }
            UWOP_SET_FPREG => {
                ops.push(UnwindCode { code_offset, op: UnwindOp::SetFpreg });
            }
            UWOP_SAVE_NONVOL => {
                if i + 1 >= code_slots.len() {
                    return Err("UWOP_SAVE_NONVOL was incomplete");
                }
                let offset = code_slots[i + 1] as u32;
                ops.push(UnwindCode { code_offset, op: UnwindOp::SaveNonVolatile { reg: op_info, offset } });
                i += 1;
            }
            UWOP_SAVE_NONVOL_FAR => {
                if i + 2 >= code_slots.len() {
                    return Err("UWOP_SAVE_NONVOL_FAR was incomplete");
                }
                let offset = code_slots[i + 1] as u32 + ((code_slots[i + 2] as u32) << 16);
                ops.push(UnwindCode { code_offset, op: UnwindOp::SaveNonVolatile { reg: op_info, offset } });
                i += 2;
            }
            UWOP_SAVE_XMM128 => {
                if i + 1 >= code_slots.len() {
                    return Err("UWOP_SAVE_XMM128 was incomplete");
                }
                let offset = code_slots[i + 1] as u32;
                ops.push(UnwindCode { code_offset, op: UnwindOp::SaveXmm128 { reg: op_info, offset: offset } });
                i += 1;
            }
            UWOP_SAVE_XMM128_FAR => {
                if i + 2 >= code_slots.len() {
                    return Err("UWOP_SAVE_XMM128_FAR was incomplete");
                }
                let offset = code_slots[i + 1] as u32 + ((code_slots[i + 2] as u32) << 16);
                ops.push(UnwindCode { code_offset, op: UnwindOp::SaveXmm128 { reg: op_info, offset: offset } });
                i += 2;
            }
            _ => return Err("Unrecognized unwind op")
        }
        i += 1;
    }

    Ok(ops)
}

fn get_op_register<'a>(context: &'a mut CONTEXT, reg: u8) -> &'a mut u64 {
    match reg {
        0 => &mut context.Rax,
        1 => &mut context.Rcx,
        2 => &mut context.Rdx,
        3 => &mut context.Rbx,
        4 => &mut context.Rsp,
        5 => &mut context.Rbp,
        6 => &mut context.Rsi,
        7 => &mut context.Rdi,
        8 => &mut context.R8,
        9 => &mut context.R9,
        10 => &mut context.R10,
        11 => &mut context.R11,
        12 => &mut context.R12,
        13 => &mut context.R13,
        14 => &mut context.R14,
        15 => &mut context.R15,
        _ => panic!("Bad register given to get_op_register()")
    }
}

fn apply_unwind_ops(context: &CONTEXT, unwind_ops: &[UnwindCode], func_address: u64, memory_source: &dyn MemorySource) -> Result<Option<CONTEXT>, &'static str> {
    let mut unwound_context = context.clone();
    for unwind in unwind_ops.iter() {
        let func_offset = unwound_context.Rip - func_address;
        if unwind.code_offset as u64 <= func_offset {
            match unwind.op {
                UnwindOp::Alloc { size } => {
                    unwound_context.Rsp += size as u64;
                }
                UnwindOp::PushNonVolatile { reg } => {
                    let addr = unwound_context.Rsp;
                    let val = read_memory_data::<u64>(memory_source, addr)?;
                    unwound_context.Rsp += 8;
                    *get_op_register(&mut unwound_context, reg) = val;
                }
                UnwindOp::SaveNonVolatile { reg, offset } => {
                    let addr = unwound_context.Rsp + offset as u64;
                    let val = read_memory_data::<u64>(memory_source, addr)?;
                    *get_op_register(&mut unwound_context, reg) = val;
                }
                _ => panic!("NYI unwind op")
            }
        }
    }
    Ok(Some(unwound_context))
}

pub fn unwind_context(process: &mut Process, context: CONTEXT, memory_source: &dyn MemorySource) -> Result<Option<CONTEXT>, &'static str> {
    let module = process.get_containing_module_mut(context.Rip);
    if let Some(module) = module {
        let data_directory = module.get_data_directory(IMAGE_DIRECTORY_ENTRY_EXCEPTION);
        if data_directory.VirtualAddress != 0 && data_directory.Size != 0 {
            let count = data_directory.Size as usize / std::mem::size_of::<RUNTIME_FUNCTION>();
            let table_address = module.address + data_directory.VirtualAddress as u64;

            // Note: In a real debugger you might want to cache these.
            let functions: Vec<RUNTIME_FUNCTION> = read_memory_full_array(memory_source, table_address, count)?;

            let rva = context.Rip - module.address;
            let func = find_runtime_function(rva as u32, &functions);

            if let Some(func) = func {
                // We have unwind data!
                let info_addr = module.address + func.UnwindInfo as u64;
                let info = read_memory_data::<UNWIND_INFO>(memory_source, info_addr)?;
                let (_version, flags) = split_up!(info.version_flags => 3, 5);
                if flags & UNW_FLAG_CHAININFO == UNW_FLAG_CHAININFO {
                    return Err("NYI: Chained info");
                }
                if info.frame_register_offset != 0 {
                    return Err("NYI frame_register_offset")
                }
                // The codes are UNWIND_CODE, but we'll have to break them up in different ways anyway based on the operation, so we might as well just
                // read them as u16 and then parse out the fields as needed.
                let codes = read_memory_full_array::<u16>(memory_source, info_addr + 4, info.count_of_codes as usize)?;
                let unwind_ops = get_unwind_ops(&codes)?;
                match apply_unwind_ops(&context, &unwind_ops, module.address + func.BeginAddress as u64, memory_source)? {
                    Some(ctx) => {
                        let mut ctx = ctx;
                        ctx.Rip = read_memory_data::<u64>(memory_source, ctx.Rsp)?;
                        ctx.Rsp += 8;

                        // TODO: There are other conditions that should be checked
                        if ctx.Rip == 0 {
                            return Ok(None);
                        }
                        return Ok(Some(ctx))
                    },
                    _ => return Ok(None)
                }
                
            } else {
                // Leaf function: the return address is simply at [RSP]
                let mut ctx = context;
                ctx.Rip = read_memory_data::<u64>(memory_source, ctx.Rsp)?;
                ctx.Rsp += 8;
                return Ok(Some(ctx));
            }
        }
    }
    
    Ok(None)
}