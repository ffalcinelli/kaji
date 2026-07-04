use anyhow::Result;
use clap::Parser;
use kcd::args::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename(".secrets").ok();
    env_logger::init();

    let cli = Cli::parse();

    kcd::run_app(cli).await
}
