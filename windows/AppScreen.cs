// An app's screen: the page its node serves, in a WebView2 of its own that answers its bridge as the Apple apps' web
// view does (apple/Shared/Views/AppScreen.swift). Screens are untrusted: each is served at an origin of its own app's,
// which reaches nothing (this app makes every request itself), and talks only through the bridge's messages, acting
// only as that app. Anything that would take the screen away goes to the browser instead.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Web.WebView2.Core;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage.Pickers;

namespace One;

public sealed class AppScreen : Grid
{
    readonly Route route;
    readonly Action<Route> push;
    readonly Action<string> retitle;
    readonly WebView2 web = new() { DefaultBackgroundColor = Colors.Transparent, Opacity = 0 };
    readonly string origin;
    string? html;

    /// `push` opens another screen over this one; `retitle` sets the bar's title (empty while the screen shows its own).
    public AppScreen(Route route, Action<Route> push, Action<string> retitle)
    {
        this.route = route;
        this.push = push;
        this.retitle = retitle;
        // A host name of its app's own: letters, digits and dashes.
        var name = new string(route.App!.ToLowerInvariant().Select(c => char.IsAsciiLetterOrDigit(c) || c == '-' ? c : '-').ToArray()).Trim('-');
        origin = $"https://{(name.Length == 0 ? "app" : name)}.screen.localhost";
        Children.Add(web);
        Children.Add(new ProgressRing { IsActive = true, Width = 28, Height = 28, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center });
        // Node events reach the screen as the bridge's `one.__event`, while it's in the window.
        Loaded += async (_, _) =>
        {
            Store.Shared.Event -= Forward;
            Store.Shared.Event += Forward;
            await Start();
        };
        Unloaded += (_, _) => Store.Shared.Event -= Forward;
    }

    void Forward(JsonNode e) => web.CoreWebView2?.PostWebMessageAsJson(new JsonObject { ["oneEvent"] = e.DeepClone() }.ToJsonString());

    async Task Start()
    {
        if (web.CoreWebView2 != null) return;
        try
        {
            html = await Node.Screen(route.App!, route.Screen, route.Params, route.Scope);
        }
        catch (Exception error)
        {
            Fail(error.Message);
            return;
        }
        await web.EnsureCoreWebView2Async();
        var core = web.CoreWebView2!;
        var settings = core.Settings;
        settings.AreHostObjectsAllowed = false;
        settings.IsStatusBarEnabled = false;
        settings.IsZoomControlEnabled = false;
        settings.AreDevToolsEnabled = false;
        settings.IsGeneralAutofillEnabled = false;
        settings.IsPasswordAutosaveEnabled = false;
        settings.AreBrowserAcceleratorKeysEnabled = false;
        core.Profile.PreferredColorScheme = Appearance.Chosen switch { "light" => CoreWebView2PreferredColorScheme.Light, "dark" => CoreWebView2PreferredColorScheme.Dark, _ => CoreWebView2PreferredColorScheme.Auto };
        await core.AddScriptToExecuteOnDocumentCreatedAsync(Bridge);
        await core.AddScriptToExecuteOnDocumentCreatedAsync(Fitted());
        core.AddWebResourceRequestedFilter($"{origin}/*", CoreWebView2WebResourceContext.All);
        core.WebResourceRequested += Serve;
        core.NavigationStarting += (_, e) =>
        {
            // Once the screen is in, anything that would take it away leaves for the browser, and the screen stays.
            if (e.Uri == $"{origin}/" && !shown) return;
            e.Cancel = true;
            Outside(e.Uri);
        };
        core.NewWindowRequested += (_, e) =>
        {
            e.Handled = true;
            Outside(e.Uri);
        };
        core.NavigationCompleted += (_, e) =>
        {
            if (shown) return;
            shown = true;
            web.Opacity = 1;
            Children.OfType<ProgressRing>().ToList().ForEach(r => Children.Remove(r));
        };
        core.ProcessFailed += (_, _) => { shown = false; core.Navigate($"{origin}/"); };
        core.WebMessageReceived += async (_, e) =>
        {
            // Only the screen's own page: not a frame inside it, nor a page it went to.
            if (!e.Source.StartsWith(origin + "/")) return;
            await Answer(e.WebMessageAsJson);
        };
        core.Navigate($"{origin}/");
    }

    bool shown;

    /// The screen's own address answers with its page; nothing else is there.
    void Serve(CoreWebView2 sender, CoreWebView2WebResourceRequestedEventArgs e)
    {
        var path = new Uri(e.Request.Uri).AbsolutePath;
        var env = web.CoreWebView2.Environment;
        if (path == "/" && html != null)
        {
            var bytes = new MemoryStream(Encoding.UTF8.GetBytes(html));
            e.Response = env.CreateWebResourceResponse(bytes.AsRandomAccessStream(), 200, "OK", "Content-Type: text/html; charset=utf-8");
        }
        else e.Response = env.CreateWebResourceResponse(null, 404, "Not Found", "");
    }

