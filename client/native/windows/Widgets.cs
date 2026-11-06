// The Widgets board's widgets (Win+W), as the iPhone's (apple/Widget): one kind, a widget from your pages, the one its
// person chooses (another than those already pinned, until they do), drawn at the size it's placed at as an Adaptive
// Card (WidgetCard.cs), its buttons and toggles running the widget's tools. Windows starts this helper as the
// package's COM server for the board (Helper.cs). It draws from what the app keeps of every page's widgets for the
// platform's own (client/native/src/widgets.rs), which the app writes whenever it sees them change, and asks the app,
// with no window, to read them again (--widgets) when the board shows them and every few minutes while it does, and to
// run a widget's tool (--call). What each pinned widget shows is kept with it, as its custom state.
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Windows.Widgets;
using Microsoft.Windows.Widgets.Providers;

namespace One;

/// Every page's widgets, as the app last read them: its `widgets.json`, in the iPhone widget's format.
public static class WidgetCache
{
    public sealed record Item(string App, string Widget, string Scope, string Page, string Title, string? Error, JsonElement Root, Dictionary<string, JsonElement>? Sizes)
    {
        /// `<scope>/<app>.<widget>`, as the iPhone's: what a pinned widget's choice is found again by.
        public string Id => $"{Scope}/{App}.{Widget}";

        public JsonElement Tree(string size) => Sizes?.TryGetValue(size, out var t) == true ? t : Root;
    }

    /// What the app keeps: the widgets and when they were read (UTC); none, signed out.
    public sealed record Kept(List<Item>? Items, DateTime At);

    static string Dir => Path.Combine(App.Folder, "widgets");
    public static string File => Path.Combine(Dir, "widgets.json");

    /// Seconds from 1970 to 2001, where the iPhone's `fetched` counts from.
    const double Swift = 978_307_200;

    public static Kept? Last()
    {
        try
        {
            using var doc = JsonDocument.Parse(System.IO.File.ReadAllBytes(File));
            var root = doc.RootElement;
            if (root.TryGetProperty("signedOut", out var o) && o.ValueKind == JsonValueKind.True) return new Kept(null, default);
            var at = root.TryGetProperty("fetched", out var f) && f.ValueKind == JsonValueKind.Number ? DateTime.UnixEpoch.AddSeconds(f.GetDouble() + Swift) : default;
            var items = new List<Item>();
            foreach (var w in root.GetProperty("widgets").EnumerateArray())
            {
                string S(string key) => w.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() ?? "" : "";
                var error = w.TryGetProperty("error", out var e) && e.ValueKind != JsonValueKind.Null ? e.ValueKind == JsonValueKind.String ? e.GetString() : e.GetRawText() : null;
                var sizes = w.TryGetProperty("sizes", out var s) && s.ValueKind == JsonValueKind.Object ? s.EnumerateObject().ToDictionary(p => p.Name, p => p.Value.Clone()) : null;
                items.Add(new Item(S("app"), S("widget"), S("scope"), S("page"), S("title"), error, w.TryGetProperty("root", out var r) ? r.Clone() : default, sizes));
            }
            // The icons they draw, kept beside them.
            try { WidgetArt.Icons = JsonSerializer.Deserialize<Dictionary<string, string>>(System.IO.File.ReadAllText(Path.Combine(Dir, "icons.json"))) ?? []; }
            catch { }
            return new Kept(items, at);
        }
        catch
        {
            return null;
        }
    }

    /// "Updated 2 hr ago", once what's shown was read over an hour ago (it's read every few minutes it can be).
    public static string? Updated(DateTime at)
    {
        var ago = DateTime.UtcNow - at;
        if (at == default || ago < TimeSpan.FromHours(1)) return null;
        return ago.TotalDays < 1 ? $"Updated {(int)ago.TotalHours} hr ago" : $"Updated {at.ToLocalTime():d MMM}";
    }

