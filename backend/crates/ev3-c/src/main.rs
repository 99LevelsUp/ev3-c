//! `ev3-c`: EVƎ C from a terminal, an agent or the VS Code extension. The commands arrive with
//! the move of the TypeScript backend to Rust (issue #13); until then this prints its version.

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--version") => println!("{}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!(
                "ev3 c: the Rust commands are not built yet; use the TypeScript ev3-c until issue #13 is done."
            );
            std::process::exit(2);
        }
    }
}
