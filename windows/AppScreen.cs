// A screen: an app's, or one of this app's own (a page, Services, a Vault request, Settings'), the same on every client.
// The page its node serves, in a WebView2 of its own that answers its bridge (src/screen/bridge.js) as the web client's
// page answers its frames (kernel/web/web.js), natively for whatever a person touches: menus, alerts, sheets, files and
// links. Screens are untrusted: each is served at an origin of its own app's (the own screens' their own), which reaches
// nothing (this app makes every request itself), and talks only through the bridge's messages, an app's acting only as
// that app, and only the own screens' as the person. Anything that would take the screen away goes to the browser.
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
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.Web.WebView2.Core;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.Storage.Pickers;

namespace One;

public sealed class AppScreen : Grid
{
    readonly Route route;
    readonly Action<string> retitle;
    /// The sheet it's in, if it's in one.
    readonly Sheet? sheet;
    readonly WebView2 web = new() { DefaultBackgroundColor = Colors.Transparent, Opacity = 0 };
    readonly string origin;
    string? html;

    /// When this computer's appearance changes: every screen's web view follows.
    public static event Action? Restyled;
    public static void Restyle() => Restyled?.Invoke();

    /// `retitle` sets its bar's title (empty while the screen shows its own); in a sheet, the sheet's.
    public AppScreen(Route route, Action<string> retitle, Sheet? sheet = null)
    {
        this.route = route;
        this.retitle = retitle;
        this.sheet = sheet;
        // A host name of its app's own: letters, digits and dashes.
        var name = new string((route.App ?? "").ToLowerInvariant().Select(c => char.IsAsciiLetterOrDigit(c) || c == '-' ? c : '-').ToArray()).Trim('-');
        origin = Own ? "https://own.localhost" : $"https://{(name.Length == 0 ? "app" : name)}.screen.localhost";
        Children.Add(web);
        Children.Add(new ProgressRing { IsActive = true, Width = 28, Height = 28, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center });
        // Node events reach the screen as the bridge's `one.__event`, while it's in the window.
        Loaded += async (_, _) =>
        {
            Store.Shared.Event -= Forward;
            Store.Shared.Event += Forward;
            Restyled -= Rescheme;
            Restyled += Rescheme;
            await Start();
        };
        Unloaded += (_, _) =>
        {
            Store.Shared.Event -= Forward;
            Restyled -= Rescheme;
            ticking?.Stop();
            waiting?.Cancel();
        };
    }

    /// Whether it's one of this app's own screens: the route this app chose, never anything the page says.
    bool Own => route.Own;

    static MainWindow Window => App.Window!;

    void Forward(JsonNode e) => web.CoreWebView2?.PostWebMessageAsJson(new JsonObject { ["oneEvent"] = e.DeepClone() }.ToJsonString());

    void Rescheme()
    {
        if (web.CoreWebView2 == null) return;
        web.CoreWebView2.Profile.PreferredColorScheme = Scheme;
        _ = web.CoreWebView2.ExecuteScriptAsync($"document.documentElement.style.zoom = '{Zoom}'");
    }

