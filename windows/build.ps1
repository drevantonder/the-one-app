# In the Windows VM, from build.sh: builds the app from the sources beside this file as a package, signed with this
# machine's certificate (setup.ps1), installs it for this user, and starts it on the signed-in desktop.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$app = Get-Content "$here\build.json" -Raw | ConvertFrom-Json
$signing = & "$here\setup.ps1" -Publisher $app.publisher | Select-Object -Last 1
$dotnet = "$env:LOCALAPPDATA\Microsoft\dotnet\dotnet.exe"
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'

# The icon is drawn on Apple's Mac grid, an 824-pixel tile in a clear 100-pixel margin in 1024: Windows gets that tile,
# filling its frame, its corners clear.
Add-Type -AssemblyName System.Drawing
$source = [System.Drawing.Image]::FromFile("$here\Assets\Icon.png")
$margin = [int](100 * $source.Width / 1024)
$tile = New-Object System.Drawing.Rectangle $margin, $margin, ($source.Width - 2 * $margin), ($source.Height - 2 * $margin)
$edges = New-Object System.Drawing.Imaging.ImageAttributes
$edges.SetWrapMode('TileFlipXY')
function Draw($size, $inset = 0, $width = $size) {
  $bitmap = New-Object System.Drawing.Bitmap $width, $size
  $g = [System.Drawing.Graphics]::FromImage($bitmap)
  $g.InterpolationMode = 'HighQualityBicubic'
  $g.PixelOffsetMode = 'HighQuality'
  $s = $size - 2 * $inset
  $g.DrawImage($source, (New-Object System.Drawing.Rectangle ([int](($width - $s) / 2)), $inset, $s, $s), $tile.X, $tile.Y, $tile.Width, $tile.Height, [System.Drawing.GraphicsUnit]::Pixel, $edges)
  $g.Dispose()
  $bitmap
}
function Png($file, $size, $inset = 0, $width = $size) {
  $bitmap = Draw $size $inset $width
  $bitmap.Save("$here\Assets\$file", [System.Drawing.Imaging.ImageFormat]::Png)
  $bitmap.Dispose()
}
# The window's and the taskbar's: each size a PNG inside one .ico.
$sizes = 16, 24, 32, 48, 64, 128, 256
$pngs = foreach ($s in $sizes) {
  $bitmap = Draw $s
  $stream = New-Object System.IO.MemoryStream
  $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
  $bitmap.Dispose()
  , $stream.ToArray()
}
# The package's own: its tiles, its logo, and the picture the Widgets board's picker shows. Small.png is the taskbar's
# and Start's, at each scale and target size Windows asks for, and unplated, so Windows draws no accent-colour square
# behind it.
Png Tile.png 150 24
Png Store.png 50
Png Widget.png 300 90 300
Png Small.png 44
foreach ($scale in 125, 150, 200, 400) { Png "Small.scale-$scale.png" ([int](44 * $scale / 100)) }
foreach ($s in 16, 24, 32, 48, 256) {
  foreach ($form in '', '_altform-unplated', '_altform-lightunplated') { Png "Small.targetsize-$s$form.png" $s }
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

$out = "$here\package"
$log = & $dotnet publish "$here\One.csproj" -c Release -r win-x64 --self-contained -p:Platform=x64 -p:AppFile=$($app.file) "-p:AppName=$($app.name)" -p:SigningCertificate=$signing 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
  # Only the compiler's error lines, each once and without where the build was, on stderr where whoever ran this reads it.
  $errors = $log -split "`r?`n" | Where-Object { $_ -match ': error [A-Z]+\d+:' } | ForEach-Object { ($_.Replace("$here\", '') -replace '\s*\[[^\]]*\.csproj\]\s*$', '').Trim() } | Select-Object -Unique -First 10
  # An error that isn't the compiler's (a missing SDK, say) shows the log's last lines instead.
  if (-not $errors) { $errors = $log.Trim() -split "`r?`n" | Select-Object -Last 5 }
  [Console]::Error.WriteLine("the Windows build failed:`n" + ($errors -join "`n"))
  exit 1
}
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
try {
  Start-ScheduledTask -TaskName $task
  for ($i = 0; -not (Test-Path $log) -and $i -lt 240; $i++) { Start-Sleep -Milliseconds 500 }
} finally { Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue }
$said = Get-Content $log -ErrorAction SilentlyContinue
if (-not $said) { throw "the install didn't finish" }
if ($said -like "Couldn't*") { throw $said }
$said
