use windows_sys::Win32::Foundation;

use crate::{module::Module, memory::MemorySource};

pub struct Process {
    module_list: std::vec::Vec<Module>,
    thread_list: std::vec::Vec<u32>,
}

impl Process {
    pub fn new() -> Process {
        Process { module_list: Vec::new(), thread_list: Vec::new() }
    }

    pub fn add_module(&mut self, address: u64, name: Option<String>, memory_source: &dyn MemorySource) -> Result<&Module, &'static str> {
        let module = Module::from_memory_view(address, name, memory_source)?;
        self.module_list.push(module);
        Ok(self.module_list.last().unwrap())
    }

    pub fn add_thread(&mut self, thread_id: u32) {
        self.thread_list.push(thread_id);
    }

    pub fn remove_thread(&mut self, thread_id: u32) {
        self.thread_list.retain(|x| *x != thread_id);
    }

    pub fn iterate_threads(&self) -> core::slice::Iter<'_, u32> {
        self.thread_list.iter()
    }

    pub fn _get_containing_module(&self, address: u64) -> Option<&Module> {
        for module in self.module_list.iter() {
            if module.contains_address(address) {
                return Some(&module);
            }
        };

        None
    }

    pub fn get_containing_module_mut(&mut self, address: u64) -> Option<&mut Module> {
        for module in self.module_list.iter_mut() {
            if module.contains_address(address) {
                return Some(module);
            }
        };

        None
    }

    pub fn get_module_by_name_mut(&mut self, module_name: &str) -> Option<&mut Module> {
        let mut potential_trimmed_match = None;
        let mut potential_trimmed_noext_match = None;
    
        for module in self.module_list.iter_mut() {
            if module.name == module_name {
                return Some(module);
            }
    
            let trimmed = module.name.rsplitn(2, '\\').next().unwrap_or(&module.name);
            if potential_trimmed_match.is_none() && trimmed.to_lowercase() == module_name.to_lowercase() {
                potential_trimmed_match = Some(module);
            } else if potential_trimmed_noext_match.is_none() {
                let parts: Vec<&str> = trimmed.rsplitn(2, '.').collect();
                let trimmed_noext = if parts.len() == 2 {
                    parts[1]
                } else {
                    parts[0]
                };
                if trimmed_noext.to_lowercase() == module_name.to_lowercase() {
                    potential_trimmed_noext_match = Some(module);
                }
            }
        };
    
        potential_trimmed_match.or(potential_trimmed_noext_match)
    }
}
