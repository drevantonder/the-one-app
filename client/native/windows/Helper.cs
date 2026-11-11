// The package's helper: what Windows starts beside the app for what only the Windows App SDK does, with no window and
// no client or key of its own. The Widgets board starts it as the package's COM server for its widgets (Widgets.cs); a
// toast's click or button, or a notification, starts it to answer them (Notes.cs); the app starts it to register its push
// channel (--register-notifications). Whatever it needs of the user's devices it asks the app's own program, which answers
// with no window (client/native/src/headless.rs), as JSON.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.Windows.AppLifecycle;

namespace One;

public static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        if (args.Contains(Widgets.ServerArgument))
        {
            Widgets.Serve();
            return 0;
        }
        Notes.Start();
        if (args.Contains("--register-notifications"))
        {
            Notes.Register().Wait();
            return 0;
        }
        Notes.Started(AppInstance.GetCurrent().GetActivatedEventArgs());
        return 0;
    }
}

/// What the app's program says went wrong, as a sentence for the user.
public sealed class Refused(string message) : Exception(message);

/// The app's own program, beside this one in the package.
public static class App
{
    static string Exe => Path.Combine(AppContext.BaseDirectory, CustomBuild.File + ".exe");

    /// Where the app keeps what it keeps (Tauri's app local data folder: the package's own, as both are in it).
    public static string Folder => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), $"app.{CustomBuild.Cli}.windows");

    /// One thing asked of the app with no window (`--headless <kind> <input>`) → what it answered, or `Refused` with
    /// why not.
    public static async Task<JsonNode?> Ask(string kind, JsonObject input)
    {
        var start = new ProcessStartInfo(Exe) { UseShellExecute = false, RedirectStandardOutput = true, CreateNoWindow = true, ArgumentList = { "--headless", kind, input.ToJsonString() } };
        using var p = Process.Start(start) ?? throw new Refused($"{CustomBuild.Name} didn’t start.");
        // One that can't reach the user's devices gives up, as a page's request would.
        using var limit = new System.Threading.CancellationTokenSource(TimeSpan.FromSeconds(90));
        string said;
        try
        {
            said = await p.StandardOutput.ReadToEndAsync(limit.Token);
            await p.WaitForExitAsync(limit.Token);
        }
        catch (OperationCanceledException)
        {
            try { p.Kill(); } catch { }
            throw new TimeoutException($"{CustomBuild.Name} didn’t reach your devices in time.");
        }
        var line = said.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).LastOrDefault();
        var answer = line == null ? null : JsonNode.Parse(line);
        if (answer?["error"]?.GetValue<string>() is { } error) throw new Refused(error);
        return answer;
    }

    /// A link of the app's (`<cli>://…`), opened in its window, which comes to the front: the user just clicked.
    public static void Open(string link)
    {
        AllowSetForegroundWindow(-1);
        Process.Start(new ProcessStartInfo(Exe) { UseShellExecute = false, ArgumentList = { link } })?.Dispose();
    }

    [System.Runtime.InteropServices.DllImport("user32.dll")] static extern bool AllowSetForegroundWindow(int process);

    /// What went wrong, kept where the user (or whoever helps them) can send it.
    public static void Problem(Exception e)
    {
        try
        {
            Directory.CreateDirectory(Folder);
            File.AppendAllText(Path.Combine(Folder, "What went wrong.txt"), $"{DateTime.UtcNow:u} {e}\n");
        }
        catch { }
    }
}