    async Task Start()
    {
        if (web.CoreWebView2 != null || !await Fetch()) return;
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
        core.Profile.PreferredColorScheme = Scheme;
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

    // MARK: Offline

    /// Since when the screen shows its last copy, the node not answering; null while it answers.
    DateTime? kept;
    /// The node's tries since it last answered, for the wait before the next: 2, 4, 8, 16, then every 30 seconds.
    int misses;
    InfoBar? offline;
    DispatcherTimer? ticking;
    System.Threading.CancellationTokenSource? waiting;

    /// Reads the screen's page. Offline, an app's screen shows its last copy, saying so, and asks again and again,
    /// waiting longer each time, as the iPhone's does. → whether there's a page to show.
    async Task<bool> Fetch()
    {
        try
        {
            html = Own ? await Node.ClientScreen(route.Screen, route.Params) : await Node.Screen(route.App!, route.Screen, route.Params, route.Scope);
            if (!Own) ScreenCopies.Keep(route, html);
            misses = 0;
            kept = null;
            Offline(null);
            return true;
        }
        catch (Exception error)
        {
            if (kept == null)
            {
                if (error is SignedOutException || Own || ScreenCopies.Last(route) is not { } copy)
                {
                    Fail(error.Message);
                    return false;
                }
                (html, kept) = copy;
            }
            misses++;
            _ = Again(TimeSpan.FromSeconds(Math.Min(30, 1 << Math.Min(misses, 5))));
            return true;
        }
    }

    /// Asks the node again after `wait`, or at Try Now; once it answers, the screen as it is now.
    async Task Again(TimeSpan wait)
    {
        waiting = new System.Threading.CancellationTokenSource();
        Offline(DateTime.Now + wait);
        try { await Task.Delay(wait, waiting.Token); } catch (TaskCanceledException) { }
        if (!IsLoaded) return;
        Offline(null);
        if (await Fetch() && kept == null && web.CoreWebView2 is { } core)
        {
            shown = false;
            core.Navigate($"{origin}/");
        }
    }

    /// The bar over a screen shown as it last looked: since when, with Try Now while it waits to ask again (`next`);
    /// asking now, if `next` is null; gone once the node answers.
    void Offline(DateTime? next)
    {
        if (kept is not { } since)
        {
            if (offline != null) Children.Remove(offline);
            offline = null;
            ticking?.Stop();
            web.Margin = new Thickness(0);
            return;
        }
        // Below the bar, while it's there.
        web.Margin = new Thickness(0, 64, 0, 0);
        if (offline == null)
        {
            offline = new InfoBar { IsOpen = true, IsClosable = false, Severity = InfoBarSeverity.Informational, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(12, 8, 12, 0) };
            var now = new Button { Content = "Try Now" };
            now.Click += (_, _) => waiting?.Cancel();
            offline.ActionButton = now;
            Children.Add(offline);
            ticking = new DispatcherTimer { Interval = TimeSpan.FromSeconds(30) };
            ticking.Tick += (_, _) => Say();
            ticking.Start();
        }
        offline.ActionButton.Visibility = next == null ? Visibility.Collapsed : Visibility.Visible;
        void Say()
        {
            if (offline == null) return;
            var ago = DateTime.Now - since;
            var when = ago.TotalMinutes < 1 ? null : ago.TotalHours < 1 ? $"{(int)ago.TotalMinutes} min ago" : ago.TotalDays < 1 ? $"{(int)ago.TotalHours} hr ago" : since.ToString("d MMM");
            offline.Message = (when == null ? "Offline" : $"Offline · from {when}") + (next == null ? " · Trying again…" : "");
        }
        Say();
    }

    static CoreWebView2PreferredColorScheme Scheme => Appearance.Chosen switch { "light" => CoreWebView2PreferredColorScheme.Light, "dark" => CoreWebView2PreferredColorScheme.Dark, _ => CoreWebView2PreferredColorScheme.Auto };

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

    /// The kit is sized for a phone's 17-point text: a computer's is 14, larger as Windows' text size is (Settings >
    /// Accessibility > Text size).
    static string Zoom => (14.0 / 17 * new Windows.UI.ViewManagement.UISettings().TextScaleFactor).ToString(System.Globalization.CultureInfo.InvariantCulture);

    /// What makes a screen sit in the window as its own: the window's backdrop shows through, the install's accent in
    /// light and dark, and on a computer, Windows' size of text (a page's widgets keep their own size).
    static string Fitted()
    {
        var accent = Store.Shared.Config?.Accent ?? WhiteLabel.Accent;
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
                :root:root { --accent: {{Css(false)}}; --bg: transparent; --card: rgba(0, 0, 0, 0.04); }
                @media (prefers-color-scheme: dark) { :root:root { --accent: {{Css(true)}}; --card: rgba(255, 255, 255, 0.05); } }`;
              document.head.appendChild(style);
              document.documentElement.style.zoom = "{{Zoom}}";
            });
            """;
    }

    static void Outside(string uri)
    {
        if (Uri.TryCreate(uri, UriKind.Absolute, out var url) && url.Scheme is "https" or "http" or "mailto" or "tel") Shell.Open(url.AbsoluteUri);
    }

    void Reply(JsonNode? id, JsonNode? ok) => web.CoreWebView2?.PostWebMessageAsJson(new JsonObject { ["oneReply"] = id?.DeepClone(), ["ok"] = ok?.DeepClone() }.ToJsonString());
    void Fail(JsonNode? id, string error) => web.CoreWebView2?.PostWebMessageAsJson(new JsonObject { ["oneReply"] = id?.DeepClone(), ["error"] = error }.ToJsonString());

    static string? Text(JsonObject m, string key) => m[key] is JsonValue v && v.TryGetValue<string>(out var s) ? s : null;

    /// The bridge: `{ one: { type, … }, id }`, each of its `messages`. `api`, `signOut`, `appearance`, `device` and
    /// `request` are this app's own screens' alone. A screen only ever calls its own app, but may open another's screens.
    async Task Answer(string json)
    {
        JsonNode? message;
        try { message = JsonNode.Parse(json); } catch { return; }
        var id = message?["id"];
        if (message?["one"] is not JsonObject m) return;
        try
        {
            var type = Text(m, "type");
            if (type is "api" or "signOut" or "appearance" or "device" or "request" or "cache" && !Own)
            {
                Fail(id, "Only Settings can do that.");
                return;
            }
            switch (type)
            {
                case "api":
                    Reply(id, await Node.Api(Text(m, "method") ?? "GET", Text(m, "path") ?? "", m["body"]?.DeepClone()));
                    return;
                case "signOut":
                    Reply(id, null);
                    await Window.SignOut();
                    return;
                case "cache":
                    {
                        var file = KeptFile(Text(m, "key") ?? "");
                        if (!m.ContainsKey("value"))
                        {
                            JsonNode? kept = null;
                            try { if (file != null && System.IO.File.Exists(file)) kept = JsonNode.Parse(await System.IO.File.ReadAllTextAsync(file)); } catch { }
                            Reply(id, kept);
                            return;
                        }
                        // A screen still open as the person signs out keeps nothing after.
                        if (file != null && Auth.Shared.SignedIn)
                        {
                            Directory.CreateDirectory(Path.GetDirectoryName(file)!);
                            await System.IO.File.WriteAllTextAsync(file, m["value"]?.ToJsonString() ?? "null");
                        }
                        Reply(id, null);
                        return;
                    }
                case "appearance":
                    if (Text(m, "value") is "automatic" or "light" or "dark") Appearance.Choose(Text(m, "value")!, Window);
                    Reply(id, Appearance.Chosen);
                    return;
                case "device":
                    Reply(id, new JsonObject { ["name"] = Environment.MachineName, ["signOut"] = true });
                    return;
                case "request":
                    Reply(id, await Asked(Route.Request(Text(m, "scope") ?? "", Text(m, "id") ?? "")));
                    return;
                case "call":
                    if (Own) Fail(id, "Settings has no app to call.");
                    else Reply(id, await Node.Call(route.App!, Text(m, "op") ?? "", m["input"]?.DeepClone() ?? new JsonObject()));
                    return;
                case "open":
                    {
                        // Another app's screen; else, from an own screen, another own one; else one of this app's.
                        var parameters = (m["params"] ?? new JsonObject()).ToJsonString();
                        var screen = Text(m, "screen");
                        var to = Text(m, "app") is { Length: > 0 } app ? Route.ScreenOf(app, Text(m, "scope") ?? route.Scope, screen ?? "main", parameters)
                            : Own ? Route.OwnOf(screen ?? "", parameters)
                            : Route.ScreenOf(route.App!, Text(m, "scope") ?? route.Scope, screen ?? "main", parameters);
                        if (m["sheet"]?.GetValueKind() == JsonValueKind.True)
                        {
                            Reply(id, await Window.Present(to));
                            return;
                        }
                        Reply(id, null);
                        // From a sheet, the sheet goes and it opens where the window is.
                        sheet?.Close(null);
                        Window.Push(to);
                        return;
                    }
                case "close":
                    Reply(id, null);
                    if (sheet != null) sheet.Close(m["value"]);
                    else Window.Back(this);
                    return;
                case "title":
                    if (sheet != null) sheet.Retitle(Text(m, "text") ?? "");
                    else retitle(Text(m, "text") ?? "");
                    Reply(id, null);
                    return;
                case "menu":
                    Reply(id, await Menu(m));
                    return;
                case "alert":
                    Reply(id, await Alert(m));
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
                    Outside(Text(m, "url") ?? "");
                    Reply(id, null);
                    return;
                case "vault":
                    {
                        if (Own) throw new Exception("Settings has no app to ask for.");
                        var scope = route.Scope ?? "user";
                        var asked = await Node.RequestItem(scope, route.App!, Text(m, "need") ?? "") ?? throw new Exception("The Vault didn’t take the request");
                        Reply(id, await Asked(Route.Request(scope, asked)));
                        return;
                    }
                case "scan":
                    Fail(id, "Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.");
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

    /// A request for an item, in its sheet → whether it was allowed, once the sheet's gone.
    static async Task<bool> Asked(Route request) => (await Window.Present(request))?.GetValueKind() == JsonValueKind.True;

    /// Where the client's own screens keep what they ask to (`one.cache`), by key; forgotten when it signs out. (Not
    /// "Screens", the web views' own folder, which is the same name on Windows.)
    public static string Kept => Path.Combine(WhiteLabel.Folder, "Kept for screens");

    static string? KeptFile(string key) => key.Length is > 0 and <= 40 && key.All(c => char.IsAsciiLetterOrDigit(c) || c == '-') ? Path.Combine(Kept, key + ".json") : null;

    /// `one.menu`: Windows' own menu at its point in the screen → the index of the one chosen, or null once it's closed.
    /// A menu with a checked item shows each with its place for a check; one that can't be undone, in red; each with
    /// its icon; a section after a line. Choices in a row (a widget's sizes) are the first items, then a line.
    Task<JsonNode?> Menu(JsonObject m)
    {
        var done = new TaskCompletionSource<JsonNode?>();
        var entries = (m["items"] as JsonArray ?? []).OfType<JsonObject>().ToList();
        if (entries.Count == 0) return Task.FromResult<JsonNode?>(null);
        static bool Flag(JsonObject e, string key) => e[key]?.GetValueKind() == JsonValueKind.True;
        var checks = entries.Any(e => Flag(e, "checked"));
        var flyout = new MenuFlyout();
        for (var i = 0; i < entries.Count; i++)
        {
            var e = entries[i];
            if (i > 0 && (Flag(e, "section") || (Flag(entries[i - 1], "row") && !Flag(e, "row")))) flyout.Items.Add(new MenuFlyoutSeparator());
            MenuFlyoutItem item = checks ? new ToggleMenuFlyoutItem { IsChecked = Flag(e, "checked") } : new MenuFlyoutItem();
            item.Text = Text(e, "title") ?? "";
            if (Text(e, "icon") is { } icon && Look.Icons.TryGetValue(icon, out var d) && Look.PathData(d) is { } geometry)
            {
                geometry.Transform = new Microsoft.UI.Xaml.Media.ScaleTransform { ScaleX = 16 / 256.0, ScaleY = 16 / 256.0 };
                item.Icon = new PathIcon { Data = geometry };
            }
            if (e["danger"]?.GetValueKind() == JsonValueKind.True)
            {
                var red = Look.Critical;
                item.Foreground = red;
                foreach (var key in new[] { "MenuFlyoutItemForeground", "MenuFlyoutItemForegroundPointerOver", "MenuFlyoutItemForegroundPressed", "ToggleMenuFlyoutItemForeground", "ToggleMenuFlyoutItemForegroundPointerOver", "ToggleMenuFlyoutItemForegroundPressed" })
                    item.Resources[key] = red;
            }
            var index = i;
            item.Click += (_, _) => done.TrySetResult(index);
            flyout.Items.Add(item);
        }
        // Windows says which was chosen before the menu goes: closed with none, it's null.
        flyout.Closed += (_, _) => done.TrySetResult(null);
        // Its point in the page, in CSS pixels, which are the window's (the web view isn't zoomed).
        double At(string key) => m[key] is JsonValue v && v.TryGetValue<double>(out var d) ? d : 0;
        // From an element in the page's lower half, it opens upward.
        var up = m["rect"] is JsonArray r && r.Count == 4 && r[1] is JsonValue top && top.TryGetValue<double>(out var t) && t > web.ActualHeight / 2;
        flyout.ShowAt(web, new FlyoutShowOptions
        {
            Position = new Point(Math.Clamp(At("x"), 0, web.ActualWidth), Math.Clamp(At("y"), 0, web.ActualHeight)),
            Placement = up ? FlyoutPlacementMode.Top : FlyoutPlacementMode.Auto,
        });
        return done.Task;
    }

    /// `one.alert`: Windows' own dialog, its title and text, then OK; or Cancel and its action (red if it can't be
    /// undone) → whether that was chosen.
    async Task<JsonNode?> Alert(JsonObject m)
    {
        var action = Text(m, "action") is { Length: > 0 } a ? a : null;
        var dialog = new ContentDialog
        {
            Title = Text(m, "title") ?? "",
            Content = new TextBlock { Text = Text(m, "text") ?? "", TextWrapping = TextWrapping.WrapWholeWords },
            CloseButtonText = action == null ? "OK" : "Cancel",
            XamlRoot = XamlRoot,
            RequestedTheme = Appearance.Theme,
        };
        if (action != null)
        {
            dialog.PrimaryButtonText = action;
            dialog.DefaultButton = ContentDialogButton.Primary;
            if (m["danger"]?.GetValueKind() == JsonValueKind.True)
            {
                var red = Look.Critical;
                foreach (var key in new[] { "AccentButtonBackground", "AccentButtonBackgroundPointerOver", "AccentButtonBackgroundPressed" })
                    dialog.Resources[key] = red;
            }
        }
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
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

    static string Previewed => Path.Combine(Path.GetTempPath(), WhiteLabel.Cli);

    /// What screens kept on this computer, gone: their copies, what they asked to keep and the files they showed.
    public static void Forget()
    {
        ScreenCopies.Forget();
        foreach (var dir in new[] { Kept, Previewed })
            try { if (Directory.Exists(dir)) Directory.Delete(dir, true); } catch { }
    }

    /// `one.preview`: in the app Windows opens its kind of file with.
    static async Task Preview(JsonObject f)
    {
        var (name, bytes) = await File(f);
        var dir = Path.Combine(Previewed, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        var path = Path.Combine(dir, Path.GetFileName(name));
        await System.IO.File.WriteAllBytesAsync(path, bytes);
        Shell.Open(path);
    }
}

/// The last copy of each app screen this computer showed, which stands in while the node can't be reached; gone once
/// the person signs out.
static class ScreenCopies
{
    static string Dir => Path.Combine(WhiteLabel.Folder, "Screen copies");

    static string FileOf(Route r)
    {
        var key = string.Join("\u0001", r.App, r.Screen, r.Params, r.Scope ?? "");
        return Path.Combine(Dir, Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(Encoding.UTF8.GetBytes(key)))[..24] + ".html");
    }

    public static void Keep(Route r, string html)
    {
        try
        {
            Directory.CreateDirectory(Dir);
            System.IO.File.WriteAllText(FileOf(r), html);
        }
        catch { }
    }

    /// Its page and when it was kept.
    public static (string html, DateTime? kept)? Last(Route r)
    {
        try
        {
            var file = FileOf(r);
            return System.IO.File.Exists(file) ? (System.IO.File.ReadAllText(file), System.IO.File.GetLastWriteTime(file)) : null;
        }
        catch { return null; }
    }

    public static void Forget()
    {
        try { if (Directory.Exists(Dir)) Directory.Delete(Dir, true); } catch { }
    }
}
