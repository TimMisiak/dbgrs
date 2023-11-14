use crate::memory::{*, self};
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64;
use windows::Win32::System::SystemServices::*;
use windows::Win32::System::Diagnostics::Debug::{*, IMAGE_DATA_DIRECTORY};
use pdb::PDB;
use std::fs::File;

pub struct Module {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub exports: Vec::<Export>,
    pub pdb_name: Option<String>,
    pub pdb_info: Option<PdbInfo>,
    pub pdb: Option<PDB<'static, File>>,
    pe_header: IMAGE_NT_HEADERS64,
}

pub struct Export {
    pub name: Option<String>,
    // This is the "biased" ordinal
    pub ordinal: u32,
    pub target: ExportTarget,
}

impl ToString for Export {
    fn to_string(&self) -> String {
        if let Some(str) = &self.name {
            str.to_string()
        } else {
            format!("Ordinal{}", self.ordinal)
        }
    }
}

pub enum ExportTarget {
    RVA(u64),
    Forwarder(String)
}

#[derive(Default)]
#[repr(C)]
pub struct PdbInfo {
    pub signature: u32,
    pub guid: windows::core::GUID,
    pub age: u32,
    // Null terminated name goes after the end
}

impl ::core::marker::Copy for PdbInfo {}
impl ::core::clone::Clone for PdbInfo {
    fn clone(&self) -> Self {
        *self
    }
}

impl Module {
    pub fn from_memory_view(module_address: u64, module_name: Option<String>, memory_source: &dyn MemorySource) -> Result<Module, &'static str> {

        let dos_header: IMAGE_DOS_HEADER = memory::read_memory_data(memory_source, module_address)?;

        // NOTE: Do we trust that the headers are accurate, even if it means we could read outside the bounds of the
        //       module? For this debugger, we'll trust the data, but a real debugger should do sanity checks and 
        //       report discrepancies to the user in some way.
        let pe_header_addr = module_address + dos_header.e_lfanew as u64;

        // NOTE: This should be IMAGE_NT_HEADERS32 for 32-bit modules, but the FileHeader lines up for both structures.
        let pe_header: IMAGE_NT_HEADERS64 = memory::read_memory_data(memory_source, pe_header_addr)?;
        let size = pe_header.OptionalHeader.SizeOfImage as u64;

        if pe_header.FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 {
            return Err("Unsupported machine architecture for module");
        }
        
        let (pdb_info, pdb_name, pdb) = Module::read_debug_info(&pe_header, module_address, memory_source)?;
        let (exports, export_table_module_name) = Module::read_exports(&pe_header, module_address, memory_source)?;

        let module_name = module_name.or(export_table_module_name);
         let module_name = match module_name {
            Some(s) => s,
            None => {
                format!("module_{:X}", module_address)
            }
        };

