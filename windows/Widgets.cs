// The Widgets board's widgets (Win+W), as the iPhone's (apple/Widget): one kind, a widget from your pages, the one its
// person chooses (another than those already pinned, until they do), drawn at the size it's placed at as an Adaptive
// Card (WidgetCard.cs), its buttons and toggles running the widget's operations. Windows starts this program as the
// package's COM server for the board (Program.cs), with no window: it reads the pages' widgets as the window does,
// with the sign-in the window keeps, when the board shows them and every few minutes while it does; the window pushes
// them too, whenever it sees them change. What each pinned widget shows is kept with it, as its custom state.
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Windows.Widgets;
using Microsoft.Windows.Widgets.Providers;

namespace One;

/// Every page's widgets, as this app last read them.
public static class WidgetCache
{
    public sealed record Item(string App, string Widget, string Scope, string Page, string Title, string? Error, JsonElement Root, Dictionary<string, JsonElement>? Sizes, bool Opens)
    {
        /// `<scope>/<app>.<widget>`, as the iPhone's: what a pinned widget's choice is found again by.
        public string Id => $"{Scope}/{App}.{Widget}";

        public JsonElement Tree(string size) => Sizes?.TryGetValue(size, out var t) == true ? t : Root;
    }

    /// Every page's widgets as last read, and when (UTC), kept on disk so the board's are drawn at once, before
    /// they're read again.
    public sealed record Kept(List<Item> Items, DateTime At);

    static string File => System.IO.Path.Combine(WhiteLabel.Folder, "Widgets.json");

    public static Kept? Last()
    {
        try { return JsonSerializer.Deserialize<Kept>(System.IO.File.ReadAllText(File)); }
        catch { return null; }
    }

    public static void Keep(List<Item> items, DateTime at)
    {
        try
        {
            System.IO.Directory.CreateDirectory(WhiteLabel.Folder);
            System.IO.File.WriteAllText(File, JsonSerializer.Serialize(new Kept(items, at)));
        }
        catch { }
    }

    public static void Forget()
    {
        try { System.IO.File.Delete(File); } catch { }
    }

    /// "Updated 2 hr ago", once what's shown was read over an hour ago (it's read every few minutes it can be).
    public static string? Updated(DateTime at)
    {
        var ago = DateTime.UtcNow - at;
        if (at == default || ago < TimeSpan.FromHours(1)) return null;
        return ago.TotalDays < 1 ? $"Updated {(int)ago.TotalHours} hr ago" : $"Updated {at.ToLocalTime():d MMM}";
    }

