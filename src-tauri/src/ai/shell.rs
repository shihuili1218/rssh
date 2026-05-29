//! Remote / local shell classification + sentinel command templates.
//!
//! Why this module exists: AI's `run_command` tool pastes a sentinel-bearing
//! line into the user's interactive terminal to mark "this command finished
//! with exit code N". The sentinel syntax is shell-dependent — POSIX uses
//! `; echo "X:$?"`, cmd.exe uses `& echo X:%errorlevel%`, PowerShell uses
//! `; Write-Output "X:$LASTEXITCODE"`. Hard-coding POSIX broke Windows
//! remotes (cmd.exe / PowerShell hosts).
//!
//! Three families cover >99% of real-world shells. fish / csh / nushell
//! fall back to POSIX template — sentinel may fail there (we lose exit
//! code) but the command itself still runs; the user sees the output in
//! the terminal even if the AI doesn't pick it up.

use serde::{Deserialize, Serialize};

/// Which shell family the target session speaks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    /// bash / zsh / sh / dash / ksh / csh / tcsh / fish (best-effort).
    /// `;` separator, `$?` exit code, `echo "X"` strips quotes.
    #[default]
    Posix,
    /// Windows cmd.exe — `&` separator, `%errorlevel%` exit code,
    /// `echo "X"` keeps the quotes (so we don't quote).
    Cmd,
    /// PowerShell 5/7 — `;` separator, `$LASTEXITCODE` exit code
    /// (NOT `$?` which is a boolean in PS), `Write-Output` for clarity.
    Powershell,
}

impl ShellKind {
    /// Generate a fresh sentinel marker (UUID-simple) and the full command
    /// line to paste into the user's terminal. Returns `(marker, full_cmd)`.
    ///
    /// Why this pair must be atomic: marker is what the front-end greps
    /// for in PTY output; full_cmd is what carries the marker into shell
    /// output. If a caller generated a marker but forgot to use it when
    /// building full_cmd (or vice versa), the front-end would silently
    /// timeout instead of seeing a sentinel. Returning both from one
    /// function makes that bug unreachable.
    pub fn sentinel_command(self, cmd: &str) -> (String, String) {
        let marker = format!("__rssh_done_{}", uuid::Uuid::new_v4().simple());
        let full = self.format_sentinel(cmd, &marker);
        (marker, full)
    }

    /// Build the line we paste into the user's terminal: their command +
    /// a marker echo line that carries the exit code. The front-end
    /// `findSentinel` regex `<marker>:(-?\d+)` matches the marker
    /// regardless of surrounding quotes.
    ///
    /// Most callers want `sentinel_command()` (which also generates the
    /// marker UUID). This raw form exists for tests that need a fixed
    /// marker for assertion.
    pub fn format_sentinel(self, cmd: &str, marker: &str) -> String {
        match self {
            Self::Posix => format!("{cmd}; echo \"{marker}:$?\""),
            // cmd.exe's `echo "X"` prints the literal quotes. Skip them.
            // `&` is the unconditional separator (analogue of POSIX `;`).
            // `&&` would short-circuit on first failure, hiding the exit code.
            Self::Cmd => format!("{cmd} & echo {marker}:%errorlevel%"),
            // PS 5+: `;` separates, `Write-Output` is the canonical way to
            // emit a string. `$LASTEXITCODE` is the exit code of the last
            // *native* command (the `cmd` part); `$?` in PS is boolean and
            // would output `True`/`False` — front-end regex wouldn't match.
            Self::Powershell => {
                format!("{cmd}; Write-Output \"{marker}:$LASTEXITCODE\"")
            }
        }
    }

