//! Generate shell completions from the same Clap command tree used at runtime.

use std::io;

use clap::Command;
use clap_complete::{generate, Shell};

pub fn print_completions(shell: &str, mut command: Command) {
    let shell = match shell {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        "powershell" | "pwsh" => Shell::PowerShell,
        _ => {
            eprintln!("Supported shells: zsh, bash, powershell, fish");
            return;
        }
    };

    generate(shell, &mut command, "rssh", &mut io::stdout());
}
