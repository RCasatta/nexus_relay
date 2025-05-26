use clap::Parser;
use nexus_relay::{async_main, Config};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let config = Config::parse();

    // Create a runtime for async code execution
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Run the async main function with the parsed config
    rt.block_on(async_main(config))
}
