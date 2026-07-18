use clap::Parser;
use console::style;
use kaji::args::Cli;
use kaji::utils::ui::ERROR;

#[cfg(not(tarpaulin_include))]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    dotenvy::dotenv().ok();
    dotenvy::from_filename(".secrets").ok();
    env_logger::init();

    let cli = Cli::parse();

    if let Err(err) = kaji::run_app(cli).await {
        eprintln!("{} {}", ERROR, style("Error:").red().bold());
        for (i, cause) in err.chain().enumerate() {
            if i == 0 {
                eprintln!("  {}", style(cause).bold());
            } else {
                eprintln!("    {} {}", style("↳").dim(), cause);
            }
        }
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
