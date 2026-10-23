//! A native process fixture for pipe, cancellation and launch-context regressions.

use std::{fs, process::ExitCode, thread, time::Duration};

fn run() -> std::io::Result<ExitCode> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| std::io::Error::other("fixture executable has no parent"))?;
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    match fs::read_to_string(directory.join("fixture-mode"))?.as_str() {
        "cancel" => {
            for argument in arguments {
                println!("{argument}");
            }
            eprintln!("ready");
            // A broken test must not leave a persistent child behind.
            thread::sleep(Duration::from_secs(15));
            Ok(ExitCode::FAILURE)
        }
        "context" => {
            println!("{}", std::env::current_dir()?.canonicalize()?.display());
            println!(
                "{}",
                std::env::var("BLOCK_TEST_RENDERER").unwrap_or_default()
            );
            for argument in arguments.iter().skip(1) {
                println!("{argument}");
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(std::io::Error::other("unknown fixture mode")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Fixture failed: {error}");
            ExitCode::FAILURE
        }
    }
}
