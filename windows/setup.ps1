# Once per Windows machine that builds the app: the .NET SDK, for this user. NuGet brings the Windows App SDK at build.
$ErrorActionPreference = 'Stop'
$dotnet = "$env:LOCALAPPDATA\Microsoft\dotnet"
if (-not (Test-Path "$dotnet\dotnet.exe") -or -not (& "$dotnet\dotnet.exe" --list-sdks | Select-String '^10\.')) {
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  Invoke-WebRequest https://dot.net/v1/dotnet-install.ps1 -OutFile "$env:TEMP\dotnet-install.ps1" -UseBasicParsing
  & "$env:TEMP\dotnet-install.ps1" -Channel 10.0 -InstallDir $dotnet -NoPath
}
[Environment]::SetEnvironmentVariable('DOTNET_CLI_TELEMETRY_OPTOUT', '1', 'User')
& "$dotnet\dotnet.exe" --version
