use iced_x86::{Decoder, DecoderOptions, Formatter, Instruction, MasmFormatter};

use crate::memory::MemorySource;

pub fn unassemble(memory_source: &dyn MemorySource, va: u64, lines: usize) {

    // read one page
    let rw = memory_source.read_memory(va, 0x1000);
    if !rw.is_ok() {
        println!("Failed to read memory at {:X}", va);
        return;
    }
    let bytes = rw.unwrap();

    // convert Vec<Option<u8>> to Vec<u8>
    let mut bytes_read = vec![];
    for b in bytes {
        if let Some(b) = b {
            bytes_read.push(b);
        }
    }

    let code_bitness = 64;
    let hexbytes_column_byte_length = 10;
    let mut decoder = Decoder::with_ip(
        code_bitness,
        bytes_read.as_slice(),
        va,
        DecoderOptions::NONE,
    );

    // Formatters: Masm*, Nasm*, Gas* (AT&T) and Intel* (XED).
    // For fastest code, see `SpecializedFormatter` which is ~3.3x faster. Use it if formatting
    // speed is more important than being able to re-assemble formatted instructions.
    let mut formatter = MasmFormatter::new();

    // Change some options, there are many more
    //formatter.options_mut().set_digit_separator("`");
    formatter.options_mut().set_first_operand_char_index(10);

    // String implements FormatterOutput
    let mut output = String::new();

    // Initialize this outside the loop because decode_out() writes to every field
    let mut instruction = Instruction::default();

    // The decoder also implements Iterator/IntoIterator so you could use a for loop:
    //      for instruction in &mut decoder { /* ... */ }
    // or collect():
    //      let instructions: Vec<_> = decoder.into_iter().collect();
    // but can_decode()/decode_out() is a little faster:
    let mut instruction_count = 0;
    while decoder.can_decode() && instruction_count < lines {
        // There's also a decode() method that returns an instruction but that also
        // means it copies an instruction (40 bytes):
        //     instruction = decoder.decode();
        decoder.decode_out(&mut instruction);

        // Format the instruction ("disassemble" it)
        output.clear();
        formatter.format(&instruction, &mut output);

        // Eg. "00007FFAC46ACDB2 488DAC2400FFFFFF     lea       rbp,[rsp-100h]"
        print!("{:016X} ", instruction.ip());
        let start_index = (instruction.ip() - va) as usize;
        let instr_bytes = &bytes_read[start_index..start_index + instruction.len()];
        for b in instr_bytes.iter() {
            print!("{:02X}", b);
        }
        if instr_bytes.len() < hexbytes_column_byte_length {
            for _ in 0..hexbytes_column_byte_length - instr_bytes.len() {
                print!("  ");
            }
        }
        println!(" {}", output);
        instruction_count += 1;
    }
}
