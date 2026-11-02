# In the Windows VM, from build.sh: builds the app from the sources beside this file, installs it for this user (its
# folder under Programs, and a Start menu shortcut), and starts it on the signed-in desktop.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$app = Get-Content "$here\build.json" -Raw | ConvertFrom-Json
& "$here\setup.ps1" | Out-Null
$dotnet = "$env:LOCALAPPDATA\Microsoft\dotnet\dotnet.exe"
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'

# The icon: the config's artwork at Windows' sizes, each a PNG inside one .ico.
Add-Type -AssemblyName System.Drawing
$source = [System.Drawing.Image]::FromFile("$here\Assets\Icon.png")
$sizes = 16, 24, 32, 48, 64, 128, 256
$pngs = foreach ($s in $sizes) {
  $bitmap = New-Object System.Drawing.Bitmap $s, $s
  $g = [System.Drawing.Graphics]::FromImage($bitmap)
  $g.InterpolationMode = 'HighQualityBicubic'
  $g.DrawImage($source, 0, 0, $s, $s)
  $g.Dispose()
  $stream = New-Object System.IO.MemoryStream
  $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
  , $stream.ToArray()
}
$source.Dispose()
$ico = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ico
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
  $s = $sizes[$i] % 256
  $w.Write([byte]$s); $w.Write([byte]$s); $w.Write([byte]0); $w.Write([byte]0)
  $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$pngs[$i].Length); $w.Write([uint32]$offset)
  $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $w.Write($p) }
[System.IO.File]::WriteAllBytes("$here\Assets\App.ico", $ico.ToArray())

$out = "$here\out"
& $dotnet publish "$here\One.csproj" -c Release -r win-x64 --self-contained -p:Platform=x64 -p:AppFile=$($app.file) "-p:AppName=$($app.name)" -o $out
if ($LASTEXITCODE -ne 0) { throw "the build failed" }

# Installed for this user: a running copy is closed first.
Get-Process -Name $app.file -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500
$dest = "$env:LOCALAPPDATA\Programs\$($app.name)"
if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
Copy-Item -Recurse $out $dest
$exe = "$dest\$($app.file).exe"
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut("$env:APPDATA\Microsoft\Windows\Start Menu\Programs\$($app.name).lnk")
$link.TargetPath = $exe
$link.WorkingDirectory = $dest
$link.Save()

# Started on the desktop of whoever is signed in (an SSH session has none), by Explorer, as a click on it would: a task
# runs Explorer there, which opens the shortcut. (Started by the task itself, the window never draws.)
$task = "Start $($app.file)"
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
$open = New-ScheduledTaskAction -Execute explorer.exe -Argument "`"$($link.FullName)`""
Register-ScheduledTask -TaskName $task -Action $open -Principal $principal -Force | Out-Null
Start-ScheduledTask -TaskName $task
"Installed $($app.name) at $dest, and started it."
