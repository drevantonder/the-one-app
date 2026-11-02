// What the window shows: the node's state, its apps, widgets and badges, and what follows from them, as the web
// client and the Mac app work it out (kernel/web/web.js, apple/Shared/Store.swift). Changed raises after each load.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;

namespace One;

/// A page: a person's own, a group's, or everyone's.
public sealed record Page(string Scope, string Title, string Subtitle, string Whose, string? Group = null);

/// Where the window is: a page, a page's Services, an app's screen, Settings or one of its screens.
public sealed record Route(string Kind, string? Scope = null, string? App = null, string Screen = "main", string Params = "{}", string? Setting = null)
{
    public static Route PageOf(string scope) => new("page", Scope: scope);
    public static Route Services(string scope) => new("services", Scope: scope);
    public static Route ScreenOf(string app, string? scope, string screen = "main", string parameters = "{}") => new("screen", Scope: scope, App: app, Screen: screen, Params: parameters);
    public static readonly Route Settings = new("settings");
    public static Route SettingOf(string name, string parameters = "{}") => new("settings", Params: parameters, Setting: name);
}

/// What a page shows, in order: an app's icon, or a copy of a widget as it's placed.
public sealed record PageItem(AppInfo? App, WidgetInfo? Widget = null, ArrangedItem? Kept = null, string Size = "medium");

public sealed class Store
{
    public static readonly Store Shared = new();

    public NodeState? State { get; private set; }
    public List<AppInfo> AppList { get; private set; } = [];
    public Dictionary<string, List<WidgetInfo>> WidgetsOf { get; } = [];
    public Dictionary<string, Dictionary<string, int>> BadgesOf { get; } = [];

    /// After anything it holds changes.
    public event Action? Changed;
    /// After the pages' widgets are read again.
    public event Action? WidgetsRead;
    /// Each node event, for the open screen.
    public event Action<JsonNode>? Event;

    public Person? Me => State?.User;
    public Install? Install => State?.Install;
    public bool IsAdmin => Me?.Admin == true;
    public Config? Config => State?.Config;
    public AppInfo? App(string? name) => AppList.FirstOrDefault(a => a.Name == name);

    public string UserName(string id) => id == Me?.Id ? "You" : Install?.Users.FirstOrDefault(u => u.Id == id)?.Name ?? id[..Math.Min(6, id.Length)];

    static string People(int n) => n == 1 ? "1 person" : $"{n} people";

    string ChosenBy(IEnumerable<string> admins)
    {
        var names = admins.OrderBy(a => a == Me?.Id ? 0 : 1).ThenBy(UserName).Select(a => a == Me?.Id ? "you" : UserName(a)).ToList();
        return names.Count switch
        {
            0 => "apps chosen by admins",
            1 => $"apps chosen by {names[0]}",
            2 => $"apps chosen by {names[0]} and {names[1]}",
            _ => $"apps chosen by {names.Count} admins",
        };
    }

    /// Every page this person can read that this app's sign-in opens: theirs, their groups', then everyone's.
    public List<Page> Pages()
    {
        var i = Install;
        var u = Me;
        if (i == null || u == null) return [];
        var devices = i.Servers.Count(s => s.User == u.Id) + i.Clients.Count(c => c.User == u.Id);
        var out_ = new List<Page> { new("user", "You", devices == 1 ? "Just you · 1 device" : $"Just you · {devices} devices", "For you only") };
        foreach (var g in i.Groups.Where(g => g.Members.Any(m => m.User == u.Id)).OrderBy(g => g.Name))
        {
            var admins = g.Members.Where(m => m.Role == "admin").Select(m => m.User);
            out_.Add(new($"group:{g.Id}", g.Name, $"{People(g.Members.Count)} · {ChosenBy(admins)}", $"For {g.Name}", g.Id));
        }
        out_.Add(new("global", "Everyone", $"{People(i.Users.Count)} · {ChosenBy(i.Users.Where(x => x.Admin).Select(x => x.Id))}", "For everyone"));
        var opens = State?.SignIn?.Scopes;
        return opens == null ? out_ : out_.Where(p => opens.Contains(p.Scope == "user" ? $"user:{u.Id}" : p.Scope)).ToList();
    }

    public Page? PageFor(string? scope) => Pages().FirstOrDefault(p => p.Scope == scope);

    /// Whether this person may add apps to a page: their own, a group they admin, or everyone's as an admin.
    public bool CanArrange(string scope)
    {
        if (scope == "user") return true;
        if (scope == "global") return IsAdmin;
        var g = Install?.Groups.FirstOrDefault(g => $"group:{g.Id}" == scope);
        return g?.Members.Any(m => m.User == Me?.Id && m.Role == "admin") == true;
    }

    /// Whether this app's sign-in opens an app: every one, unless it was let use only some.
    public bool Opened(AppInfo a) => State?.SignIn?.Apps is not { } apps || apps.Contains(a.Name);

