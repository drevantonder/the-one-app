# Installs the newest build for this Windows computer, if the release key signed it: the program, with <cli>d, the
# device, run as a task of this user's, with no administrator. It runs while they're signed in, like a Mac's launch
# agent. Run again to update.
# `install.ps1 <binary>` installs a binary you built instead.
param([string]$Binary)
$ErrorActionPreference = 'Stop'
# Windows PowerShell's progress bar makes a large download many times slower.
$ProgressPreference = 'SilentlyContinue'
# Where its builds are, a folder laid out as a network's /updates/ is (oned/update/src/latest.rs), and config.toml's cli
# and release_key: whoever serves it fills them in (oned_update::install).
$from = ''
$cli = ''
$key = ''
if ($Binary) { $cli = [IO.Path]::GetFileNameWithoutExtension($Binary) }
$bin = Join-Path $env:LOCALAPPDATA "$cli\bin"
$oned = "${cli}d"
$tmp = Join-Path $env:TEMP "$cli-install-$PID"
New-Item -ItemType Directory -Force $bin, $tmp | Out-Null

try {
    # The program, and its app runtime (<cli>-apps), which <cli>d starts beside it.
    if ($Binary) {
        $program = $Binary
        $runtime = Join-Path (Split-Path $Binary) "$cli-apps.exe"
    } else {
        if (-not $from) { throw "this installer doesn't say where builds are: run the one your network's address serves (irm <address>/install.ps1 | iex)" }
        # What's newest at $from, if the release key signed it: the record `<cli> update` reads, checked as it checks it,
        # with Windows' OpenSSH client, as macOS's and Linux's check it for install.sh.
        if (-not (Get-Command ssh-keygen -ErrorAction SilentlyContinue)) {
            throw "checking the download needs ssh-keygen: add Windows' OpenSSH Client, in Settings > Optional Features"
        }
        Invoke-WebRequest -UseBasicParsing "$from/newest" -OutFile (Join-Path $tmp 'newest')
        $newest = [IO.File]::ReadAllText((Join-Path $tmp 'newest'))
        $at = $newest.IndexOf("-----BEGIN SSH SIGNATURE-----")
        if ($at -lt 0) { throw "what's newest at $from isn't signed with the release key, so nothing is installed" }
        $record = $newest.Substring(0, $at)
        [IO.File]::WriteAllText((Join-Path $tmp 'record'), $record)
        [IO.File]::WriteAllText((Join-Path $tmp 'record.sig'), $newest.Substring($at))
        [IO.File]::WriteAllText((Join-Path $tmp 'signers'), "release namespaces=`"release-latest`" $key")
        cmd /c "ssh-keygen -Y verify -f `"$tmp\signers`" -I release -n release-latest -s `"$tmp\record.sig`" < `"$tmp\record`" 2>nul" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "what's newest at $from isn't signed with the release key, so nothing is installed" }
        # Each file the record names (node/<target>, …), if it's what the record names: its hash.
        $program, $runtime = foreach ($name in 'node/x86_64-pc-windows-gnu', 'service/apps/x86_64-pc-windows-gnu') {
            $line = $record -split "`n" | ForEach-Object { , ($_ -split ' ') } | Where-Object { $_.Count -eq 6 -and $_[0] -eq $name } | Select-Object -First 1
            if (-not $line) { throw "there's no build for this machine at $from ($name)" }
            $file = Join-Path $tmp ($name -replace '/', '-')
            Write-Host "Downloading $name"
            Invoke-WebRequest -UseBasicParsing "$from/$($line[3])" -OutFile $file
            if ((Get-FileHash -Algorithm SHA256 $file).Hash -ne $line[3]) { throw "$name isn't what the signed record names, so it isn't installed" }
            $file
        }
    }

    # A running program can't be overwritten, only renamed: what's renamed goes once nothing runs it, the next time
    # <cli>d starts.
    Stop-ScheduledTask -TaskName $oned -ErrorAction SilentlyContinue
    Get-Process -Name $oned -ErrorAction SilentlyContinue | Stop-Process -Force
    $stamp = [DateTime]::UtcNow.Ticks
    $names = $cli, $oned, "git-remote-$cli", "$cli-apps"
    foreach ($name in $names) {
        $file = Join-Path $bin "$name.exe"
        if (Test-Path $file) { Move-Item -Force $file "$file.$stamp.old" }
    }
    Copy-Item $program (Join-Path $bin "$cli.exe")
    if (Test-Path $runtime) { Copy-Item $runtime (Join-Path $bin "$cli-apps.exe") }
    foreach ($name in $oned, "git-remote-$cli") {
        New-Item -ItemType HardLink -Path (Join-Path $bin "$name.exe") -Target (Join-Path $bin "$cli.exe") | Out-Null
    }

    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($path -split ';') -notcontains $bin) {
        [Environment]::SetEnvironmentVariable('Path', "$bin;$path", 'User')
        $env:Path = "$bin;$env:Path"
    }

    # Every minute as well as at sign-in: Windows starts it again only if it's told to look, and IgnoreNew leaves one
    # that's running alone. conhost, because a program that uses a console gets a window of its own otherwise.
    $action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument "--headless `"$(Join-Path $bin "$oned.exe")`""
    $triggers = @(
        (New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME),
        (New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval (New-TimeSpan -Minutes 1))
    )
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero)
    # Only an administrator can have one that runs signed out: a standard user is refused.
    $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName $oned -Action $action -Trigger $triggers -Settings $settings -Principal $principal -Force | Out-Null
    Start-ScheduledTask -TaskName $oned
    Write-Host "Installed $bin\$cli.exe, with $oned running while you're signed in."
    Write-Host ""
    Write-Host "Next, on your first device:  $cli create <name> --as <your name>"
    Write-Host "Or, to join a network:       $cli join <pairing code>   ($cli pair on a device in it gives one)"
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
