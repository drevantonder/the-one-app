# Installs the latest release for this Windows computer, if its signature is the release key's: the program, with
# <cli>d, the device, run as a task of this user's, with no administrator. It runs while they're signed in, like a Mac's
# launch agent. Run again to update.
# `install.ps1 <binary>` installs a binary you built instead.
param([string]$Binary)
$ErrorActionPreference = 'Stop'
# Windows PowerShell's progress bar makes a large download many times slower.
$ProgressPreference = 'SilentlyContinue'
# config.toml's repo, cli and release_key; a test keeps them the same.
$repo = 'https://github.com/drevantonder/the-one-app'
$cli = 'one'
$key = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJxXC8FYsJQXptCREC6fvoYzO1sNLdOHtbsAnmUTljCM'
$url = "$repo/releases/latest/download"
$bin = Join-Path $env:LOCALAPPDATA "$cli\bin"
$oned = "${cli}d"
$tmp = Join-Path $env:TEMP "$cli-install-$PID"
New-Item -ItemType Directory -Force $bin, $tmp | Out-Null

try {
    if ($Binary) {
        $program = $Binary
    } else {
        # Windows' OpenSSH client checks the signature, as macOS's and Linux's do for install.sh.
        if (-not (Get-Command ssh-keygen -ErrorAction SilentlyContinue)) {
            throw "checking the download needs ssh-keygen: add Windows' OpenSSH Client, in Settings > Optional Features"
        }
        $asset = "$cli-x86_64-pc-windows-gnu.exe"
        $program = Join-Path $tmp $asset
        Write-Host "Downloading $asset"
        Invoke-WebRequest -UseBasicParsing "$url/$asset" -OutFile $program
        Invoke-WebRequest -UseBasicParsing "$url/$asset.sig" -OutFile "$program.sig"
        Set-Content -NoNewline -Encoding ascii (Join-Path $tmp 'signers') "release namespaces=`"release`" $key"
        cmd /c "ssh-keygen -Y verify -f `"$tmp\signers`" -I release -n release -s `"$program.sig`" < `"$program`"" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "$asset isn't signed with the release key, so it isn't installed" }
    }

    # A running program can't be overwritten, only renamed: what's renamed goes once nothing runs it, the next time
    # <cli>d starts.
    Stop-ScheduledTask -TaskName $oned -ErrorAction SilentlyContinue
    Get-Process -Name $oned -ErrorAction SilentlyContinue | Stop-Process -Force
    $stamp = [DateTime]::UtcNow.Ticks
    $names = $cli, $oned, "git-remote-$cli"
    foreach ($name in $names) {
        $file = Join-Path $bin "$name.exe"
        if (Test-Path $file) { Move-Item -Force $file "$file.$stamp.old" }
    }
    Copy-Item $program (Join-Path $bin "$cli.exe")
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
