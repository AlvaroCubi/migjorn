use clap::{Parser, Subcommand};

pub mod commands {
    pub mod info;
    pub mod parsing_check;
    pub mod renumber;
}
mod utils;

#[derive(Parser)]
#[command(name = "migjorn", about = "MCNP model parser and utilities", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Info(commands::info::InfoArgs),
    Renumber(commands::renumber::RenumberArgs),
    ParsingCheck(commands::parsing_check::CheckArgs),
}

pub fn run(args: impl IntoIterator<Item = String>) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            e.print().ok();
            return e.exit_code();
        }
    };

    match cli.command {
        Command::Info(args) => commands::info::run(&args),
        Command::Renumber(args) => commands::renumber::run(&args),
        Command::ParsingCheck(args) => commands::parsing_check::run(&args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|&a| a.to_string()).collect()
    }

    #[test]
    fn help_returns_0() {
        assert_eq!(run(s(&["migjorn", "--help"])), 0);
    }

    #[test]
    fn version_returns_0() {
        assert_eq!(run(s(&["migjorn", "--version"])), 0);
    }

    #[test]
    fn unknown_flag_returns_nonzero() {
        assert_ne!(run(s(&["migjorn", "--unknown-flag"])), 0);
    }
}
