use anyhow::Result;
use clap::Parser;
use kcd::args::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename(".secrets").ok();
    env_logger::init();

    let mut cli = Cli::parse();

    // Load skipped fields from environment if not provided
    if cli.password.is_none() {
        cli.password = std::env::var("KEYCLOAK_PASSWORD").ok();
    }
    if cli.client_secret.is_none() {
        cli.client_secret = std::env::var("KEYCLOAK_CLIENT_SECRET").ok();
    }

    kcd::run_app(cli).await
}
