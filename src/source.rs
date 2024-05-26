use std::path::{Path, PathBuf};

use pdb::{FallibleIterator, Rva};
use anyhow::{Result, anyhow};

use crate::process::Process;


pub fn resolve_address_to_source_line(address: u64, process: &mut Process) -> Result<(String, u32)> {
    let module = process.get_containing_module_mut(address).ok_or(anyhow!("Module not found"))?;
    let pdb = module.pdb.as_mut().ok_or(anyhow!("Symbols not available"))?;

    let address_map = module.address_map.as_mut().ok_or(anyhow!("Address map not found for module"))?;
    let rva: u32 = (address - module.address).try_into()?;
    let rva = Rva(rva);
    let offset = rva.to_internal_offset(address_map).ok_or(anyhow!("Couldn't map address"))?;

    let dbi = pdb.debug_information()?;
    let mut modules = dbi.modules()?;
    while let Some(module) = modules.next()? {
        if let Ok(Some(mi)) = pdb.module_info(&module) {
            if let Ok(lp) = mi.line_program() {
                let mut lines = lp.lines_for_symbol(offset);
                while let Some(line) = lines.next()? {
                    if line.offset.offset <= offset.offset {
                        let diff = offset.offset - line.offset.offset;
                        if diff < line.length.unwrap_or(0) {
                            let file_info = lp.get_file_info(line.file_index)?;
                            let string_table = pdb.string_table()?;
                            let file_name = string_table.get(file_info.name)?;
                            return Ok((file_name.to_string().into(), line.line_start))
                        }
                        
                    }
                }
            }
        }
    }
    Err(anyhow!("Address not found"))
}

pub fn find_source_file_match(file: &str, search_paths: &Vec<String>) -> Result<PathBuf> {
    let file_path = Path::new(file);

    // If the file path is absolute and exists, return it immediately.
    if file_path.is_absolute() && file_path.exists() {
        return Ok(file_path.to_path_buf());
    }

    // Get all subsets of the input path.
    let components: Vec<&str> = file_path.components().map(|c| c.as_os_str().to_str().unwrap()).collect();

    for search_path in search_paths {
        let search_path = Path::new(search_path);

        for i in 0..components.len() {
            // Join the search path with the subset of the input path.
            let test_path: PathBuf = search_path.join(components[i..].iter().collect::<PathBuf>());
            if test_path.exists() {
                return Ok(test_path.to_path_buf());
            }
        }
    }

    Err(anyhow!("File not found"))
}