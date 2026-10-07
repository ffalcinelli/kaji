use console::style;
use kaji::args::Cli;
use kaji::utils::ui::{ERROR, INFO, WARN};

#[cfg(not(tarpaulin_include))]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    dotenvy::dotenv().ok();
    dotenvy::from_filename(".secrets").ok();
    env_logger::init();

    let cli = Cli::parse_with_sources();

    if let Err(err) = kaji::run_app(cli).await {
        if let Some(drift) = err.downcast_ref::<kaji::DriftDetected>() {
            eprintln!("{} {}", WARN, style(drift).yellow().bold());
            return std::process::ExitCode::from(2);
        }
        eprintln!("{} {}", ERROR, style("Error:").red().bold());
        for (i, cause) in err.chain().enumerate() {
            let cause_str = cause.to_string();
            if cause_str.starts_with("Hint:") {
                eprintln!("\n{} {}", INFO, style(cause_str).blue());
            } else if i == 0 {
                eprintln!("  {}", style(cause_str).bold());
            } else {
                eprintln!("    {} {}", style("↳").dim(), cause_str);
            }
        }
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
