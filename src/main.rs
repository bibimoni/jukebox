use clap::Parser;
use jukebox::cli::{self, Cmd};
use jukebox::config;

fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    match args.cmd.unwrap_or(Cmd::Play) {
        Cmd::Config { args } => {
            let _cfg = cli::ensure_config()?;
            if !args.is_empty() {
                eprintln!("config edits are not yet supported; edit {}", config::config_path().display());
            } else {
                println!("config: {}", config::config_path().display());
            }
        }
        Cmd::Play => { eprintln!("(TUI not implemented yet)"); }
        Cmd::Sync => { eprintln!("(sync not implemented yet)"); }
        Cmd::Index => { eprintln!("(index not implemented yet)"); }
        Cmd::Search { query } => { eprintln!("search: {}", query.join(" ")); }
    }
    Ok(())
}
