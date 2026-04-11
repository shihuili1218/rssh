_rssh() {
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
            if [[ $cword -eq 2 ]]; then
                local profiles=$(rssh _names profiles 2>/dev/null)
                COMPREPLY=($(compgen -W "fwd $profiles" -- "$cur"))
            elif [[ ${words[2]} == "fwd" && $cword -eq 3 ]]; then
                local fwds=$(rssh _names fwd 2>/dev/null)
                COMPREPLY=($(compgen -W "$fwds" -- "$cur"))
            fi
            ;;
        add)
            COMPREPLY=($(compgen -W "profile cred fwd" -- "$cur"))
            ;;
        edit|rm)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=($(compgen -W "profile cred fwd" -- "$cur"))
            elif [[ $cword -eq 3 ]]; then
                case ${words[2]} in
                    profile) COMPREPLY=($(compgen -W "$(rssh _names profiles 2>/dev/null)" -- "$cur")) ;;
                    cred)    COMPREPLY=($(compgen -W "$(rssh _names creds 2>/dev/null)" -- "$cur")) ;;
                    fwd)     COMPREPLY=($(compgen -W "$(rssh _names fwd 2>/dev/null)" -- "$cur")) ;;
                esac
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
