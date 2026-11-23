//! `ev3-c`: EVƎ C from a terminal, an agent or the VS Code extension. This wires the
//! commands to the process: its arguments, folders, environment, output, Ctrl+C and exit code.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use ev3_c::args::wants_events;
use ev3_c::cli::{Client, Deps, EXIT_FAILED, EXIT_USAGE, Interrupts, Io, run_cli};
use ev3_c::cockpit::Unavailable;
use ev3_c::paths::{PACKAGE_ROOT_VARIABLE, package_root, real_path, toolchain_cache_dir};
use ev3c_core::port::SystemClock;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

fn main() {
    let code = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => {
            let local = tokio::task::LocalSet::new();
            let code = local.block_on(&runtime, run());
            drop(local);
            // Reading standard input blocks a thread that cannot be cancelled; waiting for it
            // would keep the program from ending.
            runtime.shutdown_background();
            code
        }
        Err(error) => {
            eprintln!("ev3 c: {error}");
            EXIT_FAILED
        }
    };
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}

async fn run() -> i32 {
    let mut argv = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.into_string() {
            Ok(arg) => argv.push(arg),
            Err(arg) => {
                eprintln!("ev3 c: the argument {} is not valid Unicode", arg.display());
                return EXIT_USAGE;
            }
        }
    }
    let deps = match deps() {
        Ok(deps) => deps,
        Err(message) => {
            eprintln!("ev3 c: {message}");
            return EXIT_FAILED;
        }
    };

    let (sender, receiver) = mpsc::unbounded_channel();
    if wants_events(&argv) {
        tokio::task::spawn_local(interrupts_from_stdin(sender.clone()));
    }
    tokio::task::spawn_local(async move {
        // Once listened for, Ctrl+C no longer ends the process: the command decides.
        while tokio::signal::ctrl_c().await.is_ok() {
            if sender.send(()).is_err() {
                break;
            }
        }
    });
    run_cli(
        &argv,
        &mut StdIo,
        &deps,
        &mut Interrupts::new(receiver),
        &Client::Terminal,
    )
    .await
}

/// Under --events, standard input stands for Ctrl+C, which a program that starts `ev3-c`
/// cannot send on every system: each line `interrupt` is one, and so is the end of input, so
/// a caller that goes away stops the run it started. Other lines are ignored.
async fn interrupts_from_stdin(sender: mpsc::UnboundedSender<()>) {
    let mut input = BufReader::new(tokio::io::stdin());
    let mut line = Vec::new();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => {
                let _ = sender.send(());
                return;
            }
            Ok(_) => {
                if String::from_utf8_lossy(&line).trim() == "interrupt" && sender.send(()).is_err()
                {
                    return;
                }
            }
        }
    }
}

fn deps() -> Result<Deps<Unavailable>, String> {
    let env: HashMap<String, String> = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    let cwd = std::env::current_dir()
        .map_err(|error| format!("The current folder cannot be read: {error}"))?;
    let package_root = std::env::current_exe()
        .map_err(|error| format!("The program's own path cannot be read: {error}"))
        .and_then(|program| {
            let program = real_path(&program);
            let folder = program.parent().map(PathBuf::from).unwrap_or_default();
            package_root(env.get(PACKAGE_ROOT_VARIABLE).map(String::as_str), &folder)
        });
    let home = home_dir(&env);
    let cache = toolchain_cache_dir(&|name| env.get(name).cloned(), std::env::consts::OS, &home);
    Ok(Deps {
        connector: Unavailable,
        clock: SystemClock,
        package_root,
        toolchain_cache_dir: cache,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        cwd,
        env,
        compile: None,
    })
}

/// The user's home folder, as Node's `os.homedir()` finds it.
fn home_dir(env: &HashMap<String, String>) -> PathBuf {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env.get(name)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Standard output and error, one line per message, flushed at once: a caller reading the
/// output as it comes sees each message when it is written.
struct StdIo;

impl Io for StdIo {
    fn out(&mut self, text: &str) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    }

    fn err(&mut self, text: &str) {
        let _ = writeln!(std::io::stderr().lock(), "{text}");
    }
}
