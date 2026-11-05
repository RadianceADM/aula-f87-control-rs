#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(aula_f87::cli::main(args));
}

#[cfg(not(unix))]
fn main() {
    eprintln!("On Windows, build and run the WinUI 3 app in windows/ instead.");
    std::process::exit(2);
}