    void Fail(string message)
    {
        Children.Clear();
        Children.Add(Look.Unavailable("Can’t Open This", message));
    }

    /// The bridge's way to this app (`__oneShell`, as Android's web view puts one): each message posted as `{ one,
    /// id }`, answered with `{ oneReply: id, ok | error }`; node events come as `{ oneEvent }`.
    const string Bridge = """
        (() => {
          const view = window.chrome?.webview;
          if (!view) return;
          let asked = 0;
          const waiting = {};
          view.addEventListener("message", (e) => {
            const m = e.data || {};
            if (m.oneEvent) window.one?.__event(m.oneEvent);
            const w = waiting[m.oneReply];
            if (!w) return;
            delete waiting[m.oneReply];
            m.error === undefined ? w.ok(m.ok) : w.fail(new Error(m.error));
          });
          window.__oneShell = { postMessage: (message) => new Promise((ok, fail) => { const id = ++asked; waiting[id] = { ok, fail }; view.postMessage({ one: message, id }); }) };
        })();
        """;

    /// What makes a screen sit in the window as its own: the window's backdrop shows through, the install's accent in
    /// light and dark, and on a computer, Windows' size of text.
    static string Fitted()
    {
        var accent = Store.Shared.Config?.Accent ?? Brand.Accent;
        string Css(bool dark)
        {
            var c = Look.Hex(accent, dark);
            return $"rgb({c.R}, {c.G}, {c.B})";
        }
        return $$"""
            document.addEventListener("DOMContentLoaded", () => {
              const style = document.createElement("style");
              style.textContent = `
                html, body { background: transparent !important; }
                html { zoom: calc(14 / 17); }
                :root:root { --accent: {{Css(false)}}; --bg: transparent; --card: rgba(0, 0, 0, 0.04); }
                @media (prefers-color-scheme: dark) { :root:root { --accent: {{Css(true)}}; --card: rgba(255, 255, 255, 0.05); } }`;
              document.head.appendChild(style);
            });
            """;
    }

    static void Outside(string uri)
    {
        if (Uri.TryCreate(uri, UriKind.Absolute, out var url) && url.Scheme is "https" or "http" or "mailto" or "tel") Shell.Open(url.AbsoluteUri);
    }

    void Reply(JsonNode? id, JsonNode? ok) => web.CoreWebView2?.PostWebMessageAsJson(new JsonObject { ["oneReply"] = id?.DeepClone(), ["ok"] = ok }.ToJsonString());
    void Fail(JsonNode? id, string error) => web.CoreWebView2?.PostWebMessageAsJson(new JsonObject { ["oneReply"] = id?.DeepClone(), ["error"] = error }.ToJsonString());

    /// The bridge: `{ one: { type: "call" | "open" | "title" | "save" | "preview" | "copy" | "vault" | "icons" | …, … }, id }`.
    /// A screen only ever calls its own app, but may open another's screens.
    async Task Answer(string json)
    {
        JsonNode? message;
        try { message = JsonNode.Parse(json); } catch { return; }
        var id = message?["id"];
        if (message?["one"] is not JsonObject m) return;
        try
        {
            switch (m["type"]?.GetValue<string>())
            {
                case "call":
                    Reply(id, await Node.Call(route.App!, m["op"]?.GetValue<string>() ?? "", m["input"]?.DeepClone() ?? new JsonObject()));
                    return;
                case "open":
                    {
                        var app = m["app"]?.GetValue<string>() is { Length: > 0 } other ? other : route.App!;
                        Reply(id, null);
                        push(Route.ScreenOf(app, route.Scope, m["screen"]?.GetValue<string>() ?? "main", (m["params"] ?? new JsonObject()).ToJsonString()));
                        return;
                    }
                case "title":
                    retitle(m["text"]?.GetValue<string>() ?? "");
                    Reply(id, null);
                    return;
                case "save":
                    await Save(m["file"] as JsonObject ?? []);
                    Reply(id, null);
                    return;
                case "preview":
                    await Preview(m["file"] as JsonObject ?? []);
                    Reply(id, null);
                    return;
                case "copy":
                    {
                        var data = new DataPackage();
                        data.SetText(m["text"]?.ToString() ?? "");
                        Clipboard.SetContent(data);
                        Reply(id, null);
                        return;
                    }
                case "link":
                    Outside(m["url"]?.GetValue<string>() ?? "");
                    Reply(id, null);
                    return;
                case "vault":
                    Reply(id, await AskVault(m["need"]?.GetValue<string>() ?? ""));
                    return;
                case "scan":
                    Fail(id, "Scanning documents needs the iPhone or iPad app. Choose the files instead.");
                    return;
                case "icons":
                    {
                        var paths = new JsonObject();
                        foreach (var name in (m["names"] as JsonArray ?? []).Select(n => n?.ToString() ?? ""))
                            if (Look.Icons.TryGetValue(name, out var d)) paths[name] = d;
                        Reply(id, paths);
                        return;
                    }
                case "icon":
                    Reply(id, Tile(m));
                    return;
                // A computer has no haptics.
                case "haptic":
                    Reply(id, null);
                    return;
                default:
                    Fail(id, "Unknown message");
                    return;
            }
        }
        catch (Exception error)
        {
            Fail(id, error.Message);
        }
    }

