use core::ffi::c_void;
use windows_sys::{Win32::Foundation, Win32::System::Diagnostics::Debug::*};

pub trait MemorySource {
    // Read up to "len" bytes, and return Option<u8> to represent what bytes are available in the range
    fn read_memory(&self, address: u64, len: usize) -> Result<Vec<Option<u8>>, &'static str>;
    // Read up to "len" bytes, and stop at the first failure
    fn read_raw_memory(&self, address: u64, len: usize) -> Vec<u8>;
}

pub fn read_memory_array<T: Sized + Default>(
    source: &dyn MemorySource,
    address: u64,
    max_count: usize,
) -> Result<Vec<T>, &'static str> {
    let element_size = ::core::mem::size_of::<T>();
    let max_bytes = max_count * element_size;
    let raw_bytes = source.read_raw_memory(address, max_bytes);
    let mut data: Vec<T> = Vec::new();
    let mut offset: usize = 0;
    while offset + element_size <= raw_bytes.len() {
        let mut item: T = T::default();
        let dst: *mut u8 = unsafe { std::mem::transmute(&mut item) };
        let src = &raw_bytes[offset] as *const u8;
        unsafe { std::ptr::copy_nonoverlapping(src, dst, element_size) };
        data.push(item);
        offset += element_size;
    }

    Ok(data)
}

pub fn read_memory_full_array<T: Sized + Default>(
    source: &dyn MemorySource,
    address: u64,
    count: usize,
) -> Result<Vec<T>, &'static str> {
    let arr = read_memory_array(source, address, count)?;

    if arr.len() != count {
        Err("Could not read all items")
    } else {
        Ok(arr)
    }
}

pub fn read_memory_data<T: Sized + Default + Copy>(
    source: &dyn MemorySource,
    address: u64,
) -> Result<T, &'static str> {
    let data = read_memory_array::<T>(source, address, 1)?;
    Ok(data[0])
}

pub fn read_memory_string(
    source: &dyn MemorySource,
    address: u64,
    max_count: usize,
    is_wide: bool,
) -> Result<String, &'static str> {
    let result: String = if is_wide {
        let mut words = read_memory_array::<u16>(source, address, max_count)?;
        let null_pos = words.iter().position(|&v| v == 0);
        if let Some(null_pos) = null_pos {
            words.truncate(null_pos);
        }
        String::from_utf16_lossy(&words)
    } else {
        let mut bytes = read_memory_array::<u8>(source, address, max_count)?;
        let null_pos = bytes.iter().position(|&v| v == 0);
        if let Some(null_pos) = null_pos {
            bytes.truncate(null_pos);
        }
        // TODO: This is not quite right. Technically most strings read here are encoded as ASCII.
        String::from_utf8(bytes).unwrap()
    };
    Ok(result)
}

pub fn read_memory_string_indirect(
    source: &dyn MemorySource,
    address: u64,
    max_count: usize,
    is_wide: bool,
) -> Result<String, &'static str> {
    let string_address = read_memory_data::<u64>(source, address)?;
    read_memory_string(source, string_address, max_count, is_wide)
}

struct LiveMemorySource {
    hprocess: Foundation::HANDLE,
}

pub fn make_live_memory_source(hprocess: Foundation::HANDLE) -> Box<dyn MemorySource> {
    Box::new(LiveMemorySource { hprocess })
}

impl MemorySource for LiveMemorySource {
    fn read_memory(&self, address: u64, len: usize) -> Result<Vec<Option<u8>>, &'static str> {
        let mut buffer: Vec<u8> = vec![0; len];
        let mut data: Vec<Option<u8>> = vec![None; len];
        let mut offset: usize = 0;

        while offset < len {
            let mut bytes_read: usize = 0;
            let len_left = len - offset;
            let cur_address = address + (offset as u64);

            let result = unsafe {
                ReadProcessMemory(
                    self.hprocess,
                    cur_address as *const c_void,
                    buffer.as_mut_ptr() as *mut c_void,
                    len_left,
                    &mut bytes_read as *mut usize,
                )
            };

            if result == 0 {
                return Err("ReadProcessMemory failed");
            };

            for index in 0..bytes_read {
                let data_index = offset + index;
                data[data_index] = Some(buffer[index]);
            }

            if bytes_read > 0 {
                offset += bytes_read;
            } else {
                offset += 1;
            }
        }

        Ok(data)
    }

    fn read_raw_memory(&self, address: u64, len: usize) -> Vec<u8> {
        let mut buffer: Vec<u8> = vec![0; len];
        let mut bytes_read: usize = 0;

        let result = unsafe {
            ReadProcessMemory(
                self.hprocess,
                address as *const c_void,
                buffer.as_mut_ptr() as *mut c_void,
                len,
                &mut bytes_read as *mut usize,
            )
        };

        if result == 0 {
            bytes_read = 0;
        }

        buffer.truncate(bytes_read);

        buffer
    }
}
