// What the node sends, as this app reads it. Each property is one line, `[JsonPropertyName("key")] public T Name`, so
// tests/contracts.rs can read them and check them against what a node sends: `required` ones must be there, and each
// one there must have the type read here.
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
    /// The app the App Store opens.
    [JsonPropertyName("store")] public string? Store { get; init; }
}

public sealed class NodeState
{
    [JsonPropertyName("config")] public required Config Config { get; init; }
    /// The node, to say which build it runs.
    [JsonPropertyName("server")] public Machine? Server { get; init; }
    [JsonPropertyName("me")] public Machine? Me { get; init; }
    [JsonPropertyName("user")] public Person? User { get; init; }
    [JsonPropertyName("install")] public Install? Install { get; init; }
    [JsonPropertyName("vault_requests")] public int? VaultRequests { get; init; }
    [JsonPropertyName("secret_requests")] public int? SecretRequests { get; init; }
    [JsonPropertyName("arrangements")] public Dictionary<string, Arrangement>? Arrangements { get; set; }
    /// What this app's sign-in opens: some scopes and apps, or (null) all of them.
    [JsonPropertyName("sign_in")] public SignedIn? SignIn { get; init; }

    /// What Settings shows this person, in order.
    [JsonPropertyName("settings_items")] public List<SettingsItem>? SettingsItems { get; init; }

    public int Waiting => VaultRequests ?? SecretRequests ?? 0;
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
    [JsonPropertyName("read_only")] public bool? ReadOnly { get; init; }
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
    [JsonPropertyName("role")] public required string Role { get; init; }

    public bool Admin => Role == "admin";
}

public sealed class Install
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("users")] public required List<Person> Users { get; init; }
    [JsonPropertyName("groups")] public required List<GroupInfo> Groups { get; init; }
    [JsonPropertyName("servers")] public required List<Server> Servers { get; init; }
    [JsonPropertyName("clients")] public required List<ClientInfo> Clients { get; init; }
}

public sealed class GroupInfo
{
    [JsonPropertyName("id")] public required string Id { get; init; }
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("members")] public required List<GroupMember> Members { get; init; }
    [JsonPropertyName("invited")] public List<GroupMember>? Invited { get; init; }
}

public sealed class GroupMember
{
    [JsonPropertyName("user")] public required string User { get; init; }
    [JsonPropertyName("role")] public required string Role { get; init; }
}

public sealed class Server
{
    [JsonPropertyName("key")] public required string Key { get; init; }
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("user")] public required string User { get; init; }
    [JsonPropertyName("online")] public bool? Online { get; init; }
    [JsonPropertyName("build")] public string? Build { get; init; }
    [JsonPropertyName("load")] public Load? Load { get; init; }
}

public sealed class Load
{
    [JsonPropertyName("memory")] public Space? Memory { get; init; }
    [JsonPropertyName("cpu")] public List<int>? Cpu { get; init; }
}

public sealed class Space
{
    [JsonPropertyName("used")] public required double Used { get; init; }
    [JsonPropertyName("total")] public required double Total { get; init; }
}

public sealed class ClientInfo
{
    [JsonPropertyName("key")] public required string Key { get; init; }
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("user")] public required string User { get; init; }
}

public sealed class Arrangement
{
    [JsonPropertyName("items")] public required List<ArrangedItem> Items { get; init; }
    [JsonPropertyName("hidden")] public List<string>? Hidden { get; init; }
}

/// An app's icon on a page, or (with `widget` and `id`) one copy of a widget, at its size, with its options.
public sealed class ArrangedItem
{
    [JsonPropertyName("app")] public required string App { get; init; }
    [JsonPropertyName("widget")] public string? Widget { get; init; }
    [JsonPropertyName("id")] public string? Id { get; init; }
    [JsonPropertyName("size")] public string? Size { get; init; }
    [JsonPropertyName("options")] public Dictionary<string, JsonElement>? Options { get; init; }
}

