#![doc = "Transport-neutral `OpenMat` language services and an LSP stdio frontend."]

pub mod document;
mod index;
pub mod language;
pub mod protocol;
pub mod server;
pub mod transport;

use std::io::{self, BufReader};

pub use document::{
    Document, DocumentError, DocumentStore, WorkspaceContext, WorkspaceDocument, WorkspaceSnapshot,
};
pub use server::{ConnectionError, RunOutcome, Server, ServerState, run_connection};

/// Runs the language server over the process standard input and output.
///
/// This frontend never opens a network listener and writes protocol frames only
/// to standard output.
///
/// # Errors
///
/// Returns an error when stdio framing, reading, JSON serialization, or writing
/// fails.
pub fn run_stdio() -> Result<RunOutcome, ConnectionError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    run_connection(&mut reader, &mut writer)
}