    public List<AppInfo> AppsOn(Page p) => AppList.Where(a => a.HasScreens && Opened(a) && a.Scopes.Contains(p.Scope)).ToList();
    public List<AppInfo> ServicesOn(Page p) => AppList.Where(a => a.IsService && Opened(a) && a.Scopes.Contains(p.Scope)).OrderBy(a => a.Label).ToList();
    public int Badge(string app, Page p) => Math.Max(0, BadgesOf.GetValueOrDefault(p.Scope)?.GetValueOrDefault(app) ?? 0);

    /// Whether this person owns an app: deleting it is theirs.
    public bool Owns(AppInfo a)
    {
        if (a.Owner is not { } owner || Me is not { } u) return false;
        if (owner == "admins") return IsAdmin;
        if (owner == $"user:{u.Id}") return true;
        var g = owner.StartsWith("group:") ? Install?.Groups.FirstOrDefault(g => g.Id == owner[6..]) : null;
        return g?.Members.Any(m => m.User == u.Id && m.Role == "admin") == true;
    }

    /// Every widget a page could show: those of the apps on it.
    public List<WidgetInfo> WidgetsOn(Page p)
    {
        var here = AppList.Where(a => a.Scopes.Contains(p.Scope)).Select(a => a.Name).ToHashSet();
        return (WidgetsOf.GetValueOrDefault(p.Scope) ?? []).Where(w => here.Contains(w.App)).ToList();
    }

    /// What a page shows, in its person's order: the widgets they put there and its apps' icons, those added since
    /// they last arranged it at the end, without those they took off it. A page never arranged shows its apps.
    public List<PageItem> Items(Page p)
    {
        var apps = AppsOn(p);
        var widgets = WidgetsOn(p);
        if (State?.Arrangements?.GetValueOrDefault(p.Scope) is not { } arrangement) return apps.Select(a => new PageItem(a)).ToList();
        var out_ = new List<PageItem>();
        var seen = new HashSet<string>();
        foreach (var item in arrangement.Items)
        {
            if (item.Widget != null)
            {
                var w = widgets.FirstOrDefault(w => w.App == item.App && w.Widget == item.Widget);
                if (item.Id == null || w == null || !seen.Add($"widget:{item.Id}")) continue;
                var size = item.Size != null && w.Supported.Contains(item.Size) ? item.Size : w.Size ?? "medium";
                out_.Add(new PageItem(App(w.App), w, item, size));
            }
            else if (apps.FirstOrDefault(a => a.Name == item.App) is { } a && seen.Add($"app:{a.Name}"))
            {
                out_.Add(new PageItem(a));
            }
        }
        var hidden = (arrangement.Hidden ?? []).ToHashSet();
        out_.AddRange(apps.Where(a => !seen.Contains($"app:{a.Name}") && !hidden.Contains(a.Name)).Select(a => new PageItem(a)));
        return out_;
    }

    /// A widget's tree at `size` for one copy of it: with the options its person chose, if they did.
    public static JsonElement Tree(WidgetInfo w, string size, ArrangedItem? kept)
    {
        if (kept?.Id is { } id && w.Placed?.GetValueOrDefault(id)?.Sizes?.TryGetValue(size, out var mine) == true) return mine;
        return w.Sizes?.TryGetValue(size, out var tree) == true ? tree : w.Root;
    }

    public static string? PlacedError(WidgetInfo w, ArrangedItem? kept) => kept?.Id is { } id ? w.Placed?.GetValueOrDefault(id)?.Error ?? w.Error : w.Error;

    /// The install's App Store, if it's on a page this app's sign-in opens.
    public AppInfo? StoreApp()
    {
        var s = App(Config?.Store);
        return s != null && s.HasScreens && Opened(s) && Pages().Any(p => s.Scopes.Contains(p.Scope)) ? s : null;
    }

    public Route ScreenRoute(AppInfo a, Page? p, string screen = "main", JsonObject? parameters = null) =>
        Route.ScreenOf(a.Name, p?.Scope, screen, (parameters ?? new JsonObject()).ToJsonString());

    public Route? SettingsRoute(AppInfo a, Page p, bool add = false) => a.Has("settings") ? ScreenRoute(a, p, "settings", add ? new JsonObject { ["add"] = true } : null) : null;

    public Route? StoreRoute(AppInfo a) =>
        StoreApp() is { } s && s.Has("app") && (a.Deploys ?? 0) > 0 ? ScreenRoute(s, null, "app", new JsonObject { ["app"] = a.Name }) : null;

    public Route? ServiceStoreRoute() => StoreApp() is { } s ? ScreenRoute(s, null, "main", new JsonObject { ["service"] = true }) : null;

    /// What Settings shows this person, in order, as the node says.
    public List<SettingsItem> SettingsItems => State?.SettingsItems ?? [];
    public SettingsItem? SettingsItemOf(string? name) => SettingsItems.FirstOrDefault(i => i.Name == name);

