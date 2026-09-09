use std::io;

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let exit = openmat_server::run(std::env::args(), stdin.lock(), &mut stdout, &mut stderr);
    if exit != 0 {
        std::process::exit(exit);
    }
}
