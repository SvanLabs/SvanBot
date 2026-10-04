//! `sv10-release release|update|rollback …`: the three surfaces of the installer.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(sv10_release::cli::run(&args));
}
