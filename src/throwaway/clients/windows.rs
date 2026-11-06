//! The Windows client, on a throwaway copy of a borrowed device.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use one::config::{CLI, NAME};

use super::super::devices::{approve_code, build, didnt_sign_in, on_device};

/// The Windows app (client/native): cross-built here and packaged with its helper in the device
/// (client/native/windows/build.sh), installed there, then started by its program's path in the package (so it runs as
/// the package, as its badge and widgets need), at the throwaway network's address, printing its sign-in rather than
/// opening a browser, and with a control port, as linux() starts it: its Sign In pressed, the sign-in approved as the
/// throwaway network's first person or `person`, coming back to it on the device, and its code approved by the
/// throwaway network's machine. Once signed in on a device as them (a file in the throwaway network's folder says so), a
/// new build keeps that sign-in.
pub(crate) fn windows(dir: &Path, checkout: &Path, device: &str, settings: &one::cloudflare::Settings, person: Option<&str>) -> Result<()> {
    let address = settings.address.as_str();
    build(checkout, "client/native/windows/build.sh", &[("DEVICE", device), ("PROFILE", "dev"), ("NO_START", "1")])?;
    let file = NAME.chars().filter(char::is_ascii_alphanumeric).collect::<String>();
    let signed_in = dir.join(format!("signed-in-{device}"));
    let first = settings.throwaway_person.as_ref().map(|(_, name)| name.as_str()).unwrap_or_default();
    let who = person.unwrap_or(first);
    let here = format!("{address}\n{who}");
    let kept = std::fs::read_to_string(&signed_in).is_ok_and(|was| was == here);
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{device}", dir.display()).as_bytes()).into();
    let control = 30_000 + u16::from_le_bytes([hash[2], hash[3]]) % 20_000;
    let env = |key: &str| format!("{}_{key}", CLI.to_uppercase());
    // A copy of the person's computer may have the app signed in elsewhere, or as someone else: what it kept goes (its
    // folder, the package's, and its sign-in in the Credential Manager), and it signs in again. Started on the device's
    // desktop, as a task signed in there (a window started from SSH never draws), as build.ps1 does; what it says, in a
    // log there.
    let script = format!(
        r#"$p = Get-AppxPackage -Name '{file}'
Get-Process -Name '{file}' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500
$data = "$env:LOCALAPPDATA\Packages\$($p.PackageFamilyName)\LocalCache\Local\app.{CLI}.windows"
if (${fresh}) {{
  Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
  cmdkey /delete:'refresh.{CLI} sign-in' | Out-Null
}}
# The address, kept where the one a person types is, so what runs beside it (the widgets' helper) finds it too.
New-Item -ItemType Directory -Force $data | Out-Null
Set-Content "$data\address" '{address}' -NoNewline
$env:{print_var} = '1'
$env:{control_var} = '{control}'
Start-Process -FilePath "$($p.InstallLocation)\{file}.exe" -RedirectStandardOutput "$env:TEMP\one-throwaway-app.log" -RedirectStandardError "$env:TEMP\one-throwaway-app.err""#,
        fresh = if kept { "false" } else { "true" },
        print_var = env("SIGN_IN_PRINT"),
        control_var = env("CONTROL"),
    );
    let encoded = data_encoding::BASE64.encode(&script.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    on_device(
        device,
        &format!(
            r#"Remove-Item "$env:TEMP\one-throwaway-app.log" -ErrorAction SilentlyContinue
$a = New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -WindowStyle Hidden -EncodedCommand {encoded}'
$p = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
Register-ScheduledTask -TaskName one-throwaway-app -Action $a -Principal $p -Force | Out-Null
Start-ScheduledTask -TaskName one-throwaway-app"#
        ),
    )?;
    let eval = |js: &str| on_device(device, &format!("try {{ Invoke-WebRequest -UseBasicParsing -Method Post -Body '{}' http://127.0.0.1:{control}/eval | Out-Null }} catch {{ }}", js.replace('\'', "''")));
    let start = Instant::now();
    let (mut signed, mut approved) = (false, false);
    loop {
        if start.elapsed() > Duration::from_secs(180) {
            let tail = |file: &str| on_device(device, &format!(r#"$f = "$env:TEMP\{file}"; if (Test-Path $f) {{ Get-Content $f -Tail 20 }}"#)).unwrap_or_default();
            return Err(didnt_sign_in("Windows app", &[("the app", tail("one-throwaway-app.log")), ("its errors", tail("one-throwaway-app.err"))], &format!("look at it with {CLI} computer screen --device {device}")));
        }
        std::thread::sleep(Duration::from_secs(2));
        let said = on_device(device, r#"$log = "$env:TEMP\one-throwaway-app.log"; if (Test-Path $log) { Get-Content $log }"#)?;
        if said.lines().any(|l| l.trim() == "Let in.") {
            break;
        }
        match said.lines().rev().find_map(|l| l.trim().strip_prefix("Sign in: ")) {
            // Approved as the person, then back to the app at its loopback port, which only the device reaches.
            Some(url) if !signed => {
                let back = crate::sign_in::approved_on_throwaway(address, url, person)?;
                on_device(device, &format!("Invoke-WebRequest -UseBasicParsing '{}' | Out-Null", back.replace('\'', "''")))?;
                signed = true;
            }
            Some(_) => {}
            // Signed out: its Sign In, pressed for the person.
            None => drop(eval(super::PRESS_SIGN_IN)),
        }
        if let Some(code) = said.lines().rev().find_map(|l| l.trim().strip_prefix("Code: "))
            && !approved
        {
            approve_code(dir, code)?;
            approved = true;
        }
    }
    std::fs::write(&signed_in, &here)?;
    println!("  In its page, /eval runs JavaScript and /value gives an expression's value, as JSON: {CLI} agent-devices run --device {device} \"Invoke-WebRequest -UseBasicParsing -Method Post -Body '<js>' http://127.0.0.1:{control}/eval\"; what it said is in %TEMP%\\one-throwaway-app.log there");
    Ok(())
}
