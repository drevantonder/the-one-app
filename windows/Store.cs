// What the sidebar lists and the Widgets board draws: the node's state, its apps, the pages' widgets and badges. What's
// beside the sidebar is a screen the node serves, which reads its own. Changed raises after each load.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;

namespace One;

/// A page: a person's own, a group's, or everyone's.
public sealed record Page(string Scope, string Title);

/// Where the window is: one of this app's own screens (`own`, its name as `Screen`), the node's, the same on every
/// client: a page, a page's Services, a Settings screen or a request for a Vault item; or an app's screen. `Scope` is
/// the page it's for.
public sealed record Route(string Kind, string? Scope = null, string? App = null, string Screen = "main", string Params = "{}")
{
    public bool Own => Kind == "own";

    /// An own screen by its name (a Settings item's, say), for the page its `scope` names, if it names one.
    public static Route OwnOf(string name, string parameters = "{}")
    {
        string? scope = null;
        try { if (JsonNode.Parse(parameters)?["scope"] is JsonValue v && v.TryGetValue<string>(out var s)) scope = s; } catch { }
        return new("own", Scope: scope, Screen: name, Params: parameters);
    }

    public static Route PageOf(string scope) => OwnOf("pages", new JsonObject { ["scope"] = scope }.ToJsonString());
    public static Route Services(string scope) => OwnOf("services", new JsonObject { ["scope"] = scope }.ToJsonString());
    /// Settings, as a page of its items.
    public static readonly Route Settings = PageOf("settings");
    public static Route Request(string scope, string id) => OwnOf("request", new JsonObject { ["scope"] = scope, ["id"] = id }.ToJsonString());
    public static Route ScreenOf(string app, string? scope, string screen = "main", string parameters = "{}") => new("screen", Scope: scope, App: app, Screen: screen, Params: parameters);
}

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
    /// Each node event, for the open screens.
    public event Action<JsonNode>? Event;

    public Person? Me => State?.User;
    public Install? Install => State?.Install;
    public Config? Config => State?.Config;
    public AppInfo? App(string? name) => AppList.FirstOrDefault(a => a.Name == name);

    /// Every page this person can read that this app's sign-in opens: theirs, their groups', then everyone's.
    public List<Page> Pages()
    {
        var i = Install;
        var u = Me;
        if (i == null || u == null) return [];
        var out_ = new List<Page> { new("user", "You") };
        out_.AddRange(i.Groups.Where(g => g.Members.Any(m => m.User == u.Id)).OrderBy(g => g.Name).Select(g => new Page($"group:{g.Id}", g.Name)));
        out_.Add(new("global", "Everyone"));
        var opens = State?.SignIn?.Scopes;
        return opens == null ? out_ : out_.Where(p => opens.Contains(p.Scope == "user" ? $"user:{u.Id}" : p.Scope)).ToList();
    }

    public Page? PageFor(string? scope) => Pages().FirstOrDefault(p => p.Scope == scope);

    /// Whether this app's sign-in opens an app: every one, unless it was let use only some.
    public bool Opened(AppInfo a) => State?.SignIn?.Apps is not { } apps || apps.Contains(a.Name);

    public List<AppInfo> AppsOn(Page p) => AppList.Where(a => a.HasScreens && Opened(a) && a.Scopes.Contains(p.Scope)).ToList();
    public List<AppInfo> ServicesOn(Page p) => AppList.Where(a => a.IsService && Opened(a) && a.Scopes.Contains(p.Scope)).OrderBy(a => a.Label).ToList();
    public int Badge(string app, Page p) => Math.Max(0, BadgesOf.GetValueOrDefault(p.Scope)?.GetValueOrDefault(app) ?? 0);

    /// Every widget a page could show: those of the apps on it.
    public List<WidgetInfo> WidgetsOn(Page p)
    {
        var here = AppList.Where(a => a.Scopes.Contains(p.Scope)).Select(a => a.Name).ToHashSet();
        return (WidgetsOf.GetValueOrDefault(p.Scope) ?? []).Where(w => here.Contains(w.App)).ToList();
    }

    /// What Settings shows this person, in order, as the node says.
    public List<SettingsItem> SettingsItems => State?.SettingsItems ?? [];

    /// The count on the app's taskbar button, as an iPhone's icon has it: every app's badge on every page, and the
    /// requests waiting.
    public int IconBadge => (SettingsItems.FirstOrDefault(i => i.Name == "requests")?.Badge ?? 0) + BadgesOf.Values.SelectMany(b => b.Values).Sum(n => Math.Max(0, n));

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
        await Task.WhenAll(Pages().Select(async p =>
        {
            try { WidgetsOf[p.Scope] = await Node.Widgets(p.Scope); } catch { }
            try { BadgesOf[p.Scope] = await Node.Badges(p.Scope); } catch { BadgesOf[p.Scope] = []; }
        }));
        Changed?.Invoke();
        WidgetsRead?.Invoke();
    }

    /// Follows the node's events until signed out: what changed, so the window and the open screens keep up.
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

    /// Back after a while away (asleep, say), when events may have been missed: everything read again, and every open
    /// screen told something may have changed.
    public async Task CaughtUp()
    {
        try { await RefreshState(); } catch { }
        Event?.Invoke(new JsonObject { ["type"] = "install" });
    }

    async Task Handle(JsonNode e)
    {
        Event?.Invoke(e);
        var type = e["type"]?.GetValue<string>();
        try
        {
            if (type is "devices" or "install") await RefreshState();
            if (type == "changed")
            {
                if ((e["apps"] as JsonArray)?.Count is null or 0) await RefreshState();
                await RefreshApps();
            }
        }
        catch { }
    }

    public void Clear()
    {
        State = null;
        AppList = [];
        WidgetsOf.Clear();
        BadgesOf.Clear();
    }
}
