use windows_sys::{
    Win32::Foundation::*,
    Win32::System::Environment::*,
    Win32::System::{Diagnostics::Debug::*, Threading::*, WindowsProgramming::INFINITE},
};

use std::ptr::null;

// For now, we only accept the command line of the process to launch, so we'll just return that for now. Later, we can parse additional
// command line options such as attaching to processes.
fn parse_command_line() -> *mut u16 {
    unsafe {
        // As far as I can tell, standard rust command line argument parsing won't preserve spaces. So we'll call
        // the win32 api directly and then parse it out.
        let mut p = GetCommandLineW();

        // The command line will start with the path of the currently running executable, which may or may not be contained in quotes.
        // We'll skip that first

        if *p == '"' as u16 {
            // If we start with a quote like "foo/foo.exe", we should keep going until we get to the end of the quote
            p = p.offset(1);
            while (*p != 0) && (*p != '"' as u16) {
                p = p.offset(1);
            }

            if *p == '"' as u16 {
                p = p.offset(1);
            }
        } else {
            // Skip anything that isn't a space
            while (*p != 0) && (*p != ' ' as u16) {
                p = p.offset(1);
            }
        }

        // Skip any leading whitespace
        while *p == ' ' as u16 {
            p = p.offset(1);
        }

        p
    }
}

unsafe fn main_debugger_loop() {
    loop {
        let mut debug_event: DEBUG_EVENT = std::mem::zeroed();
        WaitForDebugEventEx(&mut debug_event, INFINITE);

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

        ContinueDebugEvent(
            debug_event.dwProcessId,
            debug_event.dwThreadId,
            DBG_EXCEPTION_NOT_HANDLED,
        );
    }
}

fn main() {
    unsafe {
        let target_command_line = parse_command_line();

        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ret = CreateProcessW(
            null(),
            target_command_line,
            null(),
            null(),
            FALSE,
            DEBUG_ONLY_THIS_PROCESS | CREATE_NEW_CONSOLE,
            null(),
            null(),
            &mut si.StartupInfo,
            &mut pi,
        );

        if ret == 0 {
            panic!("CreateProcessW failed");
        }

        CloseHandle(pi.hThread);

        // Later, we'll need to pass in a process handle.
        main_debugger_loop();

        CloseHandle(pi.hProcess);
    }
}
