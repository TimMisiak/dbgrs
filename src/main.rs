use windows_sys::{
    Win32::Foundation::*,
    Win32::System::Environment::*,
    Win32::System::{Diagnostics::Debug::*, Threading::*, WindowsProgramming::INFINITE},
};

use std::ptr::null;

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

fn main_debugger_loop() {
    loop {
        let mut debug_event: DEBUG_EVENT = unsafe { std::mem::zeroed() };
        unsafe {
            WaitForDebugEventEx(&mut debug_event, INFINITE);
        }

        match debug_event.dwDebugEventCode {
            EXCEPTION_DEBUG_EVENT => println!("Exception"),
            CREATE_THREAD_DEBUG_EVENT => println!("CreateThread"),
            CREATE_PROCESS_DEBUG_EVENT => println!("CreateProcess"),
            EXIT_THREAD_DEBUG_EVENT => println!("ExitThread"),
            EXIT_PROCESS_DEBUG_EVENT => println!("ExitProcess"),
            LOAD_DLL_DEBUG_EVENT => println!("LoadDll"),
            UNLOAD_DLL_DEBUG_EVENT => println!("UnloadDll"),
            OUTPUT_DEBUG_STRING_EVENT => println!("OutputDebugString"),
            RIP_EVENT => println!("RipEvent"),
            _ => panic!("Unexpected debug event"),
        }

        if debug_event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT {
            break;
        }

        unsafe {
            ContinueDebugEvent(
                debug_event.dwProcessId,
                debug_event.dwThreadId,
                DBG_EXCEPTION_NOT_HANDLED,
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
    main_debugger_loop();

    unsafe { CloseHandle(pi.hProcess) };
}