        Ok(Module{
            name: module_name,
            address: module_address,
            size,
            exports,
            pdb_info,
            pdb_name,
            pdb,
            pe_header
        })
    }

    pub fn contains_address(&self, address: u64) -> bool {
        let end = self.address + self.size;
        self.address <= address && address < end
    }

    fn read_debug_info(pe_header: &IMAGE_NT_HEADERS64, module_address: u64, memory_source: &dyn MemorySource) -> Result<(Option<PdbInfo>, Option<String>, Option<PDB<'static, File>>), &'static str> {
        let mut pdb_info: Option<PdbInfo> = None;
        let mut pdb_name: Option<String> = None;
        let mut pdb: Option<PDB<File>> = None;
        

        let debug_table_info = pe_header.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_DEBUG.0 as usize];
        if debug_table_info.VirtualAddress != 0 {
            let dir_size = std::mem::size_of::<IMAGE_DEBUG_DIRECTORY>() as u64;
            // We'll arbitrarily limit to 20 entries to keep it sane.
            let count: u64 = std::cmp::min(debug_table_info.Size as u64 / dir_size, 20);
            for dir_index in 0..count {
                let debug_directory_address = module_address + (debug_table_info.VirtualAddress as u64) + (dir_index * dir_size);
                let debug_directory: IMAGE_DEBUG_DIRECTORY = memory::read_memory_data(memory_source, debug_directory_address)?;
                if debug_directory.Type == IMAGE_DEBUG_TYPE_CODEVIEW {
                    let pdb_info_address = debug_directory.AddressOfRawData as u64 + module_address;
                    pdb_info = Some(memory::read_memory_data(memory_source, pdb_info_address)?);
                    // We could check that pdb_info.signature is RSDS here.
                    let pdb_name_address = pdb_info_address + std::mem::size_of::<PdbInfo>() as u64;
                    let max_size = debug_directory.SizeOfData as usize - std::mem::size_of::<PdbInfo>();
                    pdb_name = Some(memory::read_memory_string(memory_source, pdb_name_address, max_size, false)?);

                    let pdb_file = File::open(pdb_name.as_ref().unwrap());
                    if let Ok(pdb_file) = pdb_file {
                        let pdb_data = PDB::open(pdb_file);
                        if let Ok(pdb_data) = pdb_data {
                            pdb = Some(pdb_data);
                        }
                    }
                }
            }
        }

        Ok((pdb_info, pdb_name, pdb))
    }

    pub fn get_data_directory(&self, entry: IMAGE_DIRECTORY_ENTRY) -> IMAGE_DATA_DIRECTORY {
        self.pe_header.OptionalHeader.DataDirectory[entry.0 as usize]
    }

    fn read_exports(pe_header: &IMAGE_NT_HEADERS64, module_address: u64, memory_source: &dyn MemorySource) -> Result<(Vec::<Export>, Option<String>), &'static str> {
        let mut exports = Vec::<Export>::new();
        let mut module_name: Option<String> = None;
        let export_table_info = pe_header.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_EXPORT.0 as usize];
        if export_table_info.VirtualAddress != 0 {
            let export_table_addr = module_address + export_table_info.VirtualAddress as u64;
            let export_table_end = export_table_addr + export_table_info.Size as u64;
            let export_directory: IMAGE_EXPORT_DIRECTORY = memory::read_memory_data(memory_source, export_table_addr)?;

            // This is a fallback that lets us find a name if none was available.
            if export_directory.Name != 0 {
                let name_addr = module_address + export_directory.Name as u64;
                module_name = Some(memory::read_memory_string(memory_source, name_addr, 512, false)?);
            }

            // We'll read the name table first, which is essentially a list of (ordinal, name) pairs that give names 
            // to some or all of the exports. The table is stored as parallel arrays of orindals and name pointers
            let ordinal_array_address = module_address + export_directory.AddressOfNameOrdinals as u64;
            let ordinal_array = memory::read_memory_full_array::<u16>(memory_source, ordinal_array_address, export_directory.NumberOfNames as usize)?;
            let name_array_address = module_address + export_directory.AddressOfNames as u64;
            let name_array = memory::read_memory_full_array::<u32>(memory_source, name_array_address, export_directory.NumberOfNames as usize)?;

            let address_table_address = module_address + export_directory.AddressOfFunctions as u64;
            let address_table = memory::read_memory_full_array::<u32>(memory_source, address_table_address, export_directory.NumberOfFunctions as usize)?;

            for (unbiased_ordinal, function_address) in address_table.iter().enumerate() {
                let ordinal = export_directory.Base + unbiased_ordinal as u32;
                let target_address = module_address + *function_address as u64;

                let name_index = ordinal_array.iter().position(|&o| o == unbiased_ordinal as u16);
                let export_name = match name_index {
                    None => None,
                    Some(idx) => {
                        let name_address = module_address + name_array[idx] as u64;
                        Some(memory::read_memory_string(memory_source, name_address, 4096, false)?)
                    }
                };

                // An address that falls inside the export directory is actually a forwarder
                if target_address >= export_table_addr && target_address < export_table_end {
                    // I don't know that there actually is a max size for a forwader name, but 4K is probably reasonable.
                    let forwarding_name = memory::read_memory_string(memory_source, target_address, 4096, false)?;
                    exports.push(Export {name: export_name, ordinal, target: ExportTarget::Forwarder(forwarding_name)});                    
                } else {
                    exports.push(Export{name: export_name, ordinal, target: ExportTarget::RVA(target_address)});
                }
            }
        };

        Ok((exports, module_name))
    }
}