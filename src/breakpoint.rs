use crate::process::Process;
use crate::name_resolution;

struct Breakpoint {
    addr: u64,
    id: u32,
}

pub struct BreakpointManager {
    breakpoints: Vec::<Breakpoint>,
}

impl BreakpointManager {

    pub fn new() -> BreakpointManager {
        BreakpointManager { breakpoints: Vec::new() }
    }

    fn get_free_id(&self) -> u32 {
        for i in 0..1024 {
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
}