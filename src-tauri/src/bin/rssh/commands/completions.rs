//! 输出 zsh / bash / powershell / fish 补全脚本。
//! 静态字符串，没有运行时分支 —— 内容里的 `rssh _names <kind>` 由
//! `commands::ls::cmd_names` 实现。

pub fn print_completions(shell: &str) {
    match shell {
        "zsh" => print!("{}", ZSH_COMPLETIONS),
        "bash" => print!("{}", BASH_COMPLETIONS),
        "powershell" | "pwsh" => print!("{}", POWERSHELL_COMPLETIONS),
        "fish" => print!("{}", FISH_COMPLETIONS),
        _ => eprintln!("Supported shells: zsh, bash, powershell, fish"),
    }
}

const ZSH_COMPLETIONS: &str = r#"#compdef rssh

_rssh() {
    local -a commands
    commands=(
        'ls:List profiles, credentials, or forwards'
        'open:Connect via SSH or start port forward'
        'add:Add profile, credential, or forward'
        'edit:Edit profile, credential, or forward'
        'rm:Delete profile, credential, or forward'
        'config:Configuration management'
        'completions:Generate shell completions'
    )

    _arguments -C \
        '1:command:->command' \
        '*::arg:->args'

    case $state in
        command)
            _describe 'command' commands
            ;;
        args)
            case $words[1] in
                ls)
                    local -a ls_opts=('cred:List credentials' 'fwd:List forwards')
                    _describe 'type' ls_opts
                    ;;
                open)
                    # ${(f)...} 按行切，保留含空格的 name（validate_name 允许空格）；
                    # 普通 $(...) 会按 IFS 字 break。
                    if [[ $CURRENT -eq 2 ]]; then
                        local -a _profs=("${(@f)$(rssh _names profiles 2>/dev/null)}")
                        compadd fwd -- "${_profs[@]}"
                    elif [[ $words[2] == "fwd" && $CURRENT -eq 3 ]]; then
                        local -a _fwds=("${(@f)$(rssh _names fwd 2>/dev/null)}")
                        compadd -- "${_fwds[@]}"
                    fi
                    ;;
                add)
                    compadd profile cred fwd
                    ;;
                edit|rm)
                    if [[ $CURRENT -eq 2 ]]; then
                        compadd profile cred fwd
                    elif [[ $CURRENT -eq 3 ]]; then
                        local -a _names
                        case $words[2] in
                            profile) _names=("${(@f)$(rssh _names profiles 2>/dev/null)}") ;;
                            cred)    _names=("${(@f)$(rssh _names creds 2>/dev/null)}") ;;
                            fwd)     _names=("${(@f)$(rssh _names fwd 2>/dev/null)}") ;;
                        esac
                        compadd -- "${_names[@]}"
                    fi
                    ;;
                config)
                    if [[ $CURRENT -eq 2 ]]; then
                        local -a cfg_cmds=('export:Export encrypted backup' 'import:Import backup' 'set:Set GitHub settings' 'push:Push to GitHub' 'pull:Pull from GitHub')
                        _describe 'action' cfg_cmds
                    elif [[ $CURRENT -eq 3 && ($words[2] == "export" || $words[2] == "import") ]]; then
                        _files
                    fi
                    ;;
                completions)
                    compadd zsh bash powershell fish
                    ;;
            esac
            ;;
    esac
}

_rssh "$@"
"#;