    /// Short ASCII label used in audit logs and the system-prompt "# Target
    /// shell" section. Natural casing — POSIX is the canonical acronym,
    /// PowerShell is the product name — both are grep-stable so don't lowercase.
    pub fn name(self) -> &'static str {
        match self {
            Self::Posix => "POSIX (bash / zsh / sh)",
            Self::Cmd => "cmd.exe (Windows)",
            Self::Powershell => "PowerShell",
        }
    }

    /// Statement separator the LLM should use when packing multiple commands
    /// into one `cmd` argument. Reminder: rssh does NOT split — LLM is the
    /// one composing the line, so it needs to pick the right glue itself.
    pub fn separator(self) -> &'static str {
        match self {
            // POSIX `;` ignores prior failure (each cmd runs regardless),
            // analogue is what we want for diagnose batches.
            Self::Posix | Self::Powershell => ";",
            // cmd.exe `;` is literal; `&` is the unconditional separator.
            Self::Cmd => "&",
        }
    }

    /// Render the "# Target shell" section appended to the system prompt.
    /// The LLM uses this to (a) pick shell-appropriate command syntax and
    /// (b) pick the right separator when batching multiple commands into a
    /// single `cmd` argument. Update side-by-side with the templates in
    /// `format_sentinel` if you change conventions.
    pub fn prompt_section(self) -> String {
        let (examples, exit_var, tips) = match self {
            Self::Posix => (
                "`ps aux`, `df -h`, `which python3`, `cat /etc/os-release`",
                "`$?`",
                "Standard POSIX shell. `;` separates statements; `&&` short-circuits on failure. \
                 Pipes (`|`), redirects (`>`, `2>&1`), and command substitution (`$(...)`) all work as usual.",
            ),
            Self::Cmd => (
                "`ipconfig`, `tasklist`, `dir`, `where python`, `systeminfo`",
                "`%errorlevel%`",
                "Windows cmd.exe. **Do NOT use POSIX syntax** — `;` is literal, `$?` is undefined, `$VAR` is not expanded. \
                 Use `%VAR%` for environment variables (e.g. `%USERPROFILE%`). \
                 Use `&` to separate statements (run both regardless); `&&` for short-circuit on success. \
                 No pipes-of-pipes tricks; redirect to `nul` not `/dev/null`.",
            ),
            Self::Powershell => (
                "`Get-Process`, `Get-NetIPAddress`, `Get-Command python`, `Get-ChildItem`, `$PSVersionTable`",
                "`$LASTEXITCODE`",
                "Windows PowerShell. `;` separates statements. \
                 **Critical**: `$?` is a BOOLEAN (True/False), NOT the exit code — use `$LASTEXITCODE` for the numeric exit code of native commands. \
                 Variables are `$Name`. Use cmdlets (`Get-X` / `Set-X`) over external binaries when both exist. \
                 Redirect to `$null` (PS) not `/dev/null`.",
            ),
        };
        format!(
            "\n---\n\n# Target shell\n\n\
             The remote session is running **{name}**.\n\n\
             - Use shell-appropriate command syntax. Typical commands here: {examples}.\n\
             - When you need multiple commands in one `cmd` argument, separate them with `{sep}` (rssh does not split — you compose the line).\n\
             - Exit code variable for this shell: {exit_var}.\n\
             - {tips}\n",
            name = self.name(),
            sep = self.separator(),
            examples = examples,
            exit_var = exit_var,
            tips = tips,
        )
    }

    /// Map a local shell binary path (as remembered by `PtyHandle`) to a
    /// ShellKind. Used only for local PTY tabs where the shell is already
    /// known — no probing needed.
    pub fn from_local_path(path: &str) -> Self {
        // File-name basename, case-insensitive. Path may be Unix-style
        // (`/bin/bash`), Windows-style (`C:\Windows\System32\cmd.exe`),
        // or just a bare name. `.exe` suffix is optional.
        let base = path
            .rsplit(|c| c == '/' || c == '\\')
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase();
        let stem = base.strip_suffix(".exe").unwrap_or(&base);
        match stem {
            "cmd" => Self::Cmd,
            "powershell" | "pwsh" => Self::Powershell,
            _ => Self::Posix, // bash, zsh, fish, sh, dash, ksh, nu, xonsh, ...
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_sentinel_posix() {
        let got = ShellKind::Posix.format_sentinel("ls -la", "__rssh_done_abc");
        assert_eq!(got, "ls -la; echo \"__rssh_done_abc:$?\"");
    }

    #[test]
    fn format_sentinel_cmd_no_quotes() {
        // cmd's `echo "X"` would print the literal quotes — the template
        // intentionally omits them. `&` is the unconditional separator.
        let got = ShellKind::Cmd.format_sentinel("ipconfig", "__rssh_done_abc");
        assert_eq!(got, "ipconfig & echo __rssh_done_abc:%errorlevel%");
    }

    #[test]
    fn format_sentinel_powershell_uses_lastexitcode() {
        // `$?` in PS is boolean (True/False) — front-end regex `:(-?\d+)`
        // wouldn't match. Must use `$LASTEXITCODE`.
        let got = ShellKind::Powershell.format_sentinel("Get-Process", "__rssh_done_abc");
        assert_eq!(
            got,
            "Get-Process; Write-Output \"__rssh_done_abc:$LASTEXITCODE\""
        );
    }

    #[test]
    fn from_local_path_unix() {
        assert_eq!(ShellKind::from_local_path("/bin/bash"), ShellKind::Posix);
        assert_eq!(ShellKind::from_local_path("/usr/local/bin/zsh"), ShellKind::Posix);
        assert_eq!(ShellKind::from_local_path("/opt/homebrew/bin/fish"), ShellKind::Posix);
        assert_eq!(ShellKind::from_local_path("/bin/sh"), ShellKind::Posix);
    }

    #[test]
    fn from_local_path_windows_cmd() {
        assert_eq!(
            ShellKind::from_local_path("C:\\Windows\\System32\\cmd.exe"),
            ShellKind::Cmd
        );
        assert_eq!(ShellKind::from_local_path("cmd.exe"), ShellKind::Cmd);
        // Case-insensitive: Windows file-system is case-insensitive,
        // user-typed paths may have any casing.
        assert_eq!(ShellKind::from_local_path("CMD.EXE"), ShellKind::Cmd);
    }

    #[test]
    fn from_local_path_windows_powershell() {
        assert_eq!(
            ShellKind::from_local_path("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            ShellKind::Powershell
        );
        assert_eq!(
            ShellKind::from_local_path("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
            ShellKind::Powershell
        );
        assert_eq!(ShellKind::from_local_path("pwsh"), ShellKind::Powershell);
    }

    #[test]
    fn from_local_path_unknown_falls_back_to_posix() {
        // Git Bash on Windows, WSL, xonsh, nushell — all POSIX-ish enough
        // that the POSIX template won't break the actual command.
        assert_eq!(
            ShellKind::from_local_path("C:\\Program Files\\Git\\bin\\bash.exe"),
            ShellKind::Posix
        );
        assert_eq!(ShellKind::from_local_path("/usr/bin/nu"), ShellKind::Posix);
        assert_eq!(ShellKind::from_local_path("xonsh.exe"), ShellKind::Posix);
    }

    #[test]
    fn prompt_section_mentions_right_keywords() {
        // The LLM picks shell-appropriate syntax based on this section, so it
        // MUST contain the distinguishing tokens. If a future refactor strips
        // any of these, the LLM may revert to POSIX guesses on Windows targets
        // — which is the exact bug this whole module exists to fix.
        let posix = ShellKind::Posix.prompt_section();
        assert!(posix.contains("POSIX"));
        assert!(posix.contains("$?"));

        let cmd = ShellKind::Cmd.prompt_section();
        assert!(cmd.contains("cmd.exe"));
        assert!(cmd.contains("%errorlevel%"));
        assert!(cmd.contains("&"));
        assert!(cmd.contains("ipconfig") || cmd.contains("tasklist"));

        let ps = ShellKind::Powershell.prompt_section();
        assert!(ps.contains("PowerShell"));
        assert!(ps.contains("$LASTEXITCODE"));
        // PowerShell's `$?` is the booby trap — the section MUST warn the LLM.
        assert!(ps.contains("$?"));
    }

    #[test]
    fn separator_matches_sentinel_template() {
        // Sanity: the separator we tell the LLM to use must match the one
        // our own sentinel template uses. Otherwise the LLM would write
        // `cmd1; cmd2` thinking it's cmd.exe, while we'd append our sentinel
        // with `&` — half POSIX, half cmd — neither shell would parse it.
        assert!(ShellKind::Posix.format_sentinel("x", "M").contains(";"));
        assert!(ShellKind::Cmd.format_sentinel("x", "M").contains(" & "));
        assert!(ShellKind::Powershell.format_sentinel("x", "M").contains(";"));
        assert_eq!(ShellKind::Posix.separator(), ";");
        assert_eq!(ShellKind::Cmd.separator(), "&");
        assert_eq!(ShellKind::Powershell.separator(), ";");
    }

    #[test]
    fn default_is_posix() {
        // The safe fallback when we don't know the shell. POSIX is the
        // majority case (Linux/macOS SSH remotes), and the POSIX template
        // at worst loses the sentinel match on fish/csh — it doesn't break
        // the command itself.
        assert_eq!(ShellKind::default(), ShellKind::Posix);
    }

    #[test]
    fn sentinel_command_marker_is_unique() {
        // Two consecutive calls must produce different markers — otherwise
        // overlapping in-flight commands would alias each other's results.
        let (m1, _) = ShellKind::Posix.sentinel_command("ls");
        let (m2, _) = ShellKind::Posix.sentinel_command("ls");
        assert_ne!(m1, m2);
        assert!(m1.starts_with("__rssh_done_"));
        assert!(m2.starts_with("__rssh_done_"));
    }

    #[test]
    fn sentinel_command_marker_appears_in_full_cmd() {
        // The marker must show up verbatim inside full_cmd, otherwise the
        // front-end can never match the sentinel. This is the invariant
        // that makes the (marker, full_cmd) pair "atomic".
        let (m, full) = ShellKind::Cmd.sentinel_command("ipconfig");
        assert!(full.contains(&m), "marker {m:?} missing from full {full:?}");
    }

    #[test]
    fn serde_lowercase_roundtrip() {
        // Wire format: lowercase strings. Front-end sends "posix"/"cmd"/
        // "powershell" via the Tauri command; serde rename_all matches.
        let p: ShellKind = serde_json::from_str("\"posix\"").unwrap();
        let c: ShellKind = serde_json::from_str("\"cmd\"").unwrap();
        let s: ShellKind = serde_json::from_str("\"powershell\"").unwrap();
        assert_eq!(p, ShellKind::Posix);
        assert_eq!(c, ShellKind::Cmd);
        assert_eq!(s, ShellKind::Powershell);

        assert_eq!(serde_json::to_string(&ShellKind::Cmd).unwrap(), "\"cmd\"");
    }
}