    /// The store's widgets, page by page, as the iPhone lists them; each opens its app's main screen if it has one.
    public static List<Item> Of(Store store) =>
        store.Pages().SelectMany(p => store.WidgetsOn(p).Select(w => new Item(w.App, w.Widget, p.Scope, p.Title, w.Title ?? store.App(w.App)?.Label ?? w.Widget, w.Error, w.Root, w.Sizes, store.App(w.App)?.HasScreens == true))).ToList();
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
        /// What went wrong the last time one of its buttons was used, until it's drawn again.
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
                if (Auth.Shared.SignedIn && WidgetCache.Last() is { } last) (items, readAt) = (last.Items, last.At);
                // Those pinned before it started, as they were left; the board's calls come only once it's registered.
                foreach (var info in WidgetManager.GetDefault().GetWidgetInfos())
                    pinned.TryAdd(info.WidgetContext.Id, Pinned.Of(info.WidgetContext, info.CustomState));
                Marshal.ThrowExceptionForHR(CoRegisterClassObject(Guid.Parse(WhiteLabel.WidgetClass), new Factory(), 0x4 /* local server */, 0x1 /* multiple use */, out var cookie));
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
                // Drawn as done at once (a toggle switched, a button greyed), then as its operation leaves it; back as it
                // was if it didn't work.
                var (app, op) = (data?["app"]?.GetValue<string>() ?? "", data?["op"]?.GetValue<string>() ?? "");
                var which = WidgetCard.Which(app, op, data?["input"] as JsonObject);
                WidgetCard.Doing[which] = data?["input"]?["on"] is JsonValue on && on.TryGetValue<bool>(out var to) ? to : true;
                DrawAll();
                _ = Task.Run(async () =>
                {
                    try
                    {
                        await Node.Call(app, op, data?["input"]?.DeepClone());
                        p.Problem = null;
                    }
                    catch (SignedOutException)
                    {
                        away = "signed out";
                    }
                    catch (Exception e)
                    {
                        p.Problem = $"Couldn’t do that: {e.Message}";
                    }
                    await Read(TimeSpan.Zero);
                    WidgetCard.Doing.TryRemove(which, out _);
                    DrawAll();
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
        if (items != null) Draw(context.Id, p);
        _ = Task.Run(async () =>
        {
            await Read(TimeSpan.FromSeconds(30));
            DrawAll();
        });
        timer ??= new Timer(_ => _ = Task.Run(async () =>
        {
            if (!pinned.Values.Any(x => x.Active)) return;
            await Read(TimeSpan.Zero);
            DrawAll();
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

    /// Reads every page's widgets, unless they were read within `fresh`.
    static async Task Read(TimeSpan fresh)
    {
        await reading.WaitAsync();
        try
        {
            if (Auth.Shared.Address == null || !Auth.Shared.SignedIn)
            {
                (items, away) = (null, "signed out");
                return;
            }
            if (items != null && away == null && DateTime.UtcNow - readAt < fresh) return;
            await Store.Shared.RefreshAll();
            items = WidgetCache.Of(Store.Shared);
            readAt = DateTime.UtcNow;
            away = null;
            WidgetCache.Keep(items, readAt);
        }
        catch (SignedOutException)
        {
            away = "signed out";
        }
        catch (Exception)
        {
            away = "unreachable";
        }
        finally
        {
            reading.Release();
        }
    }

    static void DrawAll()
    {
        foreach (var (id, p) in pinned) Draw(id, p);
    }

    static void Draw(string id, Pinned p)
    {
        // Signed out in the window since they were read: none of the person's widgets are drawn again.
        if (!Auth.Shared.SignedIn) (items, away) = (null, "signed out");
        Send(id, p, items, away, readAt);
    }

    /// What a pinned widget shows, sent to the board.
    static void Send(string id, Pinned p, List<WidgetCache.Item>? list, string? why, DateTime at)
    {
        (string template, string data) card;
        // One never chosen shows the first, as the iPhone's does.
        var item = p.Chosen == null ? list?.FirstOrDefault() : list?.FirstOrDefault(i => i.Id == p.Chosen);
        if (why == "signed out")
            card = WidgetCard.Saying($"Sign in to {WhiteLabel.Name} to see your pages.");
        else if (list == null)
            card = WidgetCard.Saying(why == null ? "Getting your widgets…" : $"Can’t reach {WhiteLabel.Name} right now. This tries again by itself.");
        else if (p.Choosing)
            card = WidgetCard.Chooser(list, p.Chosen);
        else if (item != null)
            card = Drawn(item, p.Size, p.Problem, WidgetCache.Updated(at));
        else if (p.Chosen != null && list.Count > 0)
            card = WidgetCard.Saying($"{p.Title ?? "This widget"} isn’t on your pages any more. Choose another with Customize, in this widget’s menu.");
        else
            card = WidgetCard.Saying($"Open {WhiteLabel.Name} to see your pages.");
        p.Problem = null;
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

    /// From the window, once it has read its widgets again: the pinned ones drawn with them, as the board has them.
    public static void Changed(List<WidgetCache.Item> list)
    {
        if (list.Count == 0) return;
        WidgetCache.Keep(list, DateTime.UtcNow);
        Each(p => !p.Choosing, (id, p) => Send(id, p, list, null, DateTime.UtcNow));
    }

    /// From the window, as the person signs out: every pinned widget says to sign in, at once.
    public static void SignedOut()
    {
        WidgetCache.Forget();
        Each(_ => true, (id, p) => Send(id, p, null, "signed out", default));
    }

    static void Each(Func<Pinned, bool> which, Action<string, Pinned> send)
    {
        try
        {
            foreach (var info in WidgetManager.GetDefault().GetWidgetInfos())
            {
                var p = Pinned.Of(info.WidgetContext, info.CustomState);
                if (which(p)) send(info.WidgetContext.Id, p);
            }
        }
        catch
        {
            // No widgets pinned, or no board on this computer.
        }
    }
}