    // MARK: Loading

    /// Loads the node's state, then its apps, widgets and badges if who this is or their groups changed.
    public async Task RefreshState()
    {
        var state = await Node.State();
        var regrouped = JsonSerializer.Serialize(new object?[] { state.User?.Id, state.Install?.Groups.Select(g => g.Id + g.Members.Count) })
            != JsonSerializer.Serialize(new object?[] { State?.User?.Id, State?.Install?.Groups.Select(g => g.Id + g.Members.Count) });
        var first = State == null;
        State = state;
        if (first || regrouped || AppList.Count == 0) await RefreshApps();
        else Changed?.Invoke();
    }

    /// Everything again, as the Widgets board's widgets read it.
    public async Task RefreshAll()
    {
        State = await Node.State();
        await RefreshApps();
    }

    public async Task RefreshApps()
    {
        if (Install == null) return;
        AppList = await Node.Apps();
        await RefreshWidgets();
    }

    public async Task RefreshWidgets()
    {
        await Task.WhenAll(Pages().Select(async p =>
        {
            try { WidgetsOf[p.Scope] = await Node.Widgets(p.Scope); } catch { }
            try { BadgesOf[p.Scope] = await Node.Badges(p.Scope); } catch { BadgesOf[p.Scope] = []; }
        }));
        Changed?.Invoke();
        WidgetsRead?.Invoke();
        ScheduleRedraw();
    }

    CancellationTokenSource? redraw;

    /// Draws the widgets again when the soonest asks to be (a tree's `next`), between a minute and fifteen from now.
    void ScheduleRedraw()
    {
        redraw?.Cancel();
        var nexts = new List<double>();
        void Walk(JsonElement n)
        {
            if (n.ValueKind != JsonValueKind.Object) return;
            if (n.TryGetProperty("next", out var next) && next.ValueKind == JsonValueKind.Number) nexts.Add(next.GetDouble());
            if (n.TryGetProperty("c", out var c) && c.ValueKind == JsonValueKind.Array) foreach (var child in c.EnumerateArray()) Walk(child);
        }
        foreach (var w in WidgetsOf.Values.SelectMany(l => l))
        {
            if (w.Sizes != null) foreach (var t in w.Sizes.Values) Walk(t);
            else Walk(w.Root);
        }
        if (nexts.Count == 0) return;
        var now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        var wait = Math.Min(15 * 60_000, Math.Max(60_000, nexts.Min() - now));
        var cancel = redraw = new CancellationTokenSource();
        _ = Task.Delay(TimeSpan.FromMilliseconds(wait), cancel.Token).ContinueWith(async t =>
        {
            if (!t.IsCanceled) try { await RefreshWidgets(); } catch { }
        }, SynchronizationContext.Current == null ? TaskScheduler.Default : TaskScheduler.FromCurrentSynchronizationContext());
    }

    /// Follows the node's events until signed out: what changed, so the window and the open screen keep up.
    public async Task Listen(Action signedOut, CancellationToken cancel)
    {
        while (!cancel.IsCancellationRequested)
        {
            try
            {
                await Node.Events(e => _ = Handle(e), cancel);
            }
            catch (SignedOutException)
            {
                signedOut();
                return;
            }
            catch { }
            try { await Task.Delay(3000, cancel); } catch { return; }
        }
    }

    async Task Handle(JsonNode e)
    {
        Event?.Invoke(e);
        var type = e["type"]?.GetValue<string>();
        try
        {
            if (type is "servers" or "install") await RefreshState();
            if (type == "changed")
            {
                if ((e["apps"] as JsonArray)?.Count is null or 0) await RefreshState();
                await RefreshApps();
            }
        }
        catch { }
    }

    /// Keeps a page in its person's order with `edit` applied to each item (null takes it off).
    public async Task Arrange(Page p, Func<ArrangedItem, ArrangedItem?> edit)
    {
        var kept = Items(p).Select(i => i.Widget == null ? new ArrangedItem { App = i.App!.Name } : i.Kept!).Select(edit).OfType<ArrangedItem>().ToList();
        var hidden = State?.Arrangements?.GetValueOrDefault(p.Scope)?.Hidden ?? [];
        if (State != null)
        {
            State.Arrangements ??= [];
            State.Arrangements[p.Scope] = new Arrangement { Items = kept, Hidden = hidden };
        }
        Changed?.Invoke();
        await Node.Arrange(p.Scope, kept, hidden);
    }

    /// Runs a change, then takes in what it changed.
    public async Task Change(Func<Task> work)
    {
        await work();
        await RefreshState();
        await RefreshApps();
    }

    public void Clear()
    {
        State = null;
        AppList = [];
        WidgetsOf.Clear();
        BadgesOf.Clear();
        redraw?.Cancel();
    }
}
