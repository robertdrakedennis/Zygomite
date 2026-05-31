//! Decode an assembled .cs2 and report its switch case table + references to the
//! skill-29 viewed-level varcs (6783/7292). Used to verify the `skillguide_initialise`
//! (5690) / script5712 switch edits didn't corrupt case targets.
//!
//! Usage: cargo run --example `decode_cs2` -- <file.cs2>

use rs3_cache_rs::script::{OpcodeBook, Operand, decode_script};
use std::path::Path;

fn main() -> rs3_cache_rs::error::Result<()> {
    let path = std::env::args().nth(1).expect("usage: <cs2-file>");
    let book = OpcodeBook::load(Path::new("data"), 910, 0)?;
    let data = std::fs::read(&path).unwrap();
    let s = decode_script(&data, &book, 910)?;
    println!("{path}: {} instructions", s.code.len());

    for (i, instr) in s.code.iter().enumerate() {
        if let Operand::Switch(cases) = &instr.operand {
            let mut vals: Vec<i32> = cases.iter().map(|c| c.value).collect();
            vals.sort_unstable();
            println!("switch @ instr {i}: {} cases, values {vals:?}", cases.len());
            let max = i32::try_from(s.code.len())?;
            for c in cases {
                let oob = if c.target < 0 || c.target > max {
                    " !!OUT-OF-RANGE"
                } else {
                    ""
                };
                if c.value == 28 || c.value == 29 || !oob.is_empty() {
                    println!("  case {} -> target {}{oob}", c.value, c.target);
                }
            }
        }
    }

    for (i, instr) in s.code.iter().enumerate() {
        if let Operand::VarRef(v) = &instr.operand
            && (v.id == 6783 || v.id == 7292)
        {
            println!(
                "  instr {i}: {} {}:{}",
                instr.command,
                v.domain.as_label(),
                v.id
            );
        }
    }
    Ok(())
}
