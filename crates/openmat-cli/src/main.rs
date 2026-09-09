use std::io;

fn main() {
    let stdout = io::stdout();
    let stderr = io::stderr();
    let exit = openmat_cli::run(std::env::args(), &mut stdout.lock(), &mut stderr.lock());
    if exit != 0 {
        std::process::exit(exit);
    }
}
