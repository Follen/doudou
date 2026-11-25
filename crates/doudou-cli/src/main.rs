//! `doudou` binary entrypoint.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let code = doudou_cli::run().await;
    u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from)
}