const BASH_COMPLETIONS: &str = r#"_rssh() {
    local cur prev words cword
    _init_completion || return

    if [[ $cword -eq 1 ]]; then
        COMPREPLY=($(compgen -W "ls open add edit rm config completions" -- "$cur"))
        return
    fi

    case ${words[1]} in
        ls)
            COMPREPLY=($(compgen -W "cred fwd" -- "$cur"))
            ;;
        open)
            # mapfile -t 按行读，保留含空格的 name；compgen -W 仍会按 IFS 切，
            # 所以手动 push 进 COMPREPLY 而不是用 compgen。
            if [[ $cword -eq 2 ]]; then
                local -a _profs
                mapfile -t _profs < <(rssh _names profiles 2>/dev/null)
                COMPREPLY=()
                [[ "fwd" == "$cur"* ]] && COMPREPLY+=("fwd")
                for _n in "${_profs[@]}"; do
                    [[ "$_n" == "$cur"* ]] && COMPREPLY+=("$_n")
                done
            elif [[ ${words[2]} == "fwd" && $cword -eq 3 ]]; then
                local -a _fwds
                mapfile -t _fwds < <(rssh _names fwd 2>/dev/null)
                COMPREPLY=()
                for _n in "${_fwds[@]}"; do
                    [[ "$_n" == "$cur"* ]] && COMPREPLY+=("$_n")
                done
            fi
            ;;
        add)
            COMPREPLY=($(compgen -W "profile cred fwd" -- "$cur"))
            ;;
        edit|rm)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=($(compgen -W "profile cred fwd" -- "$cur"))
            elif [[ $cword -eq 3 ]]; then
                local -a _names
                case ${words[2]} in
                    profile) mapfile -t _names < <(rssh _names profiles 2>/dev/null) ;;
                    cred)    mapfile -t _names < <(rssh _names creds 2>/dev/null) ;;
                    fwd)     mapfile -t _names < <(rssh _names fwd 2>/dev/null) ;;
                esac
                COMPREPLY=()
                for _n in "${_names[@]}"; do
                    [[ "$_n" == "$cur"* ]] && COMPREPLY+=("$_n")
                done
            fi
            ;;
        config)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=($(compgen -W "export import set push pull" -- "$cur"))
            elif [[ $cword -eq 3 && (${words[2]} == "export" || ${words[2]} == "import") ]]; then
                _filedir
            fi
            ;;
        completions)
            COMPREPLY=($(compgen -W "zsh bash powershell fish" -- "$cur"))
            ;;
    esac
}

complete -F _rssh rssh
"#;

const POWERSHELL_COMPLETIONS: &str = r#"Register-ArgumentCompleter -Native -CommandName rssh -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    # 用 AST 的 CommandElements 而非朴素 Split(' ')，否则 `rssh open "my prod"` 这类
    # 含空格 / 引号的 name 会被切碎，导致补全跳错分支。
    # Extent.Text 保留原始引号，这里手工剥一层让后面 -eq / -in 比对正常。
    $words = @($commandAst.CommandElements | ForEach-Object {
        $t = $_.Extent.Text
        if ($t.Length -ge 2 -and (($t.StartsWith('"') -and $t.EndsWith('"')) -or ($t.StartsWith("'") -and $t.EndsWith("'")))) {
            $t.Substring(1, $t.Length - 2)
        } else { $t }
    })
    $cmd = if ($words.Length -gt 1) { $words[1] } else { '' }
    $pos = $words.Length

    if ($pos -le 1 -or ($pos -eq 2 -and $wordToComplete)) {
        @('ls','open','add','edit','rm','config','completions') | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
            [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
        }
        return
    }

    if ($cmd -eq 'config' -and ($words[2] -eq 'export' -or $words[2] -eq 'import') -and ($words.Length -ge 4 -or -not $wordToComplete)) {
        [System.Management.Automation.CompletionCompleters]::CompleteFilename($wordToComplete)
        return
    }

    $(switch ($cmd) {
        'ls' { @('cred','fwd') | Where-Object { $_ -like "$wordToComplete*" } }
        'open' {
            if ($pos -eq 2 -or ($pos -eq 3 -and $wordToComplete -and $words[2] -ne 'fwd')) {
                $names = @('fwd') + @(rssh _names profiles 2>$null)
                $names | Where-Object { $_ -like "$wordToComplete*" }
            } elseif ($words[2] -eq 'fwd') {
                rssh _names fwd 2>$null | Where-Object { $_ -like "$wordToComplete*" }
            }
        }
        'add' { @('profile','cred','fwd') | Where-Object { $_ -like "$wordToComplete*" } }
        { $_ -in 'edit','rm' } {
            if ($pos -eq 2 -or ($pos -eq 3 -and $wordToComplete -and $words[2] -notin @('profile','cred','fwd'))) {
                @('profile','cred','fwd') | Where-Object { $_ -like "$wordToComplete*" }
            } elseif ($pos -ge 3) {
                $kind = $words[2]
                $n = switch ($kind) { 'profile' { 'profiles' } 'cred' { 'creds' } default { $kind } }
                rssh _names $n 2>$null | Where-Object { $_ -like "$wordToComplete*" }
            }
        }
        'config' { @('export','import','set','push','pull') | Where-Object { $_ -like "$wordToComplete*" } }
        'completions' { @('zsh','bash','powershell','fish') | Where-Object { $_ -like "$wordToComplete*" } }
    }) | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
