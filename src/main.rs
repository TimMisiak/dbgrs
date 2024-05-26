use event::DebugEvent;
use memory::MemorySource;
use windows_sys::{
    Win32::Foundation::*,
    Win32::System::Environment::*,
    Win32::System::{Diagnostics::Debug::*, Threading::*},
};

use std::{fs::File, io::{self, BufRead}, mem::MaybeUninit, ptr::null, cmp::{max, min}};

mod command;
mod eval;
mod memory;
mod process;
mod registers;
mod stack;
mod module;
mod name_resolution;
mod event;
mod breakpoint;
mod util;
mod unassemble;
mod source;

use process::Process;
use command::grammar::{CommandExpr, EvalExpr};
use breakpoint::BreakpointManager;
use util::*;
use source::resolve_address_to_source_line;

const TRAP_FLAG: u32 = 1 << 8;

fn show_usage(error_message: &str) {
    println!("Error: {msg}", msg = error_message);
    println!("Usage: DbgRs <Command Line>");
}

unsafe fn wcslen(ptr: *const u16) -> usize {
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    len
}

// For now, we only accept the command line of the process to launch, so we'll just return that for now. Later, we can parse additional
// command line options such as attaching to processes.
// Q: Why not just convert to UTF8?
// A: There can be cases where this is lossy, and we want to make sure we can debug as close as possible to normal execution.
fn parse_command_line() -> Result<Vec<u16>, &'static str> {
    let cmd_line = unsafe {
        // As far as I can tell, standard rust command line argument parsing won't preserve spaces. So we'll call
        // the win32 api directly and then parse it out.
        let p = GetCommandLineW();
        let len = wcslen(p);
        std::slice::from_raw_parts(p, len + 1)
    };

    let mut cmd_line_iter = cmd_line.iter().copied();

    let first = cmd_line_iter.next().ok_or("Command line was empty")?;

    // If the first character is a quote, we need to find a matching end quote. Otherwise, the first space.
    let end_char = (if first == '"' as u16 { '"' } else { ' ' }) as u16;

    loop {
        let next = cmd_line_iter.next().ok_or("No arguments found")?;
        if next == end_char {
            break;
        }
    }

    // Now we need to skip any whitespace
    let cmd_line_iter = cmd_line_iter.skip_while(|x| x == &(' ' as u16));

    Ok(cmd_line_iter.collect())
}

fn load_module_at_address(process: &mut Process, memory_source: &dyn MemorySource, base_address: u64, module_name: Option<String>) {
    let module = process.add_module(base_address, module_name, memory_source).unwrap();

    println!("LoadDll: {:X}   {}", base_address, module.name);
}

