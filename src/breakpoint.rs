use windows_sys::Win32::System::Diagnostics::Debug::GetThreadContext;
use windows_sys::Win32::System::Diagnostics::Debug::SetThreadContext;
use windows_sys::Win32::System::Diagnostics::Debug::CONTEXT;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::Foundation::*;
use num_traits::int::PrimInt;

use crate::memory::MemorySource;
use crate::process::Process;
use crate::name_resolution;
use crate::util::*;

const DR7_LEN_BIT: [usize; 4] = [19, 23, 27, 31];
const DR7_RW_BIT: [usize; 4] = [17, 21, 25, 29];
const DR7_LE_BIT: [usize; 4] = [0, 2, 4, 6];
const DR7_GE_BIT: [usize; 4] = [1, 3, 5, 7];

const DR7_LEN_SIZE: usize = 2;
const DR7_RW_SIZE: usize = 2;

const DR6_B_BIT: [usize; 4] = [0, 1, 2, 3];

const EFLAG_RF: usize = 16;

struct Breakpoint {
    addr: u64,
    id: u32,
}

pub struct BreakpointManager {
    breakpoints: Vec::<Breakpoint>,
}

fn set_bits<T: PrimInt>(val: &mut T, set_val: T, start_bit: usize, bit_count: usize) {
    // First, mask out the relevant bits
    let max_bits = std::mem::size_of::<T>() * 8;
    let mask: T = T::max_value() << (max_bits - bit_count);
    let mask: T = mask >> (max_bits - 1 - start_bit);
    let inv_mask = !mask;

    *val = *val & inv_mask;
    *val = *val | (set_val << (start_bit + 1 - bit_count));
}

fn get_bit<T: PrimInt>(val: T, bit_index: usize) -> bool {
    let mask = T::one() << bit_index;
    let masked_val = val & mask;
    masked_val != T::zero()
}

impl BreakpointManager {

    pub fn new() -> BreakpointManager {
        BreakpointManager { breakpoints: Vec::new() }
    }

    fn get_free_id(&self) -> u32 {
        for i in 0..4 {
            if self.breakpoints.iter().find(|&x| x.id == i).is_none() {
                return i;
            }
        }
        panic!("Too many breakpoints!")
    }

    pub fn add_breakpoint(&mut self, addr: u64) {
        self.breakpoints.push(Breakpoint{addr, id: self.get_free_id()});
        self.breakpoints.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn list_breakpoints(&self, process: &mut Process) {
        for bp in self.breakpoints.iter() {
            if let Some(sym) = name_resolution::resolve_address_to_name(bp.addr, process) {
                println!("{:3} {:#018x} ({})", bp.id, bp.addr, sym)
            } else {
                println!("{:3} {:#018x}", bp.id, bp.addr)
            }            
        }
    }

    pub fn clear_breakpoint(&mut self, id: u32) {
        self.breakpoints.retain(|x| x.id != id)
    }

    pub fn was_breakpoint_hit(&self, thread_context: &CONTEXT) -> Option<u32> {
        for idx in 0..self.breakpoints.len() {
            if get_bit(thread_context.Dr6, DR6_B_BIT[idx]) {
                return Some(idx as u32);
            }
        }
        None
    }

    pub fn apply_breakpoints(&mut self, process: &mut Process, resume_thread_id: u32, _memory_source: &dyn MemorySource) {

        for thread_id in process.iterate_threads() {
            let mut ctx: AlignedContext = unsafe { std::mem::zeroed() };
            ctx.context.ContextFlags = CONTEXT_ALL;            
            let thread = AutoClosedHandle(unsafe {
                OpenThread(
                    THREAD_GET_CONTEXT | THREAD_SET_CONTEXT,
                    FALSE,
                    *thread_id,
                )
            });
            let ret = unsafe { GetThreadContext(thread.handle(), &mut ctx.context) };

            if ret == 0 {
                println!("Could not get thread context of thread {:x}", thread_id);
                continue;
            }

            // Currently there is a limit of 4 breakpoints, since we are using hardware breakpoints.
            for idx in 0..4 {
                if self.breakpoints.len() > idx {
                    
                    set_bits(&mut ctx.context.Dr7, 0, DR7_LEN_BIT[idx], DR7_LEN_SIZE);
                    set_bits(&mut ctx.context.Dr7, 0, DR7_RW_BIT[idx], DR7_RW_SIZE);
                    set_bits(&mut ctx.context.Dr7, 1, DR7_LE_BIT[idx], 1);
                    match idx {
                        0 => ctx.context.Dr0 = self.breakpoints[idx].addr,
                        1 => ctx.context.Dr1 = self.breakpoints[idx].addr,
                        2 => ctx.context.Dr2 = self.breakpoints[idx].addr,
                        3 => ctx.context.Dr3 = self.breakpoints[idx].addr,
                        _ => (),
                    }
                } else {
                    // We'll assume that we own all breakpoints. This will cause problems with programs that expect to control their own debug registers.
                    // As a result, we'll disable any breakpoints that we aren't using.
                    set_bits(&mut ctx.context.Dr7, 0, DR7_LE_BIT[idx], 1);
                    break;
                }    
            }

            // This prevents the current thread from hitting a breakpoint on the current instruction
            if *thread_id == resume_thread_id {
                set_bits(&mut ctx.context.EFlags, 1, EFLAG_RF, 1);
            }

            let ret = unsafe { SetThreadContext(thread.handle(), &mut ctx.context) };
            if ret == 0 {
                println!("Could not set thread context of thread {:x}", thread_id);
            }

        }
    }
}
