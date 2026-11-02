// Settings, as the Mac app's: who you are and signing out, then what this app has of the Mac app's Settings: the
// requests waiting for you, the App Store, your devices, the Vault, your passkeys and how the window looks.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace One;

public static class SettingsView
{
    static ScrollViewer Scroll(params FrameworkElement[] parts)
    {
        var panel = new StackPanel { Spacing = 24 };
        foreach (var p in parts) panel.Children.Add(p);
        return Look.Column(panel);
    }

    static FrameworkElement Chevron(string glyph, string color, string title, Action open, string? value = null, int badge = 0) =>
        Look.Row(Look.SymbolTile(glyph, color), title, end: badge > 0 ? Look.Count(badge) : value == null ? null : Look.Text(value, 14, Look.Secondary), click: open, chevron: true);

    public static FrameworkElement Home(MainWindow window)
    {
        var store = Store.Shared;
        var me = store.Me!;
        var install = store.Install!;
        var profile = new Grid { ColumnSpacing = 14, Padding = new Thickness(16) };
        profile.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        profile.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        profile.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        profile.Children.Add(Look.Avatar(me.Id, me.Name, 60));
        var who = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        who.Children.Add(Look.Text(me.Name, 20, semibold: true));
        who.Children.Add(Look.Text(me.Admin ? $"Admin of {install.Name}" : $"Member of {install.Name}", 13, Look.Secondary));
        who.Children.Add(Look.Text(Environment.MachineName, 13, Look.Secondary));
        Grid.SetColumn(who, 1);
        profile.Children.Add(who);
        var signOut = Look.Plain("Sign Out", () => _ = window.SignOut());
        signOut.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(signOut, 2);
        profile.Children.Add(signOut);

        var parts = new List<FrameworkElement> { Look.Header("Settings", install.Name), Look.Section(rows: profile) };
        if (store.Requests > 0) parts.Add(Look.Section(rows: Chevron("", "#ff9500", "Requests", () => _ = VaultSheet.Requests(window.Content.XamlRoot), badge: store.Requests)));
        var places = new List<FrameworkElement>();
        if (store.StoreApp() is { } shop) places.Add(Look.Row(Look.Tile(shop, 28), "App Store", click: () => window.Push(store.ScreenRoute(shop, null)), chevron: true));
        places.Add(Chevron("", "#007aff", "Devices", () => window.Push(Route.SettingOf("devices"))));
        parts.Add(Look.Section(rows: places.ToArray()));
        parts.Add(Look.Section(rows:
        [
            Chevron("", "#8e8e93", "Vault", () => window.Push(Route.SettingOf("vault"))),
            Chevron("", "#34c759", "Passkeys", () => Shell.Open($"{Auth.Shared.Address}/passkeys")),
            Chevron("", "#5856d6", "Appearance", () => window.Push(Route.SettingOf("appearance")), Appearance.Title(Appearance.Chosen)),
        ]));
        var server = install.Servers.FirstOrDefault(s => s.Key == store.State?.Server?.Key || s.Key == store.State?.Me?.Key);
        var version = $"{Brand.Name}, app {Brand.Commit}" + (server?.Build is { } build ? $"\n{server.Name} runs {build}" : "");
        parts.Add(new TextBlock { Text = version, FontSize = 12, Foreground = Look.Secondary, TextAlignment = TextAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, IsTextSelectionEnabled = true });
        return Scroll(parts.ToArray());
    }