fn main_debugger_loop(process: HANDLE) {
    let mut expect_step_exception = false;
    let mem_source = memory::make_live_memory_source(process);
    let mut process = Process::new();
    let mut breakpoints = BreakpointManager::new();

    let mut source_search_paths = Vec::new();

    loop {
        let (event_context, debug_event) = event::wait_for_next_debug_event(mem_source.as_ref());

        // The thread context will be needed to determine what to do with some events
        let thread = AutoClosedHandle(unsafe {
            OpenThread(
                THREAD_GET_CONTEXT | THREAD_SET_CONTEXT,
                FALSE,
                event_context.thread_id,
            )
        });

        let mut ctx: AlignedContext = unsafe { std::mem::zeroed() };
        ctx.context.ContextFlags = CONTEXT_ALL;
        let ret = unsafe { GetThreadContext(thread.handle(), &mut ctx.context) };
        if ret == 0 {
            panic!("GetThreadContext failed");
        }

        let mut continue_status = DBG_CONTINUE;
        let mut is_exit = false;
        match debug_event {
            DebugEvent::Exception { first_chance, exception_code } => {
                let chance_string = if first_chance {
                    "first chance"
                } else {
                    "second chance"
                };

                if expect_step_exception && exception_code == EXCEPTION_SINGLE_STEP {
                    expect_step_exception = false;
                    continue_status = DBG_CONTINUE;
                } else if let Some(bp_index) = breakpoints.was_breakpoint_hit(&ctx.context) {
                    println!("Breakpoint {} hit", bp_index);
                    continue_status = DBG_CONTINUE;
                } else {
                    println!("Exception code {:x} ({})", exception_code, chance_string);
                    continue_status = DBG_EXCEPTION_NOT_HANDLED;
                }
            },
            DebugEvent::CreateProcess { exe_name, exe_base } => {
                load_module_at_address(&mut process, mem_source.as_ref(), exe_base, exe_name);
                process.add_thread(event_context.thread_id);
            },
            DebugEvent::CreateThread { thread_id } => {
                process.add_thread(thread_id);
                println!("Thread created: {:x}", thread_id);
            },
            DebugEvent::ExitThread { thread_id } => {
                process.remove_thread(thread_id);
                println!("Thread exited: {:x}", thread_id);
            },
            DebugEvent::LoadModule { module_name, module_base } => {
                load_module_at_address(&mut process, mem_source.as_ref(), module_base, module_name);
            },
            DebugEvent::OutputDebugString(debug_string) => println!("DebugOut: {}", debug_string),
            DebugEvent::Other(msg) => println!("{}", msg),
            DebugEvent::ExitProcess => {
                is_exit = true;
                println!("ExitProcess");
            },
        }

        let mut next_unassemble_address = ctx.context.Rip;
        let mut continue_execution = false;

        while !continue_execution {

            if let Some(sym) = name_resolution::resolve_address_to_name(ctx.context.Rip, &mut process) {
                println!("[{:X}] {}", event_context.thread_id, sym);
            } else {
                println!("[{:X}] {:#018x}", event_context.thread_id, ctx.context.Rip);
            }

            let cmd = command::read_command();


            let mut eval_expr = |expr: Box<EvalExpr>| -> Option<u64> {
                let mut eval_context = eval::EvalContext{ process: &mut process, register_context: &ctx.context };
                let result = eval::evaluate_expression(*expr, &mut eval_context);
                match result {
                    Ok(val) => Some(val),
                    Err(e) => {
                        print!("Could not evaluate expression: {}", e);
                        None
                    }
                }
            };

            match cmd {
                CommandExpr::StepInto(_) => {
                    ctx.context.EFlags |= TRAP_FLAG;
                    let ret = unsafe { SetThreadContext(thread.handle(), &ctx.context) };
                    if ret == 0 {
                        panic!("SetThreadContext failed");
                    }
                    expect_step_exception = true;
                    continue_execution = true;
                }
                CommandExpr::Go(_) => {
                    continue_execution = true;
                }
                CommandExpr::DisplayRegisters(_) => {
                    registers::display_all(&ctx.context);
                }
                CommandExpr::DisplaySpecificRegister(_, reg) => {
                    registers::display_named(&ctx.context, &reg);
                }
                CommandExpr::DisplayBytes(_, expr) => {
                    if let Some(address) = eval_expr(expr) {
                        let bytes = mem_source.read_raw_memory(address, 16);
                        for byte in bytes {
                            print!("{:02X} ", byte);
                        }
                        println!();
                    }
                }
                CommandExpr::Evaluate(_, expr) => {
                    if let Some(val) = eval_expr(expr) {
                        println!(" = 0x{:X}", val);
                    }
                }
                CommandExpr::ListNearest(_, expr) => {
                    if let Some(val) = eval_expr(expr) {
                        if let Some(sym) = name_resolution::resolve_address_to_name(val, &mut process) {
                            println!("{}", sym);
                        } else {
                            println!("No symbol found");
                        }
                    }
                }
                CommandExpr::Unassemble(_, expr) => {
                    if let Some(addr) = eval_expr(expr) {
                        next_unassemble_address = unassemble::unassemble(mem_source.as_ref(), addr, 16);
                    }
                }
                CommandExpr::UnassembleContinue(_) => {
                    next_unassemble_address = unassemble::unassemble(mem_source.as_ref(), next_unassemble_address, 16);
                }
                CommandExpr::ListSource(_, expr) => {
                    if let Some(val) = eval_expr(expr) {
                        match resolve_address_to_source_line(val, &mut process) {
                            Ok((file_name, line_number)) => {
                                println!("LSA: {}:{}", file_name, line_number);
                                if let Ok(file_name) = source::find_source_file_match(&file_name, &source_search_paths) {
                                    if let Ok(file) = File::open(&file_name) {
                                        println!("Found matching file: {}", file_name.display());
                                        let reader = io::BufReader::new(file);
                                        let lines: Vec<_> = reader.lines().map(|l| l.unwrap_or("".to_string())) .collect();
                                        for print_line_num in (max(1, line_number - 2))..=(min(lines.len() as u32, line_number + 2)) {
                                            if print_line_num == line_number {
                                                println!(">{:4}: {}", print_line_num, lines[print_line_num as usize - 1]);
                                            } else {
                                                println!("{:5}: {}", print_line_num, lines[print_line_num as usize - 1]);
                                            }
                                        }
                                    } else {
                                        println!("Couldn't open file: {}", file_name.display());
                                    }
                                }
                            },
                            Err(e) => {
                                println!("Couldn't look up source: {}", e);
                            }
                        }                        
                    }
                }
                CommandExpr::SrcPath(_, path) => {
                    source_search_paths.clear();
                    source_search_paths.extend(path.split(';').map(|s| s.to_string()));
                }
                CommandExpr::SetBreakpoint(_, expr) => {
                    if let Some(addr) = eval_expr(expr) {
                        breakpoints.add_breakpoint(addr);
                    }
                }
                CommandExpr::ListBreakpoints(_) => {
                    breakpoints.list_breakpoints(&mut process);
                }
                CommandExpr::ClearBreakpoint(_, expr) => {
                    if let Some(id) = eval_expr(expr) {
                        breakpoints.clear_breakpoint(id as u32);
                    }
                }
                CommandExpr::StackWalk(_) => {
                    let mut context = ctx.context.clone();
                    println!(" #   RSP              Call Site");
                    let mut frame_number = 0;
                    loop {
                        if let Some(sym) = name_resolution::resolve_address_to_name(context.Rip, &mut process) {
                            println!("{:02X} 0x{:016X} {}", frame_number, context.Rsp, sym);
                        } else {
                            println!("{:02X} 0x{:016X} 0x{:X}", frame_number, context.Rsp, context.Rip);
                        }
                        match stack::unwind_context(&mut process, context, mem_source.as_ref()) {
                            Ok(Some(unwound_context)) => context = unwound_context,
                            _ => break
                        }
                        frame_number += 1;
                    }
                }
                CommandExpr::Quit(_) => {
                    // The process will be terminated since we didn't detach.
                    return;
                }
            }
        }

        if is_exit {
            break;
        }

        breakpoints.apply_breakpoints(&mut process, event_context.thread_id, mem_source.as_ref());

        unsafe {
            ContinueDebugEvent(
                event_context.process_id,
                event_context.thread_id,
                continue_status,
            );
        }
    }
}

fn main() {
    let target_command_line_result = parse_command_line();

    let mut command_line_buffer = match target_command_line_result {
        Ok(i) => i,
        Err(msg) => {
            show_usage(msg);
            return;
        }
    };

    println!(
        "Command line was: '{str}'",
        str = String::from_utf16_lossy(&command_line_buffer)
    );

    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    let mut pi: MaybeUninit<PROCESS_INFORMATION> = MaybeUninit::uninit();
    let ret = unsafe {
        CreateProcessW(
            null(),
            command_line_buffer.as_mut_ptr(),
            null(),
            null(),
            FALSE,
            DEBUG_ONLY_THIS_PROCESS | CREATE_NEW_CONSOLE,
            null(),
            null(),
            &mut si.StartupInfo,
            pi.as_mut_ptr(),
        )
    };

    if ret == 0 {
        panic!("CreateProcessW failed");
    }

    let pi = unsafe { pi.assume_init() };

    unsafe { CloseHandle(pi.hThread) };

    // Later, we'll need to pass in a process handle.
    main_debugger_loop(pi.hProcess);

    unsafe { CloseHandle(pi.hProcess) };
}
