// What the node sends, as this app reads it: what its sidebar lists and the Widgets board draws. Each property is one
// line, `[JsonPropertyName("key")] public T Name`, so tests/contracts.rs can read them and check them against what a
// node sends: `required` ones must be there, and each one there must have the type read here.
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace One;

public sealed class Config
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("cli")] public required string Cli { get; init; }
    [JsonPropertyName("accent")] public required string Accent { get; init; }
}

public sealed class NodeState
{
    [JsonPropertyName("config")] public required Config Config { get; init; }
    /// The node, to say which build it runs.
    [JsonPropertyName("server")] public Machine? Server { get; init; }
    [JsonPropertyName("user")] public Person? User { get; init; }
    [JsonPropertyName("install")] public Install? Install { get; init; }
    /// What this app's sign-in opens: some scopes and apps, or (null) all of them.
    [JsonPropertyName("sign_in")] public SignedIn? SignIn { get; init; }

    /// What Settings shows this person, in order.
    [JsonPropertyName("settings_items")] public List<SettingsItem>? SettingsItems { get; init; }
}

/// One of Settings' items: it opens the Settings screen it names, or with `app`, that app's main screen.
public sealed class SettingsItem
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("title")] public required string Title { get; init; }
    /// A Phosphor icon's name, on `color`.
    [JsonPropertyName("icon")] public required string Icon { get; init; }
    [JsonPropertyName("color")] public required string Color { get; init; }
    [JsonPropertyName("badge")] public int? Badge { get; init; }
    [JsonPropertyName("app")] public string? App { get; init; }
}

public sealed class SignedIn
{
    [JsonPropertyName("scopes")] public List<string>? Scopes { get; init; }
    [JsonPropertyName("apps")] public List<string>? Apps { get; init; }
}

public sealed class Machine
{
    [JsonPropertyName("key")] public required string Key { get; init; }
    [JsonPropertyName("name")] public required string Name { get; init; }
}

public sealed class Person
{
    [JsonPropertyName("id")] public required string Id { get; init; }
    [JsonPropertyName("name")] public required string Name { get; init; }
}

public sealed class Install
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("groups")] public required List<GroupInfo> Groups { get; init; }
    [JsonPropertyName("servers")] public required List<Server> Servers { get; init; }
}

public sealed class GroupInfo
{
    [JsonPropertyName("id")] public required string Id { get; init; }
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("members")] public required List<GroupMember> Members { get; init; }
}

public sealed class GroupMember
{
    [JsonPropertyName("user")] public required string User { get; init; }
}

public sealed class Server
{
    [JsonPropertyName("key")] public required string Key { get; init; }
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("build")] public string? Build { get; init; }
}

public sealed class AppInfo
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
    [JsonPropertyName("icon")] public string? Icon { get; init; }
    [JsonPropertyName("color")] public string? Color { get; init; }
    [JsonPropertyName("image")] public string? Image { get; init; }
    [JsonPropertyName("image_kind")] public string? ImageKind { get; init; }
    [JsonPropertyName("screens")] public List<AppScreenInfo>? Screens { get; init; }
    [JsonPropertyName("widgets")] public List<AppWidget>? Widgets { get; init; }
    [JsonPropertyName("installed")] public List<string>? Installed { get; init; }

    public string Label => Title ?? Name;
    public bool HasScreens => Screens?.Any(s => s.Name == "main") == true;
    public bool IsService => !HasScreens && (Widgets?.Count ?? 0) == 0;
    /// The pages it's on: "user" for its person's own, "group:<id>", "global".
    public IEnumerable<string> Scopes => (Installed ?? []).Select(s => s.StartsWith("user:") ? "user" : s);
}

public sealed class AppScreenInfo
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
}

public sealed class AppWidget
{
    [JsonPropertyName("name")] public required string Name { get; init; }
}

/// A rendered widget, for the Widgets board. Its trees are read as they are (WidgetCard.cs), skipping what it doesn't know.
public sealed class WidgetInfo
{
    [JsonPropertyName("app")] public required string App { get; init; }
    [JsonPropertyName("widget")] public required string Widget { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
    [JsonPropertyName("sizes")] public Dictionary<string, JsonElement>? Sizes { get; init; }
    [JsonPropertyName("error")] public string? Error { get; init; }
    [JsonPropertyName("root")] public required JsonElement Root { get; init; }
}
