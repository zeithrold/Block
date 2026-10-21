//! A deterministic process fixture. It never starts Java or accesses the network.

use std::{
    fs,
    path::Path,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

fn wait_for_release(directory: &Path) -> bool {
    let started = Instant::now();
    while !directory.join("release").exists() {
        if started.elapsed() > Duration::from_secs(15) {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
    true
}

fn run() -> std::io::Result<ExitCode> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| std::io::Error::other("fixture executable has no parent"))?;
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.iter().any(|value| value == "-version") {
        let fail = directory.join("probe-hold").exists();
        if fail && !wait_for_release(directory) {
            return Ok(ExitCode::FAILURE);
        }
        eprintln!("java.version = {}", if fail { "8.0.1" } else { "21.0.1" });
        eprintln!("os.arch = {}", std::env::consts::ARCH);
        return Ok(ExitCode::SUCCESS);
    }
    let token = arguments
        .windows(2)
        .find_map(|pair| (pair[0] == "--accessToken").then_some(pair[1].as_str()))
        .ok_or_else(|| std::io::Error::other("fixture did not receive its synthetic token"))?;
    fs::write(directory.join("java.pid"), std::process::id().to_string())?;
    println!("Fixture stdout token={token}");
    eprintln!("Fixture stderr token={token}");
    if !wait_for_release(directory) {
        // Failed tests must not leave an endless orphan or open a help window.
        fs::write(directory.join("java.exited"), b"watchdog")?;
        return Ok(ExitCode::SUCCESS);
    }
    let crash = fs::read_to_string(directory.join("mode"))? == "crash";
    if crash {
        fs::create_dir_all("crash-reports")?;
        fs::write(
            "crash-reports/crash-fixture-current-client.txt",
            format!("java.lang.OutOfMemoryError: Java heap space\naccessToken={token}\n"),
        )?;
        fs::write(
            format!("hs_err_pid{}.log", std::process::id()),
            format!("Fixture JVM report accessToken={token}\n"),
        )?;
        eprintln!("java.lang.OutOfMemoryError: Java heap space token={token}");
    }
    fs::write(directory.join("java.exited"), b"terminal")?;
    Ok(if crash {
        ExitCode::from(17)
    } else {
        ExitCode::SUCCESS
    })
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