    /// The app wrote them again (it read them, or its person signed out there): `changed`, once it's done.
    public static FileSystemWatcher Watch(Action changed)
    {
        Directory.CreateDirectory(Dir);
        Timer? settling = null;
        var watcher = new FileSystemWatcher(Dir, "widgets.json") { NotifyFilter = NotifyFilters.LastWrite | NotifyFilters.FileName };
        void Seen(object _, FileSystemEventArgs e) => (settling ??= new Timer(_ => changed())).Change(300, Timeout.Infinite);
        watcher.Changed += Seen;
        watcher.Created += Seen;
        watcher.Renamed += (o, e) => Seen(o, e);
        watcher.EnableRaisingEvents = true;
        return watcher;
    }
}

[ComVisible(true)]
public sealed partial class Widgets : IWidgetProvider, IWidgetProvider2
{
    public const string ServerArgument = "-RegisterProcessAsComServer";

    /// A pinned widget as the board has it: its size, and what it shows (kept as its custom state).
    sealed class Pinned
    {
        public required string Size;
        public string? Chosen;
        /// The chosen widget's title, as it was chosen: what one that's gone is called.
        public string? Title;
        public bool Choosing;
        public bool Active;
        /// What went wrong the last time one of its buttons was used, until another is, or it's read again on its own.
        public string? Problem;

        public string State => new JsonObject { ["widget"] = Chosen, ["title"] = Title, ["choosing"] = Choosing }.ToJsonString();

        public static Pinned Of(WidgetContext context, string? state)
        {
            var pinned = new Pinned { Size = SizeOf(context.Size) };
            try
            {
                var kept = JsonNode.Parse(state ?? "");
                pinned.Chosen = kept?["widget"]?.GetValue<string>();
                pinned.Title = kept?["title"]?.GetValue<string>();
                pinned.Choosing = kept?["choosing"]?.GetValue<bool>() == true;
            }
            catch { }
            return pinned;
        }
    }

    static readonly ConcurrentDictionary<string, Pinned> pinned = new();
    static readonly ManualResetEventSlim done = new();
    static readonly SemaphoreSlim reading = new(1, 1);
    static List<WidgetCache.Item>? items;
    static DateTime readAt;
    /// Why there's nothing to show: signed out, or the devices aren't answering.
    static string? away;
    static Timer? timer;
    static FileSystemWatcher? watching;

    static string SizeOf(WidgetSize size) => size switch { WidgetSize.Small => "small", WidgetSize.Large => "large", _ => "medium" };

    // MARK: Serving

    [DllImport("ole32.dll")] static extern int CoRegisterClassObject([MarshalAs(UnmanagedType.LPStruct)] Guid clsid, [MarshalAs(UnmanagedType.IUnknown)] object unknown, uint context, uint flags, out uint cookie);
    [DllImport("ole32.dll")] static extern int CoRevokeClassObject(uint cookie);

    /// Serves the board until it has none of this app's widgets left.
    public static void Serve()
    {
        // Registered from a thread of the multithreaded apartment, where the board's calls come in on threads of their own.
        var thread = new Thread(() =>
        {
            try
            {
                // What they showed last, to draw at once.
                Load();
                // Drawn again whenever the app writes them.
                watching = WidgetCache.Watch(() =>
                {
                    Load();
                    DrawAll(p => !p.Choosing);
                });
                // Those pinned before it started, as they were left; the board's calls come only once it's registered.
                foreach (var info in WidgetManager.GetDefault().GetWidgetInfos())
                    pinned.TryAdd(info.WidgetContext.Id, Pinned.Of(info.WidgetContext, info.CustomState));
                Marshal.ThrowExceptionForHR(CoRegisterClassObject(Guid.Parse(CustomBuild.WidgetClass), new Factory(), 0x4 /* local server */, 0x1 /* multiple use */, out var cookie));
                done.Wait();
                CoRevokeClassObject(cookie);
            }
            catch (Exception e)
            {
                App.Problem(e);
            }
        });
        thread.SetApartmentState(ApartmentState.MTA);
        thread.Start();
        thread.Join();
    }

