// What Windows opens for this app: a web address in the person's browser, a file in the app for its kind.
using System;
using System.Diagnostics;
using System.Threading.Tasks;
using Windows.System;

namespace One;

public static class Shell
{
    public static void Open(string target)
    {
        // A web address in the browser, never back in this app, which opens links to your One App network's address.
        if (Uri.TryCreate(target, UriKind.Absolute, out var url) && url.Scheme is "https" or "http")
        {
            _ = Launcher.LaunchUriAsync(url, new LauncherOptions { IgnoreAppUriHandlers = true });
            return;
        }
        try { Process.Start(new ProcessStartInfo(target) { UseShellExecute = true }); } catch { }
    }

    /// Links to your One App network's address open this app, from other apps (Edge and Chrome keep their own): Windows asks
    /// the address for its `/.well-known/windows-app-web-link`, which names this package (the gateway serves it).
    public static async Task Claim(string? address)
    {
        try
        {
            if (AppUriHandlerRegistrationManager.GetDefault().TryGetRegistration("links") is not { } links) return;
            var hosts = Uri.TryCreate(address, UriKind.Absolute, out var url) && url.Scheme == "https" ? new[] { new AppUriHandlerHost(url.Host) } : [];
            await links.SetAppAddedHostsAsync(hosts);
        }
        catch (Exception e)
        {
            App.Problem(e);
        }
    }
}
