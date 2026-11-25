//! `xsysroot` CLI entry point.

use std::process::ExitCode;

fn main() -> ExitCode {
    println!("xsysroot {}", xsysroot::version());
    ExitCode::SUCCESS
}