    [ComImport, Guid("00000001-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IClassFactory
    {
        [PreserveSig] int CreateInstance(IntPtr outer, ref Guid iid, out IntPtr made);
        [PreserveSig] int LockServer(bool locked);
    }

    [ComVisible(true)]
    sealed class Factory : IClassFactory
    {
        public int CreateInstance(IntPtr outer, ref Guid iid, out IntPtr made)
        {
            made = IntPtr.Zero;
            if (outer != IntPtr.Zero) return unchecked((int)0x80040110); // no aggregation
            // The provider, as whichever of its interfaces the board asks for.
            var unknown = WinRT.MarshalInspectable<IWidgetProvider>.FromManaged(new Widgets());
            try { return Marshal.QueryInterface(unknown, in iid, out made); }
            finally { Marshal.Release(unknown); }
        }

        public int LockServer(bool locked) => 0;
    }

    // MARK: The board's calls

    public void CreateWidget(WidgetContext context)
    {
        var shown = pinned.Values.Select(p => p.Chosen).ToHashSet();
        var p = new Pinned { Size = SizeOf(context.Size) };
        pinned[context.Id] = p;
        _ = Task.Run(async () =>
        {
            await Read(TimeSpan.FromSeconds(30));
            var first = items?.FirstOrDefault(i => !shown.Contains(i.Id)) ?? items?.FirstOrDefault();
            if (p.Chosen == null && first != null) (p.Chosen, p.Title) = (first.Id, first.Title);
            Draw(context.Id, p);
        });
    }

    public void DeleteWidget(string id, string state)
    {
        pinned.TryRemove(id, out _);
        if (pinned.IsEmpty) done.Set();
    }

    public void OnActionInvoked(WidgetActionInvokedArgs args)
    {
        var id = args.WidgetContext.Id;
        if (!pinned.TryGetValue(id, out var p)) return;
        JsonNode? data = null;
        try { data = JsonNode.Parse(string.IsNullOrEmpty(args.Data) ? "{}" : args.Data); } catch { }
        switch (args.Verb)
        {
            case "choose":
                p.Chosen = data?["widget"]?.GetValue<string>() ?? p.Chosen;
                p.Title = items?.FirstOrDefault(i => i.Id == p.Chosen)?.Title ?? p.Title;
                p.Choosing = false;
                Draw(id, p);
                break;
            case "cancel":
                p.Choosing = false;
                Draw(id, p);
                break;
            case "run":
                // Drawn as done at once (a toggle switched, a button greyed), then as its tool leaves it; back as it
                // was if it didn't work.
                var (app, op) = (data?["app"]?.GetValue<string>() ?? "", data?["op"]?.GetValue<string>() ?? "");
                var input = data?["input"] as JsonObject;
                var which = WidgetCard.Which(app, op, input);
                p.Problem = null;
                WidgetCard.Doing[which] = input?["on"] is JsonValue on && on.TryGetValue<bool>(out var to) ? to : true;
                DrawAll(_ => true);
                _ = Task.Run(async () =>
                {
                    try
                    {
                        await App.Ask("--call", app, op, input?.ToJsonString() ?? "{}");
                        p.Problem = null;
                    }
                    catch (Exception e)
                    {
                        p.Problem = $"Couldn’t do that: {e.Message}";
                    }
                    await Read(TimeSpan.Zero);
                    WidgetCard.Doing.TryRemove(which, out _);
                    DrawAll(_ => true);
                });
                break;
        }
    }

    public void OnWidgetContextChanged(WidgetContextChangedArgs args)
    {
        if (!pinned.TryGetValue(args.WidgetContext.Id, out var p)) return;
        p.Size = SizeOf(args.WidgetContext.Size);
        Draw(args.WidgetContext.Id, p);
    }

    /// The board shows it: drawn from what was read within the last half minute, else read again; and every few minutes
    /// while it's shown.
    public void Activate(WidgetContext context)
    {
        var p = pinned.GetOrAdd(context.Id, _ => new Pinned { Size = SizeOf(context.Size) });
        p.Active = true;
        if (items != null || away != null) Draw(context.Id, p);
        _ = Task.Run(async () =>
        {
            await Read(TimeSpan.FromSeconds(30));
            DrawAll(_ => true);
        });
        timer ??= new Timer(_ => _ = Task.Run(async () =>
        {
            if (!pinned.Values.Any(x => x.Active)) return;
            await Read(TimeSpan.Zero);
            foreach (var x in pinned.Values) x.Problem = null;
            DrawAll(_ => true);
        }), null, Every, Every);
    }

    static TimeSpan Every => TimeSpan.FromMinutes(5);

    public void Deactivate(string id)
    {
        if (pinned.TryGetValue(id, out var p)) p.Active = false;
    }

    public void OnCustomizationRequested(WidgetCustomizationRequestedArgs args)
    {
        var id = args.WidgetContext.Id;
        var p = pinned.GetOrAdd(id, _ => new Pinned { Size = SizeOf(args.WidgetContext.Size) });
        p.Choosing = true;
        Draw(id, p);
        _ = Task.Run(async () =>
        {
            await Read(TimeSpan.FromSeconds(30));
            Draw(id, p);
        });
    }

    // MARK: Reading and drawing

    /// What the app kept, as it last wrote it.
    static void Load()
    {
        switch (WidgetCache.Last())
        {
            case { Items: null }:
                (items, away) = (null, "signed out");
                break;
            case { } kept:
                (items, readAt, away) = (kept.Items, kept.At, null);
                break;
        }
    }

    /// Every page's widgets read again by the app, unless they were read within `fresh`.
    static async Task Read(TimeSpan fresh)
    {
        await reading.WaitAsync();
        try
        {
            if (items != null && away == null && DateTime.UtcNow - readAt < fresh) return;
            await App.Ask("--widgets");
            Load();
        }
        catch (Refused e) when (e.Message.StartsWith("Sign in"))
        {
            (items, away) = (null, "signed out");
        }
        catch (Exception)
        {
            // What it had, if anything, and that its devices didn't answer.
            Load();
            away = "unreachable";
        }
        finally
        {
            reading.Release();
        }
    }

    static void DrawAll(Func<Pinned, bool> which)
    {
        foreach (var (id, p) in pinned)
            if (which(p)) Draw(id, p);
    }

    /// What a pinned widget shows, sent to the board.
    static void Draw(string id, Pinned p)
    {
        (string template, string data) card;
        var list = away == "signed out" ? null : items;
        // One never chosen shows the first, as the iPhone's does.
        var item = p.Chosen == null ? list?.FirstOrDefault() : list?.FirstOrDefault(i => i.Id == p.Chosen);
        if (away == "signed out")
            card = WidgetCard.Saying($"Sign in to {CustomBuild.Name} to see your pages.");
        else if (list == null)
            card = WidgetCard.Saying(away == null ? "Getting your widgets…" : $"Can’t reach {CustomBuild.Name} right now. This tries again by itself.");
        else if (p.Choosing)
            card = WidgetCard.Chooser(list, p.Chosen);
        else if (item != null)
            card = Drawn(item, p.Size, p.Problem, WidgetCache.Updated(readAt));
        else if (p.Chosen != null && list.Count > 0)
            card = WidgetCard.Saying($"{p.Title ?? "This widget"} isn’t on your pages any more. Choose another with Customize, in this widget’s menu.");
        else
            card = WidgetCard.Saying($"Open {CustomBuild.Name} to see your pages.");
        try
        {
            WidgetManager.GetDefault().UpdateWidget(new WidgetUpdateRequestOptions(id) { Template = card.template, Data = card.data, CustomState = p.State });
        }
        catch (Exception e)
        {
            App.Problem(e);
        }
    }

    /// A widget's card; one this can't draw says so, as a widget that stopped working does.
    static (string template, string data) Drawn(WidgetCache.Item item, string size, string? problem, string? updated)
    {
        try { return WidgetCard.Of(item, size, problem, updated); }
        catch (Exception e)
        {
            App.Problem(e);
            return WidgetCard.Of(item with { Error = e.Message }, size, problem, null);
        }
    }
}
