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
    let string_table = pdb.string_table().unwrap();

    while let Some(module) = modules.next()? {
        let mi = pdb.module_info(&module).unwrap().unwrap();
        let lp = mi.line_program().unwrap();
        let mut lines = lp.lines_for_symbol(offset);
        while let Some(line) = lines.next()? {
            if line.offset.offset <= offset.offset {
                let diff = offset.offset - line.offset.offset;
                if diff < line.length.unwrap_or(0) {
                    let file_info = lp.get_file_info(line.file_index).unwrap();
                    let file_name = string_table.get(file_info.name).unwrap();
                    return Ok((file_name.to_string().into(), line.line_start))
                }
                
            }
        }
    }
    Err(anyhow!("Address not found"))
}
