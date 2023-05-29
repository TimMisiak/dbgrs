use pdb::FallibleIterator;

use crate::{process::Process, module::{Export, ExportTarget}};

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