use memory::MemorySource;
use windows_sys::{
    Win32::Foundation::*,
    Win32::{System::Environment::*, Storage::FileSystem::{GetFinalPathNameByHandleW}},
    Win32::System::{Diagnostics::Debug::*, Threading::*},
};

use std::{ptr::null, os::windows::prelude::OsStringExt};

mod command;
mod eval;
mod memory;
mod process;
mod registers;
mod module;
mod name_resolution;

use process::Process;
use command::grammar::CommandExpr;

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

    loop {
        let mut debug_event: DEBUG_EVENT = unsafe { std::mem::zeroed() };
        unsafe {
            WaitForDebugEventEx(&mut debug_event, INFINITE);
        }

        let mut continue_status = DBG_CONTINUE;

        match debug_event.dwDebugEventCode {
            EXCEPTION_DEBUG_EVENT => {
                let code = unsafe { debug_event.u.Exception.ExceptionRecord.ExceptionCode };
                let first_chance = unsafe { debug_event.u.Exception.dwFirstChance };
                let chance_string = if first_chance == 0 {
                    "second chance"
                } else {
                    "first chance"
                };

                if expect_step_exception && code == EXCEPTION_SINGLE_STEP {
                    expect_step_exception = false;
                    continue_status = DBG_CONTINUE;
                } else {
                    println!("Exception code {:x} ({})", code, chance_string);
                    continue_status = DBG_EXCEPTION_NOT_HANDLED;
                }
            }
            CREATE_THREAD_DEBUG_EVENT => println!("CreateThread"),
            CREATE_PROCESS_DEBUG_EVENT => {
                println!("CreateProcess");
                let create_process = unsafe { debug_event.u.CreateProcessInfo };
                let exe_base = create_process.lpBaseOfImage as u64;
                let mut exe_name = vec![0u16; 260];
                let exe_name_len = unsafe { GetFinalPathNameByHandleW(create_process.hFile, exe_name.as_mut_ptr(), 260, 0) } as usize;
                let exe_name = if exe_name_len != 0 {
                    // This will be the full name, e.g. \\?\C:\git\HelloWorld\hello.exe
                    // It might be useful to have the full name, but it's not available for all
                    // modules in all cases.
                    let full_path = std::ffi::OsString::from_wide(&exe_name[0..exe_name_len]);
                    let file_name = std::path::Path::new(&full_path).file_name();

                    match file_name {
                        None => None,
                        Some(s) => Some(s.to_string_lossy().to_string())
                    }
                } else {
                    None
                };
                
                load_module_at_address(&mut process, mem_source.as_ref(), exe_base, exe_name);
            },
            EXIT_THREAD_DEBUG_EVENT => println!("ExitThread"),
            EXIT_PROCESS_DEBUG_EVENT => println!("ExitProcess"),
            LOAD_DLL_DEBUG_EVENT => {
                let load_dll = unsafe { debug_event.u.LoadDll };
                let dll_base: u64 = load_dll.lpBaseOfDll as u64;
                let dll_name = if load_dll.lpImageName == std::ptr::null_mut() {
                    None
                } else {
                    let is_wide = load_dll.fUnicode != 0;
                    memory::read_memory_string_indirect(mem_source.as_ref(), load_dll.lpImageName as u64, 260, is_wide)
                        .map_or(None, |x| Some(x))
                };

                load_module_at_address(&mut process, mem_source.as_ref(), dll_base, dll_name);
            }
            UNLOAD_DLL_DEBUG_EVENT => println!("UnloadDll"),
            OUTPUT_DEBUG_STRING_EVENT => {
                let debug_string_info = unsafe { debug_event.u.DebugString };
                let is_wide = debug_string_info.fUnicode != 0;
                let address = debug_string_info.lpDebugStringData as u64;
                let len = debug_string_info.nDebugStringLength as usize;
                let debug_string =
                    memory::read_memory_string(mem_source.as_ref(), address, len, is_wide).unwrap();
                println!("DebugOut: {}", debug_string);
            }
            RIP_EVENT => println!("RipEvent"),
            _ => panic!("Unexpected debug event"),
        }

        let thread = AutoClosedHandle(unsafe {
            OpenThread(
                THREAD_GET_CONTEXT | THREAD_SET_CONTEXT,
                FALSE,
                debug_event.dwThreadId,
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
                println!("[{:X}] {}", debug_event.dwThreadId, sym);
            } else {
                println!("[{:X}] {:#018x}", debug_event.dwThreadId, ctx.context.Rip);
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
                CommandExpr::Quit(_) => {
                    // The process will be terminated since we didn't detach.
                    return;
                }
            }
        }

        if debug_event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT {
            break;
        }

        unsafe {
            ContinueDebugEvent(
                debug_event.dwProcessId,
                debug_event.dwThreadId,
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
