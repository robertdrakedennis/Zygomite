//! Decode an assembled .cs2 and assert every branch/switch target is in range.
//! Also prints a window of instructions around a given index for spot-checks.
//!
//! Usage: cargo run --example `verify_branch_targets` -- <file.cs2> [`window_lo`] [`window_hi`]

use rs3_cache_rs::script::{OpcodeBook, Operand, decode_script};
use std::path::Path;

fn main() -> rs3_cache_rs::error::Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: <cs2-file> [lo] [hi]");
    let lo: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    let hi: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let book = OpcodeBook::load(Path::new("data"), 910, 0)?;
    let data = std::fs::read(&path).unwrap();
    let s = decode_script(&data, &book, 910)?;
    let count = i32::try_from(s.code.len())?;
    println!("{path}: {count} instructions");

    let mut oob = 0;
    let mut branches = 0;
    let mut switches = 0;
    for (i, instr) in s.code.iter().enumerate() {
        match &instr.operand {
            Operand::Branch(t) => {
                branches += 1;
                if *t < 0 || *t > count {
                    oob += 1;
                    println!(
                        "  !!OOB branch @ instr {i} ({}) -> target {t}",
                        instr.command
                    );
                }
            }
            Operand::Switch(cases) => {
                switches += 1;
                for c in cases {
                    if c.target < 0 || c.target > count {
                        oob += 1;
                        println!(
                            "  !!OOB switch @ instr {i} case {} -> target {}",
                            c.value, c.target
                        );
                    }
                }
            }
            _ => {}
        }
    }
    println!("branches={branches} switches={switches} out_of_range={oob}");

    if lo != usize::MAX {
        println!("-- window [{lo}..{hi}] --");
        for (i, instr) in s.code.iter().enumerate() {
            if i >= lo && i <= hi {
                println!("  {i}: {} {:?}", instr.command, instr.operand);
            }
        }
    }

    if oob > 0 {
        std::process::exit(1);
    }
    println!("OK: all branch/switch targets in range");
    Ok(())
}
