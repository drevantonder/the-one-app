// What Windows opens for this app: a web address in the person's browser, a file in the app for its kind.
using System.Diagnostics;

namespace One;

public static class Shell
{
    public static void Open(string target)
    {
        try { Process.Start(new ProcessStartInfo(target) { UseShellExecute = true }); } catch { }
    }
}