public sealed class AppInfo
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
    [JsonPropertyName("description")] public string? Description { get; init; }
    [JsonPropertyName("icon")] public string? Icon { get; init; }
    [JsonPropertyName("color")] public string? Color { get; init; }
    [JsonPropertyName("image")] public string? Image { get; init; }
    [JsonPropertyName("image_kind")] public string? ImageKind { get; init; }
    [JsonPropertyName("owner")] public string? Owner { get; init; }
    [JsonPropertyName("screens")] public List<AppScreenInfo>? Screens { get; init; }
    [JsonPropertyName("ops")] public List<AppOp>? Ops { get; init; }
    [JsonPropertyName("widgets")] public List<AppWidget>? Widgets { get; init; }
    [JsonPropertyName("installed")] public List<string>? Installed { get; init; }
    [JsonPropertyName("deploys")] public int? Deploys { get; init; }
    [JsonPropertyName("vault")] public Dictionary<string, VaultNeed>? Vault { get; init; }

    public string Label => Title ?? Name;
    public bool Has(string screen) => Screens?.Any(s => s.Name == screen) == true;
    public bool HasScreens => Has("main");
    public bool IsService => !HasScreens && (Widgets?.Count ?? 0) == 0;
    public bool HasAccounts => Ops?.Any(o => o.Name == "accounts") == true;
    public bool KeepsAccounts => Vault?.Values.Any(v => v.Accounts == true) == true;
    /// The pages it's on: "user" for its person's own, "group:<id>", "global".
    public IEnumerable<string> Scopes => (Installed ?? []).Select(s => s.StartsWith("user:") ? "user" : s);
}

public sealed class AppScreenInfo
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
}

public sealed class AppOp
{
    [JsonPropertyName("name")] public required string Name { get; init; }
}

public sealed class AppWidget
{
    [JsonPropertyName("name")] public required string Name { get; init; }
}

public sealed class VaultNeed
{
    [JsonPropertyName("title")] public string? Title { get; init; }
    [JsonPropertyName("accounts")] public bool? Accounts { get; init; }
}

/// A rendered widget. Its trees are read as they are (WidgetTree.cs), skipping what it doesn't know.
public sealed class WidgetInfo
{
    [JsonPropertyName("app")] public required string App { get; init; }
    [JsonPropertyName("widget")] public required string Widget { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
    [JsonPropertyName("size")] public string? Size { get; init; }
    [JsonPropertyName("sizes")] public Dictionary<string, JsonElement>? Sizes { get; init; }
    [JsonPropertyName("error")] public string? Error { get; init; }
    [JsonPropertyName("root")] public required JsonElement Root { get; init; }
    [JsonPropertyName("placed")] public Dictionary<string, WidgetPlaced>? Placed { get; init; }

    public static readonly string[] AllSizes = ["small", "medium", "large", "xl"];
    /// The sizes it draws, smallest first.
    public IEnumerable<string> Supported => AllSizes.Where(s => Sizes == null || Sizes.ContainsKey(s));
}

public sealed class WidgetPlaced
{
    [JsonPropertyName("sizes")] public Dictionary<string, JsonElement>? Sizes { get; init; }
    [JsonPropertyName("error")] public string? Error { get; init; }
}

public sealed class VaultItem
{
    [JsonPropertyName("id")] public required string Id { get; init; }
    [JsonPropertyName("scope")] public required string Scope { get; init; }
    [JsonPropertyName("scope_name")] public string? ScopeName { get; init; }
    [JsonPropertyName("title")] public required string Title { get; init; }
    [JsonPropertyName("type")] public string? Type { get; init; }
    [JsonPropertyName("image")] public string? Image { get; init; }
    [JsonPropertyName("apps")] public required List<string> Apps { get; init; }
    [JsonPropertyName("wanted_by")] public List<string>? WantedBy { get; init; }
}

public sealed class VaultRequest
{
    [JsonPropertyName("id")] public required string Id { get; init; }
    [JsonPropertyName("scope")] public required string Scope { get; init; }
    [JsonPropertyName("scope_name")] public required string ScopeName { get; init; }
    [JsonPropertyName("app")] public required string App { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
    [JsonPropertyName("fields")] public List<VaultField>? Fields { get; init; }
    [JsonPropertyName("hosts")] public required List<string> Hosts { get; init; }
    [JsonPropertyName("why")] public string? Why { get; init; }
    [JsonPropertyName("by")] public required string By { get; init; }
    [JsonPropertyName("agent")] public string? Agent { get; init; }
    [JsonPropertyName("state")] public required string State { get; init; }
    [JsonPropertyName("matches")] public List<VaultMatch>? Matches { get; init; }

    public string What => Title ?? "";
    public string Asker => string.IsNullOrEmpty(Agent) ? By : Agent;
    public List<VaultField> Wanted => Fields ?? new List<VaultField>();
}

public sealed class VaultField
{
    [JsonPropertyName("name")] public required string Name { get; init; }
    [JsonPropertyName("secret")] public required bool Secret { get; init; }
}

public sealed class VaultMatch
{
    [JsonPropertyName("id")] public required string Id { get; init; }
    [JsonPropertyName("title")] public required string Title { get; init; }
    [JsonPropertyName("type")] public string? Type { get; init; }
}

public sealed class NodeEvent
{
    [JsonPropertyName("type")] public required string Type { get; init; }
    [JsonPropertyName("apps")] public List<string>? Apps { get; init; }
}