"#;

// 注：Copilot 评论建议 fish 用 `string escape` 防空格，但 fish 的命令替换
// `(...)` **默认按换行符切分**（不像 bash 按 IFS 切空格），所以
// `(rssh _names profiles)` 输出 "my profile\nother" 已经正确拆成两条候选。
// 现状无需改动。
const FISH_COMPLETIONS: &str = r#"# rssh fish completions

function __rssh_arg
    # __rssh_arg N <value>...: true if the Nth word after `rssh` matches any value
    set -l tokens (commandline -opc)
    set -l idx (math 1 + $argv[1])
    test (count $tokens) -ge $idx; or return 1
    contains -- $tokens[$idx] $argv[2..-1]
end

function __rssh_pos
    # __rssh_pos N: true if currently completing the Nth word after `rssh`
    test (count (commandline -opc)) -eq $argv[1]
end

# Disable default file completion; the `config export/import <path>` rule below re-enables it via -F.
complete -c rssh -f

# Subcommands
complete -c rssh -n '__fish_use_subcommand' -a 'ls' -d 'List profiles/credentials/forwards'
complete -c rssh -n '__fish_use_subcommand' -a 'open' -d 'Connect via SSH'
complete -c rssh -n '__fish_use_subcommand' -a 'add' -d 'Add profile/credential/forward'
complete -c rssh -n '__fish_use_subcommand' -a 'edit' -d 'Edit profile/credential/forward'
complete -c rssh -n '__fish_use_subcommand' -a 'rm' -d 'Delete profile/credential/forward'
complete -c rssh -n '__fish_use_subcommand' -a 'config' -d 'Configuration management'
complete -c rssh -n '__fish_use_subcommand' -a 'completions' -d 'Generate shell completions'

# Second-level args
complete -c rssh -n '__rssh_arg 1 ls; and __rssh_pos 2' -a 'cred fwd'
complete -c rssh -n '__rssh_arg 1 open; and __rssh_pos 2' -a '(rssh _names profiles 2>/dev/null) fwd'
complete -c rssh -n '__rssh_arg 1 add; and __rssh_pos 2' -a 'profile cred fwd'
complete -c rssh -n '__rssh_arg 1 edit rm; and __rssh_pos 2' -a 'profile cred fwd'
complete -c rssh -n '__rssh_arg 1 config; and __rssh_pos 2' -a 'export import set push pull'
complete -c rssh -n '__rssh_arg 1 completions; and __rssh_pos 2' -a 'zsh bash powershell fish'

# Third-level args
complete -c rssh -n '__rssh_arg 1 open; and __rssh_arg 2 fwd; and __rssh_pos 3' -a '(rssh _names fwd 2>/dev/null)'
complete -c rssh -n '__rssh_arg 1 edit rm; and __rssh_arg 2 profile; and __rssh_pos 3' -a '(rssh _names profiles 2>/dev/null)'
complete -c rssh -n '__rssh_arg 1 edit rm; and __rssh_arg 2 cred; and __rssh_pos 3' -a '(rssh _names creds 2>/dev/null)'
complete -c rssh -n '__rssh_arg 1 edit rm; and __rssh_arg 2 fwd; and __rssh_pos 3' -a '(rssh _names fwd 2>/dev/null)'
complete -c rssh -n '__rssh_arg 1 config; and __rssh_arg 2 export import; and __rssh_pos 3' -F
"#;
