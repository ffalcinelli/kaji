use console::style;
use kaji::args::Cli;
use kaji::utils::ui::WARN;

#[cfg(not(tarpaulin_include))]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse_with_sources();

    if let Err(err) = kaji::run_app(cli).await {
        if let Some(drift) = err.downcast_ref::<kaji::DriftDetected>() {
            eprintln!("{} {}", WARN, style(drift).yellow().bold());
            return std::process::ExitCode::from(2);
        }
        kaji::utils::ui::eprint_error(&err);
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
