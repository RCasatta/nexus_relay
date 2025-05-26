use clap::Parser;
use env_logger::{self, Env};
use nexus_relay::{async_main, Config};
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with systemd support
    init_logging();

    // Parse command line arguments
    let config = Config::parse();

    // Create a runtime for async code execution
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Run the async main function with the parsed config
    rt.block_on(async_main(config))
}

fn init_logging() {
    let mut builder = env_logger::Builder::from_env(Env::default().default_filter_or("info"));
    if let Ok(s) = std::env::var("RUST_LOG_STYLE") {
        if s == "SYSTEMD" {
            builder.format(|buf, record| {
                let level = match record.level() {
                    log::Level::Error => 3,
                    log::Level::Warn => 4,
                    log::Level::Info => 6,
                    log::Level::Debug => 7,
                    log::Level::Trace => 7,
                };
                writeln!(buf, "<{}>{}: {}", level, record.target(), record.args())
            });
        }
    }

    builder.init();
}
