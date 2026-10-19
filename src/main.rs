fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(aula_f87::cli::main(args));
}
