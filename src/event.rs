use std::os::windows::prelude::OsStringExt;

use windows_sys::Win32::{System::{Diagnostics::Debug::{DEBUG_EVENT, WaitForDebugEventEx, EXCEPTION_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, CREATE_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT, UNLOAD_DLL_DEBUG_EVENT, OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT}, Threading::INFINITE}, Storage::FileSystem::GetFinalPathNameByHandleW};

use crate::memory::{MemorySource, self};

#[allow(non_snake_case)]

pub enum DebugEvent {
    Exception{first_chance: bool, exception_code: i32},
    CreateProcess{exe_name: Option<String>, exe_base: u64},
    LoadModule{module_name: Option<String>, module_base: u64},
    OutputDebugString(String),
    ExitProcess,
    Other(String)
}

pub struct EventContext {
    pub process_id: u32,
    pub thread_id: u32,
}

pub fn wait_for_next_debug_event(mem_source: &dyn MemorySource) -> (EventContext, DebugEvent) {
    let mut debug_event: DEBUG_EVENT = unsafe { std::mem::zeroed() };
    unsafe {
        WaitForDebugEventEx(&mut debug_event, INFINITE);
    }

    let ctx = EventContext{ process_id: debug_event.dwProcessId, thread_id: debug_event.dwThreadId };

    match debug_event.dwDebugEventCode {
        EXCEPTION_DEBUG_EVENT => {
            let code = unsafe { debug_event.u.Exception.ExceptionRecord.ExceptionCode };
            let first_chance = unsafe { debug_event.u.Exception.dwFirstChance };
            (ctx, DebugEvent::Exception { first_chance: first_chance != 0, exception_code: code })
        },
        CREATE_THREAD_DEBUG_EVENT => (ctx, DebugEvent::Other("CreateThread".to_string())),
        CREATE_PROCESS_DEBUG_EVENT => {
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
            
            //load_module_at_address(&mut process, mem_source.as_ref(), exe_base, exe_name);
            (ctx, DebugEvent::CreateProcess { exe_name, exe_base })
        },
        EXIT_THREAD_DEBUG_EVENT => (ctx, DebugEvent::Other("ExitThread".to_string())),
        EXIT_PROCESS_DEBUG_EVENT => (ctx, DebugEvent::ExitProcess),
        LOAD_DLL_DEBUG_EVENT => {
            let load_dll = unsafe { debug_event.u.LoadDll };
            let module_base: u64 = load_dll.lpBaseOfDll as u64;
            let module_name = if load_dll.lpImageName == std::ptr::null_mut() {
                None
            } else {
                let is_wide = load_dll.fUnicode != 0;
                memory::read_memory_string_indirect(mem_source, load_dll.lpImageName as u64, 260, is_wide)
                    .map_or(None, |x| Some(x))
            };

            //load_module_at_address(&mut process, mem_source.as_ref(), dll_base, dll_name);
            (ctx, DebugEvent::LoadModule { module_name, module_base })
        }
        UNLOAD_DLL_DEBUG_EVENT => (ctx, DebugEvent::Other("UnloadDll".to_string())),
        OUTPUT_DEBUG_STRING_EVENT => {
            let debug_string_info = unsafe { debug_event.u.DebugString };
            let is_wide = debug_string_info.fUnicode != 0;
            let address = debug_string_info.lpDebugStringData as u64;
            let len = debug_string_info.nDebugStringLength as usize;
            let debug_string =
                memory::read_memory_string(mem_source, address, len, is_wide).unwrap();
            (ctx, DebugEvent::OutputDebugString(debug_string))
        }
        RIP_EVENT => (ctx, DebugEvent::Other("RipEvent".to_string())),
        _ => panic!("Unexpected debug event"),
    }
}