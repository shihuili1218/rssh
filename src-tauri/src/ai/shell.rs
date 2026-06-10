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
    /// A raw serial port (UART / RS-232) — NOT a shell. No `;` separator, no
    /// `echo`, no exit code. Behind it may be a bootloader, an MCU/RTOS console,
    /// or an AT-command modem. Commands are pasted verbatim (no sentinel) and
    /// completion is signaled by the user, not by marker output. Most of the
    /// machinery on this enum (sentinel, exit code) is deliberately inert here.
    Serial,
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
        // Serial has no shell: no `;`/`echo`/`$?` to carry a marker. Paste the
        // raw command; completion comes from the user (manual submit), not from
        // a sentinel line in the output. Empty marker = nothing to grep for.
        if self == Self::Serial {
            return (String::new(), cmd.to_string());
        }
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
            // PS 5+: `;` separates, `Write-Output` emits the string.
            // `$LASTEXITCODE` is set ONLY by *native* (external) programs; after
            // a pure cmdlet (Get-X / Set-X — exactly what `prompt_section` steers
            // the LLM toward) it is left untouched, and on a fresh session it is
            // `$null`. `$null` interpolates to empty, so the naive
            // `marker:$LASTEXITCODE` yields `marker:` (no digit) → the front-end
            // sentinel regex `:(-?\d+)` never matches → the command false-times-out
            // after 60s even though it succeeded.
            //
            // Fix: reset `$LASTEXITCODE` first (so a prior native command's code
            // can't leak into a later cmdlet), then coalesce to a digit — use the
            // native exit code when one exists, else the cmdlet success boolean
            // `$?` (0 = ok, 1 = failed). The field is therefore always numeric.
            //
            // `$?` is captured into `$ok` *immediately* after `{cmd}`: `$?`
            // reflects the most recently executed pipeline, and the intervening
            // `$null -ne $LASTEXITCODE` comparison can overwrite it (true) before
            // the `elseif` reads it — older PowerShell (5.1 / Desktop, which we
            // detect and support) is especially loose here — so a FAILED cmdlet
            // could be misreported as `0`. Snapshotting removes that ambiguity.
            Self::Powershell => format!(
                "$LASTEXITCODE=$null; {cmd}; $ok=$?; Write-Output \"{marker}:$(if ($null -ne $LASTEXITCODE) {{$LASTEXITCODE}} elseif ($ok) {{0}} else {{1}})\""
            ),
            // Serial: no shell to interpret a sentinel — the raw command is the
            // whole line. Reached only via direct calls; `sentinel_command`
            // short-circuits Serial before it gets here.
            Self::Serial => cmd.to_string(),
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
            Self::Serial => "serial device",
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
            // Serial: no shell, no statement separator — we instruct the LLM to
            // send ONE command per line, so a newline is the only "separator".
            Self::Serial => "\n",
        }
    }

    /// Render the "# Target shell" section appended to the system prompt.
    /// The LLM uses this to (a) pick shell-appropriate command syntax and
    /// (b) pick the right separator when batching multiple commands into a
    /// single `cmd` argument. Update side-by-side with the templates in
    /// `format_sentinel` if you change conventions.
    pub fn prompt_section(self) -> String {
        // Serial has no shell — its own guidance, not the (examples/exit_var/tips)
        // shape the shell families share. Steers the LLM away from shell syntax,
        // chaining, and any reliance on the (meaningless) exit code.
        if let Self::Serial = self {
            return "\n---\n\n# Target: serial device\n\n\
                 This session is a **raw serial port** (UART / RS-232), NOT a shell. Behind it \
                 may be a bootloader (U-Boot), an MCU / RTOS console, an AT-command modem, or any \
                 embedded device. There is no POSIX or Windows shell.\n\n\
                 - **One command per line.** Do NOT chain commands with `;`, `&&`, `||`, `|`, or any \
                 other separator — there is no shell to parse them. Each `run_command` must carry a \
                 single device command.\n\
                 - **No exit code.** Serial devices report no exit status; the `exit=` field in the \
                 result is a placeholder, NOT a success signal. Judge success ONLY from the output text.\n\
                 - **Completion is user-driven.** Your command is pasted to the port, then the user \
                 watches the device and submits the output once it has finished responding. Read the \
                 returned output as the literal device response.\n\
                 - **No shell or filesystem features.** No pipes, redirects, `$(...)`, globbing, or file \
                 operations exist on a bare serial device. Use only plain device commands via run_command.\n"
                .to_string();
        }
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
            Self::Serial => unreachable!("serial handled by the early return above"),
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
    fn format_sentinel_powershell_guards_null_lastexitcode() {
        // Regression: a pure cmdlet (Get-Process) leaves $LASTEXITCODE at $null,
        // which interpolates to an empty exit-code field — the front-end sentinel
        // regex `:(-?\d+)` then never matches and the command false-times-out after
        // 60s. The template must (a) reset $LASTEXITCODE so a prior native command's
        // code can't leak into a later cmdlet, (b) snapshot the cmdlet success flag
        // into $ok *right after* the command (before the `$null -ne` comparison can
        // clobber $?), and (c) coalesce to a digit so the field is always numeric.
        let got = ShellKind::Powershell.format_sentinel("Get-Process", "__rssh_done_abc");
        assert!(got.starts_with("$LASTEXITCODE=$null;"), "got: {got}");
        assert!(
            got.contains("Get-Process; $ok=$?;"),
            "must snapshot $? right after cmd: {got}"
        );
        assert!(got.contains("__rssh_done_abc:"));
        assert!(got.contains("$LASTEXITCODE")); // native exit code when one exists
        assert!(
            got.contains("elseif ($ok)"),
            "elseif must read the snapshot, not raw $?: {got}"
        );
    }

    #[test]
    fn from_local_path_unix() {
        assert_eq!(ShellKind::from_local_path("/bin/bash"), ShellKind::Posix);
        assert_eq!(
            ShellKind::from_local_path("/usr/local/bin/zsh"),
            ShellKind::Posix
        );
        assert_eq!(
            ShellKind::from_local_path("/opt/homebrew/bin/fish"),
            ShellKind::Posix
        );
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
            ShellKind::from_local_path(
                "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"
            ),
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
        assert!(ShellKind::Powershell
            .format_sentinel("x", "M")
            .contains(";"));
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
    fn serial_sentinel_is_raw_command_no_marker() {
        // Serial has no shell — sentinel_command must NOT wrap the command and
        // must return an empty marker. The front-end greps for nothing; the user
        // signals completion manually. This is the whole point of the variant.
        let (marker, full) = ShellKind::Serial.sentinel_command("printenv");
        assert_eq!(marker, "");
        assert_eq!(full, "printenv");
    }

    #[test]
    fn serial_format_sentinel_is_passthrough() {
        // Direct call (edge): the raw command is the entire line, marker ignored.
        assert_eq!(
            ShellKind::Serial.format_sentinel("reset", "__rssh_done_x"),
            "reset"
        );
    }

    #[test]
    fn serial_prompt_section_steers_off_shell_syntax() {
        // The LLM picks behavior from this section. For serial it MUST: declare
        // the serial environment, forbid command chaining, and warn the exit code
        // is meaningless. If a refactor strips these, the LLM reverts to POSIX
        // guesses on a device that has no shell.
        let p = ShellKind::Serial.prompt_section();
        assert!(p.contains("serial"));
        assert!(p.contains("One command per line"));
        assert!(p.contains("No exit code"));
        // must show the separators it forbids
        assert!(p.contains(';') && p.contains("&&"));
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

        // Serial joins the wire vocabulary as "serial".
        let se: ShellKind = serde_json::from_str("\"serial\"").unwrap();
        assert_eq!(se, ShellKind::Serial);
        assert_eq!(serde_json::to_string(&ShellKind::Serial).unwrap(), "\"serial\"");
    }
}
