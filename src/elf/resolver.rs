use std::fmt::Debug;
use std::fmt::Formatter;
use std::fmt::Result as FmtResult;
use std::io::Result;
use std::path::Path;
use std::path::PathBuf;

use crate::inspect::FindAddrOpts;
use crate::inspect::SymInfo;
use crate::symbolize::AddrLineInfo;
use crate::Addr;
use crate::SymResolver;

use super::cache::ElfBackend;
use super::types::PT_LOAD;
use super::types::STT_FUNC;
use super::ElfParser;

/// The symbol resolver for a single ELF file.
///
/// An ELF file may be loaded into an address space with a relocation.
/// The callers should provide the path of an ELF file and where it's
/// executable segment(s) is loaded.
///
/// For some ELF files, they are located at a specific address
/// determined during compile-time.  For these cases, just pass `0` as
/// it's loaded address.
pub struct ElfResolver {
    backend: ElfBackend,
    file_name: PathBuf,
}

impl ElfResolver {
    pub(crate) fn with_backend(file_name: &Path, backend: ElfBackend) -> Result<ElfResolver> {
        Ok(ElfResolver {
            backend,
            file_name: file_name.to_path_buf(),
        })
    }

    fn get_parser(&self) -> &ElfParser {
        match &self.backend {
            #[cfg(feature = "dwarf")]
            ElfBackend::Dwarf(dwarf) => dwarf.get_parser(),
            ElfBackend::Elf(parser) => parser,
        }
    }
}

impl SymResolver for ElfResolver {
    // TODO: Need to better handle errors.
    fn find_syms(&self, addr: Addr) -> Vec<(&str, Addr)> {
        let parser = self.get_parser();
        if let Ok(Some((name, start_addr))) = parser.find_sym(addr, STT_FUNC) {
            // We found the address in ELF.
            // TODO: Long term we probably want a different heuristic here, as
            //       there can be valid differences between the two formats
            //       (e.g., DWARF could contain more symbols).
            return vec![(name, start_addr)]
        }

        match &self.backend {
            #[cfg(feature = "dwarf")]
            ElfBackend::Dwarf(dwarf) => dwarf.find_syms(addr).unwrap_or_default(),
            ElfBackend::Elf(_) => Vec::new(),
        }
    }

    fn find_addr(&self, name: &str, opts: &FindAddrOpts) -> Result<Vec<SymInfo>> {
        let parser = self.get_parser();
        let syms = parser.find_addr(name, opts)?;
        if !syms.is_empty() {
            // We found symbols in ELF and DWARF wouldn't add information on
            // top. So just roll with that.
            return Ok(syms)
        }

        match &self.backend {
            #[cfg(feature = "dwarf")]
            ElfBackend::Dwarf(dwarf) => dwarf.find_addr(name, opts),
            ElfBackend::Elf(_) => Ok(Vec::new()),
        }
    }

    #[cfg(feature = "dwarf")]
    fn find_line_info(&self, addr: Addr) -> Result<Option<AddrLineInfo>> {
        if let ElfBackend::Dwarf(dwarf) = &self.backend {
            let addr_line_info = dwarf
                .find_line(addr)?
                .map(|(dir, file, line)| AddrLineInfo {
                    path: dir.join(file),
                    line,
                    column: 0,
                });
            Ok(addr_line_info)
        } else {
            Ok(None)
        }
    }

    #[cfg(not(feature = "dwarf"))]
    fn find_line_info(&self, addr: Addr) -> Result<Option<AddrLineInfo>> {
        Ok(None)
    }

    /// Find the file offset of the symbol at address `addr`.
    // TODO: See if we could make this a constant time calculation by supplying
    //       the ELF symbol index (and potentially an offset from it) [this will
    //       require a bit of a larger rework, including on call sites].
    fn addr_file_off(&self, addr: Addr) -> Option<u64> {
        let addr = addr as u64;
        let parser = self.get_parser();
        let phdrs = parser.program_headers().ok()?;
        let offset = phdrs.iter().find_map(|phdr| {
            if phdr.p_type == PT_LOAD {
                if (phdr.p_vaddr..phdr.p_vaddr + phdr.p_memsz).contains(&addr) {
                    return Some(addr - phdr.p_vaddr + phdr.p_offset)
                }
            }
            None
        })?;
        Some(offset)
    }

    fn get_obj_file_name(&self) -> &Path {
        &self.file_name
    }
}

impl Debug for ElfResolver {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self.backend {
            #[cfg(feature = "dwarf")]
            ElfBackend::Dwarf(_) => write!(f, "DWARF {}", self.file_name.display()),
            ElfBackend::Elf(_) => write!(f, "ELF {}", self.file_name.display()),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;
    use std::rc::Rc;


    /// Check that we fail finding an offset for an address not
    /// representing a symbol in an ELF file.
    #[test]
    fn addr_without_offset() {
        let path = Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("test-stable-addresses-no-dwarf.bin");
        let elf = ElfParser::open(&path).unwrap();
        let backend = ElfBackend::Elf(Rc::new(elf));
        let resolver = ElfResolver::with_backend(&path, backend).unwrap();

        assert_eq!(resolver.addr_file_off(0x0), None);
        assert_eq!(resolver.addr_file_off(0xffffffffffffffff), None);
    }
}
