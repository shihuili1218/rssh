Register-ArgumentCompleter -Native -CommandName rssh -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $words = $commandAst.ToString().Split(' ')
    $cmd = if ($words.Length -gt 1) { $words[1] } else { '' }
    $pos = $words.Length

    if ($pos -le 1 -or ($pos -eq 2 -and $wordToComplete)) {
        @('ls','open','add','edit','rm','config','completions') | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
            [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
        }
        return
    }

    switch ($cmd) {
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
    } | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