    /// This person's devices, and the servers they can see.
    public static FrameworkElement Devices()
    {
        var store = Store.Shared;
        var me = store.Me!.Id;
        var install = store.Install!;
        static string Glyph(string name)
        {
            var n = name.ToLowerInvariant();
            return n.Contains("phone") ? "" : n.Contains("ipad") ? "" : n.Contains("macbook") || n.Contains("laptop") ? "" : n.Contains("mac") || n.Contains("windows") || n.Contains("pc") ? "" : "";
        }
        static FrameworkElement Device(string name, string? sub, bool? online)
        {
            var dot = online == null ? null : new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 8, Height = 8, Fill = online == true ? Look.Toned("good") : Look.Tertiary };
            return Look.Row(new FontIcon { Glyph = Glyph(name), FontFamily = Look.Symbols, FontSize = 20, Foreground = Look.Secondary, Width = 28 }, name.EndsWith(".local") ? name[..^6] : name, sub, dot);
        }
        string Load(Server s)
        {
            var parts = new List<string>();
            if (s.Load?.Memory is { Total: > 0 } m) parts.Add($"Memory {m.Used / 1_073_741_824:0.0} of {m.Total / 1_073_741_824:0.0} GB");
            if (s.Load?.Cpu?.LastOrDefault() is { } c) parts.Add($"CPU {c}%");
            return parts.Count > 0 ? string.Join(" · ", parts) : s.Online == true ? "Online" : "Offline";
        }
        var mine = install.Servers.Where(s => s.User == me).Select(s => Device(s.Name, Load(s), s.Online == true))
            .Concat(install.Clients.Where(c => c.User == me).Select(c => Device(c.Name, "Phone or computer", null))).ToArray();
        var others = install.Servers.Where(s => s.User != me).Select(s => Device(s.Name, $"{store.UserName(s.User)}’s · {Load(s)}", s.Online == true)).ToArray();
        var parts = new List<FrameworkElement> { Look.Header("Devices", install.Name) };
        parts.Add(Look.Section("Your Devices", null, mine.Length > 0 ? mine : [Look.Row(null, "None yet", titleBrush: Look.Secondary)]));
        if (others.Length > 0) parts.Add(Look.Section("Servers", null, others));
        parts.Add(Look.Text($"This computer is signed in as {store.Me!.Name}, through {new Uri(Auth.Shared.Address!).Host}.", 12, Look.Secondary, wrap: true));
        return Scroll(parts.ToArray());
    }

    /// The person's items in the Vault, and which apps use each. Their secrets never leave the node.
    public static FrameworkElement Vault(MainWindow window)
    {
        var holder = new ContentControl { HorizontalContentAlignment = HorizontalAlignment.Stretch };
        holder.Content = Scroll(Look.Header("Vault", "Keys, sign-ins and accounts your apps use without seeing them"), new ProgressRing { IsActive = true, HorizontalAlignment = HorizontalAlignment.Left });
        _ = Fill();
        async Task Fill()
        {
            List<VaultItem> items;
            try { items = await Node.Vault(); }
            catch (Exception e)
            {
                holder.Content = Scroll(Look.Header("Vault", ""), Look.Text(e.Message, 14, Look.Toned("bad"), wrap: true));
                return;
            }
            var store = Store.Shared;
            var parts = new List<FrameworkElement> { Look.Header("Vault", "Keys, sign-ins and accounts your apps use without seeing them") };
            foreach (var scope in items.GroupBy(i => i.ScopeName ?? i.Scope))
            {
                parts.Add(Look.Section(scope.Key, null, scope.OrderBy(i => i.Title).Select(i =>
                {
                    var users = i.Apps.Select(a => store.App(a)?.Label ?? a).ToList();
                    var sub = string.Join(" · ", new[] { i.Type != i.Title ? i.Type : null, users.Count > 0 ? $"Used by {string.Join(", ", users)}" : "No app uses it" }.Where(s => !string.IsNullOrEmpty(s)));
                    return Look.Row(Look.Tile(null, i.Title, null, i.Image, "icon", 30), i.Title, sub);
                }).ToArray()));
            }
            if (items.Count == 0) parts.Add(Look.Text("Nothing in the Vault yet. When an app asks for a key or a sign-in, it’s kept here.", 14, Look.Secondary, wrap: true));
            holder.Content = Scroll(parts.ToArray());
        }
        return holder;
    }

    public static FrameworkElement AppearanceView(MainWindow window)
    {
        string[] kinds = ["automatic", "light", "dark"];
        var choices = new RadioButtons { ItemsSource = kinds.Select(Appearance.Title).ToList() };
        choices.Loaded += (_, _) => choices.SelectedIndex = Array.IndexOf(kinds, Appearance.Chosen);
        choices.SelectionChanged += (_, _) =>
        {
            if (choices.SelectedIndex >= 0 && kinds[choices.SelectedIndex] != Appearance.Chosen) Appearance.Choose(kinds[choices.SelectedIndex], window);
        };
        return Scroll(Look.Header("Appearance", "On this computer"), choices);
    }
}

/// Light, Dark or Automatic, as chosen on this computer.
public static class Appearance
{
    static string File => Path.Combine(Brand.Folder, "appearance");

    public static string Chosen
    {
        get
        {
            try { return System.IO.File.ReadAllText(File).Trim() is "light" or "dark" ? System.IO.File.ReadAllText(File).Trim() : "automatic"; }
            catch { return "automatic"; }
        }
    }

    public static string Title(string a) => a switch { "light" => "Light", "dark" => "Dark", _ => "Automatic" };

    public static ElementTheme Theme => Chosen switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };

    public static void Choose(string a, MainWindow window)
    {
        Directory.CreateDirectory(Brand.Folder);
        System.IO.File.WriteAllText(File, a);
        window.Restyle();
    }
}
