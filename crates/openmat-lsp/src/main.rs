use openmat_lsp::{RunOutcome, run_stdio};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_stdio() {
        Ok(RunOutcome::CleanExit | RunOutcome::EndOfStream) => ExitCode::SUCCESS,
        Ok(RunOutcome::ExitWithoutShutdown) => ExitCode::from(1),
        Err(error) => {
            eprintln!("openmat-lsp: {error}");
            ExitCode::from(2)
        }
    }
}
