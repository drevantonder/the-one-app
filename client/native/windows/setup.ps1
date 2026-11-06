# Once per Windows machine that builds the app: the .NET SDK, for this user (NuGet brings the Windows App SDK at
# build), and a certificate to sign the package with, which this machine trusts. It's this machine's own, for testing:
# a package it signs installs only where its certificate is trusted, so releasing to others needs a real one.
param([Parameter(Mandatory)] [string] $Publisher)
$ErrorActionPreference = 'Stop'
$dotnet = "$env:LOCALAPPDATA\Microsoft\dotnet"
if (-not (Test-Path "$dotnet\dotnet.exe") -or -not (& "$dotnet\dotnet.exe" --list-sdks | Select-String '^10\.')) {
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  Invoke-WebRequest https://dot.net/v1/dotnet-install.ps1 -OutFile "$env:TEMP\dotnet-install.ps1" -UseBasicParsing
  & "$env:TEMP\dotnet-install.ps1" -Channel 10.0 -InstallDir $dotnet -NoPath | Out-Null
}
[Environment]::SetEnvironmentVariable('DOTNET_CLI_TELEMETRY_OPTOUT', '1', 'User')

$cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $Publisher -and $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date).AddDays(30) } | Select-Object -First 1
if (-not $cert) {
  # Code signing (1.3.6.1.5.5.7.3.3), and not a certificate authority.
  $cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher -KeyUsage DigitalSignature -FriendlyName "$Publisher (this computer's, for testing)" `
    -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddYears(5) -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
}
if (-not (Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object Thumbprint -eq $cert.Thumbprint)) {
  $public = "$env:TEMP\signing.cer"
  Export-Certificate -Cert $cert -FilePath $public | Out-Null
  Import-Certificate -FilePath $public -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
  Remove-Item $public
}
$cert.Thumbprint