    /// An app's tile, as the window draws them (Look.Tile), as an image a page shows: a finished app icon as it is, a
    /// logo inset on white, or its icon filled (else its first letter) in white on its colour.
    static string Tile(JsonObject m)
    {
        string? Text(string key) => m[key]?.ToString() is { Length: > 0 } v ? v : null;
        var (icon, seed, image) = (Text("icon"), Text("seed") ?? "", Text("image"));
        if (image != null && Text("kind") == "icon") return image;
        static string esc(string text) => System.Net.WebUtility.HtmlEncode(text);
        string svg;
        if (image != null)
            svg = $"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" rx="58" fill="#fff"/><image href="{esc(image)}" x="36" y="36" width="184" height="184"/></svg>""";
        else
        {
            var c = Look.TileColor(Text("color"), seed);
            var face = icon != null && Look.Icons.TryGetValue($"{icon}-fill", out var d)
                ? $"""<path transform="translate(64 64) scale(0.5)" fill="#fff" d="{esc(d)}"/>"""
                : $"""<text x="128" y="128" dy="0.35em" text-anchor="middle" font-family="Segoe UI" font-weight="600" font-size="118" fill="#fff">{esc(seed.Length > 0 ? seed[..1].ToUpperInvariant() : "?")}</text>""";
            svg = $"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="rgb({c.R + (255 - c.R) * 12 / 100},{c.G + (255 - c.G) * 12 / 100},{c.B + (255 - c.B) * 12 / 100})"/><stop offset="1" stop-color="rgb({c.R},{c.G},{c.B})"/></linearGradient></defs><rect width="256" height="256" rx="58" fill="url(#g)"/>{face}</svg>""";
        }
        return "data:image/svg+xml;base64," + Convert.ToBase64String(Encoding.UTF8.GetBytes(svg));
    }

    /// A file a screen gives, as `{ name, type, base64 }` or `{ name, type, blob }`.
    static async Task<(string name, byte[] bytes)> File(JsonObject f)
    {
        var name = f["name"]?.GetValue<string>() is { Length: > 0 } n ? n : "file";
        if (f["base64"]?.GetValue<string>() is { } b64) return (name, Convert.FromBase64String(b64));
        if ((f["blob"] ?? f["file"])?.GetValue<string>() is { } hash) return (name, await Node.Blob(hash));
        throw new Exception("No file to show.");
    }

    /// `one.save`: where the person chooses.
    async Task Save(JsonObject f)
    {
        var (name, bytes) = await File(f);
        var picker = new FileSavePicker { SuggestedFileName = Path.GetFileNameWithoutExtension(name), SuggestedStartLocation = PickerLocationId.Downloads };
        var ext = Path.GetExtension(name);
        picker.FileTypeChoices.Add(ext.Length > 0 ? ext.TrimStart('.').ToUpperInvariant() : "File", [ext.Length > 0 ? ext : "."]);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(App.Window));
        var file = await picker.PickSaveFileAsync();
        if (file != null) await Windows.Storage.FileIO.WriteBytesAsync(file, bytes);
    }

    /// `one.preview`: in the app Windows opens its kind of file with.
    static async Task Preview(JsonObject f)
    {
        var (name, bytes) = await File(f);
        var dir = Path.Combine(Path.GetTempPath(), Brand.Cli, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        var path = Path.Combine(dir, Path.GetFileName(name));
        await System.IO.File.WriteAllBytesAsync(path, bytes);
        Shell.Open(path);
    }

    /// `one.vault(need)`: asks for what the app declares, in the Vault's sheet → whether it's allowed once it's gone.
    async Task<bool> AskVault(string need)
    {
        var scope = route.Scope ?? "user";
        var id = await Node.RequestItem(scope, route.App!, need) ?? throw new Exception("The Vault didn’t take the request");
        await VaultSheet.Show(XamlRoot, scope, id);
        return (await Node.VaultRequests(scope, id)).FirstOrDefault()?.State == "set";
    }
}
