use pdb::FallibleIterator;
use pdb::SymbolData;

use crate::{process::Process, module::{Export, ExportTarget, Module}};
use anyhow::anyhow;

enum AddressMatch<'a> {
    None,
    Export(&'a Export),
    Public(String)
}
impl AddressMatch<'_> {
    fn is_none(&self) -> bool {
        match self {
            AddressMatch::None => true,
            _ => false
        }
    }
}

pub fn resolve_name_to_address(sym: &str, process: &mut Process) -> Result<u64, anyhow::Error> {
    match sym.chars().position(|c| c == '!') {
        None => {
            // Search all modules
            Err(anyhow!("Not yet implemented"))
        },
        Some(pos) => {
            let module_name = &sym[..pos];
            let func_name = &sym[pos + 1..];
            if let Some(module) = process.get_module_by_name_mut(module_name) {
                if let Some(addr) = resolve_function_in_module(module, func_name) {
                    Ok(addr)
                } else {
                    Err(anyhow!("Could not find {} in module {}", func_name, module_name))
                }
            } else {
                Err(anyhow!("Could not find module {}", module_name))
            }
        },
    }
}

pub fn resolve_function_in_module(module: &mut Module, func: &str) -> Option<u64> {
    // We'll search exports first and private symbols next
    let export_resolution = resolve_export_in_module(module, func);
    if export_resolution.is_some() {
        return export_resolution;
    }

    resolve_symbol_name_in_module(module, func).unwrap_or(None)
}

fn resolve_export_in_module(module: &mut Module, func: &str) -> Option<u64> {
    // We'll search exports first and private symbols next
    for export in module.exports.iter() {
        if let Some(export_name) = &export.name {
            if *export_name == *func {
                // Just support direct exports for now, rather than forwarded functions.
                if let ExportTarget::RVA(export_addr) = export.target {
                    return Some(export_addr)
                }
            }
        }
    }
    None
}

fn resolve_symbol_name_in_module(module: &mut Module, func: &str) -> Result<Option<u64>, anyhow::Error> {
    let pdb = module.pdb.as_mut().ok_or(anyhow!("No PDB loaded"))?;
    let dbi = pdb.debug_information()?;
    let mut modules = dbi.modules()?;
    let address_map = module.address_map.as_mut().ok_or(anyhow!("No address map available"))?;
    while let Some(pdb_module) = modules.next()? {
        let mi = pdb.module_info(&pdb_module)?.ok_or(anyhow!("Couldn't get module info"))?;
        let mut symbols = mi.symbols()?;
        while let Some(sym) = symbols.next()? {
            if let Ok(parsed) = sym.parse() {
                if let SymbolData::Procedure(proc_data) = parsed {
                    if proc_data.name.to_string() == func {
                        let rva = proc_data.offset.to_rva(address_map).ok_or(anyhow!("Couldn't convert procedure offset to RVA"))?;
                        let address = module.address + rva.0 as u64;
                        return Ok(Some(address));
                    }
                }
            }
        }
    }
    Ok(None)
}


pub fn resolve_address_to_name(address: u64, process: &mut Process) -> Option<String> {
    let module = match process.get_containing_module_mut(address) {
        Some(module) => module,
        None => return None
    };

    let mut closest: AddressMatch = AddressMatch::None;
    let mut closest_addr: u64 = 0;
    // This could be faster if we were always in sorted order
    for export in module.exports.iter() {
        if let ExportTarget::RVA(export_addr) = export.target {
            if export_addr <= address {
                if closest.is_none() || closest_addr < export_addr {
                    closest = AddressMatch::Export(export);
                    closest_addr = export_addr;
                }
            }
        };
    }

    if let Some(pdb) = module.pdb.as_mut() {
        if let Ok(symbol_table) = pdb.global_symbols() {
            if let Ok(address_map) = pdb.address_map() {
                let mut symbols = symbol_table.iter();
                while let Ok(Some(symbol)) = symbols.next() {
                    match symbol.parse() {
                        Ok(pdb::SymbolData::Public(data)) if data.function => {
                            let rva = data.offset.to_rva(&address_map).unwrap_or_default();
                            let global_addr = module.address + rva.0 as u64;
                            if global_addr <= address && (closest.is_none() || closest_addr <= global_addr) {
                                // TODO: Take a reference to the data?
                                closest = AddressMatch::Public(data.name.to_string().to_string());
                                closest_addr = global_addr;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if let AddressMatch::Export(closest) = closest {
        let offset = address - closest_addr;
        let sym_with_offset = if offset == 0 {
            format!("{}!{}", &module.name, closest.to_string())
        } else {
            format!("{}!{}+0x{:X}", &module.name, closest.to_string(), offset)
        };
        return Some(sym_with_offset)
    }

    if let AddressMatch::Public(closest) = closest {
        let offset = address - closest_addr;
        let sym_with_offset = if offset == 0 {
            format!("{}!{}", &module.name, closest)
        } else {
            format!("{}!{}+0x{:X}", &module.name, closest, offset)
        };
        return Some(sym_with_offset)
    }
    
    None
}