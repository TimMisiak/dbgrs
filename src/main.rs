use event::DebugEvent;
use memory::MemorySource;
use windows_sys::{
    Win32::Foundation::*,
    Win32::System::Environment::*,
    Win32::System::{Diagnostics::Debug::*, Threading::*},
};

use std::ptr::null;

mod command;
mod eval;
mod memory;
mod process;
mod registers;
mod module;
mod name_resolution;
mod event;
mod breakpoint;

use process::Process;
use command::grammar::CommandExpr;
use breakpoint::BreakpointManager;

// Not sure why these are missing from windows_sys, but the definitions are in winnt.h
const CONTEXT_AMD64: u32 = 0x00100000;
const CONTEXT_CONTROL: u32 = CONTEXT_AMD64 | 0x00000001;
const CONTEXT_INTEGER: u32 = CONTEXT_AMD64 | 0x00000002;
const CONTEXT_SEGMENTS: u32 = CONTEXT_AMD64 | 0x00000004;
const CONTEXT_FLOATING_POINT: u32 = CONTEXT_AMD64 | 0x00000008;
const CONTEXT_DEBUG_REGISTERS: u32 = CONTEXT_AMD64 | 0x00000010;
#[allow(dead_code)]
const CONTEXT_FULL: u32 = CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_FLOATING_POINT;
const CONTEXT_ALL: u32 = CONTEXT_CONTROL
    | CONTEXT_INTEGER
    | CONTEXT_SEGMENTS
    | CONTEXT_FLOATING_POINT
    | CONTEXT_DEBUG_REGISTERS;

const TRAP_FLAG: u32 = 1 << 8;

#[repr(align(16))]
struct AlignedContext {
    context: CONTEXT,
}

struct AutoClosedHandle(HANDLE);

impl Drop for AutoClosedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl AutoClosedHandle {
    pub fn handle(&self) -> HANDLE {
        self.0
    }
}

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

    loop {
        let (event_context, debug_event) = event::wait_for_next_debug_event(mem_source.as_ref());

        let mut continue_status = DBG_CONTINUE;
        let mut is_exit = false;
        match debug_event {
            DebugEvent::Exception { first_chance, exception_code } => {
                let chance_string = if first_chance {
                    "second chance"
                } else {
                    "first chance"
                };
    
                if expect_step_exception && exception_code == EXCEPTION_SINGLE_STEP {
                    expect_step_exception = false;
                    continue_status = DBG_CONTINUE;
                } else {
                    println!("Exception code {:x} ({})", exception_code, chance_string);
                    continue_status = DBG_EXCEPTION_NOT_HANDLED;
                }
            },
            DebugEvent::CreateProcess { exe_name, exe_base } => {
                load_module_at_address(&mut process, mem_source.as_ref(), exe_base, exe_name);
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

        let mut continue_execution = false;

        while !continue_execution {

            if let Some(sym) = name_resolution::resolve_address_to_name(ctx.context.Rip, &mut process) {
                println!("[{:X}] {}", event_context.thread_id, sym);
            } else {
                println!("[{:X}] {:#018x}", event_context.thread_id, ctx.context.Rip);
            }

            let cmd = command::read_command();

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
                    registers::display_all(ctx.context);
                }
                CommandExpr::DisplayBytes(_, expr) => {
                    let address = eval::evaluate_expression(*expr);
                    let bytes = mem_source.read_raw_memory(address, 16);
                    for byte in bytes {
                        print!("{:02X} ", byte);
                    }
                    println!();
                }
                CommandExpr::Evaluate(_, expr) => {
                    let val = eval::evaluate_expression(*expr);
                    println!(" = 0x{:X}", val);
                }
                CommandExpr::ListNearest(_, expr) => {
                    let val = eval::evaluate_expression(*expr);
                    if let Some(sym) = name_resolution::resolve_address_to_name(val, &mut process) {
                        println!("{}", sym);
                    } else {
                        println!("No symbol found");
                    }
                }
                CommandExpr::SetBreakpoint(_, expr) => {
                    let addr = eval::evaluate_expression(*expr);
                    breakpoints.add_breakpoint(addr)
                }
                CommandExpr::ListBreakpoints(_) => {
                    breakpoints.list_breakpoints(&mut process)
                }
                CommandExpr::ClearBreakpoint(_, expr) => {
                    let id = eval::evaluate_expression(*expr);
                    breakpoints.clear_breakpoint(id as u32);
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
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
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
            &mut pi,
        )
    };

    if ret == 0 {
        panic!("CreateProcessW failed");
    }

    unsafe { CloseHandle(pi.hThread) };

    // Later, we'll need to pass in a process handle.
    main_debugger_loop(pi.hProcess);

    unsafe { CloseHandle(pi.hProcess) };
}
