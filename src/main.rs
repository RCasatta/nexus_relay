use nexus_relay::async_main;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a runtime for async code execution
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Run the async main function
    rt.block_on(async_main())
}
