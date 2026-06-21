fn main() {
    if let Err(error) = xtask::main_entry() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
