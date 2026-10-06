//! `genie` binary: see `genie --help`.

#[tokio::main(worker_threads = 2)]
async fn main() {
    if let Err(e) = genie::cli::run().await {
        eprintln!("genie: {e}");
        std::process::exit(1);
    }
}
