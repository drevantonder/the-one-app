# In the Windows VM, from build.sh: builds the app from the sources beside this file as a package, signed with this
# machine's certificate (setup.ps1), installs it for this user, and starts it on the signed-in desktop.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$app = Get-Content "$here\build.json" -Raw | ConvertFrom-Json
$signing = & "$here\setup.ps1" -Publisher $app.publisher | Select-Object -Last 1
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
# The package's own: its tiles, its logo, and the picture the Widgets board's picker shows.
function Png($size, $file, $inset = 0, $width = $size) {
  $bitmap = New-Object System.Drawing.Bitmap $width, $size
  $g = [System.Drawing.Graphics]::FromImage($bitmap)
  $g.InterpolationMode = 'HighQualityBicubic'
  $s = $size - 2 * $inset
  $g.DrawImage($source, ($width - $s) / 2, $inset, $s, $s)
  $g.Dispose()
  $bitmap.Save("$here\Assets\$file", [System.Drawing.Imaging.ImageFormat]::Png)
  $bitmap.Dispose()
}
Png 150 Tile.png 24
Png 44 Small.png
Png 50 Store.png
Png 300 Widget.png 90 300
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

$out = "$here\package"
& $dotnet publish "$here\One.csproj" -c Release -r win-x64 --self-contained -p:Platform=x64 -p:AppFile=$($app.file) "-p:AppName=$($app.name)" -p:SigningCertificate=$signing
if ($LASTEXITCODE -ne 0) { throw "the build failed" }
$package = Get-ChildItem -Recurse "$out\*.msix" | Sort-Object LastWriteTime | Select-Object -Last 1
# The Windows App SDK's runtime, which it depends on: its own packages from NuGet, the framework first.
$runtime = (Get-ChildItem "$env:USERPROFILE\.nuget\packages\microsoft.windowsappsdk.runtime" | Sort-Object { [version]$_.Name } | Select-Object -Last 1).FullName
$runtimes = 'Microsoft.WindowsAppRuntime', 'Main', 'Singleton', 'DDLM' | ForEach-Object { Get-ChildItem "$runtime\tools\MSIX\win10-x64\*.msix" | Where-Object Name -like "*$_*" | Select-Object -First 1 -ExpandProperty FullName }

# Installed for this user, over the one there (closed first), keeping what it keeps, then started; the app as it was
# before it was a package goes. Both on the desktop of whoever is signed in, as a task: Windows installs packages only
# for a session there, not an SSH one, and a window started from SSH never draws.
Get-Process -Name $app.file -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\Programs\$($app.name)", "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\$($app.name).lnk" -ErrorAction SilentlyContinue
$log = "$here\install.log"
Remove-Item $log -ErrorAction SilentlyContinue
Set-Content "$here\install.ps1" @"
try {
  foreach (`$r in '$($runtimes -join "','")') { Add-AppxPackage -Path `$r -ErrorAction SilentlyContinue }
  Add-AppxPackage -Path '$($package.FullName)' -ForceApplicationShutdown -ForceUpdateFromAnyVersion -ErrorAction Stop
  `$p = Get-AppxPackage -Name '$($app.file)'
  Start-Process explorer.exe "shell:AppsFolder\`$(`$p.PackageFamilyName)!App"
  "Installed $($app.name) `$(`$p.Version), and started it." | Set-Content '$log'
} catch { "Couldn't install it: `$_" | Set-Content '$log' }
"@
$task = "Install $($app.file)"
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest
$run = New-ScheduledTaskAction -Execute powershell.exe -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$here\install.ps1`""
Register-ScheduledTask -TaskName $task -Action $run -Principal $principal -Force | Out-Null
Start-ScheduledTask -TaskName $task
for ($i = 0; -not (Test-Path $log) -and $i -lt 240; $i++) { Start-Sleep -Milliseconds 500 }
$said = Get-Content $log -ErrorAction SilentlyContinue
if (-not $said) { throw "the install didn't finish" }
if ($said -like "Couldn't*") { throw $said }
$said
