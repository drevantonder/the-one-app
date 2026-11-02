// Settings, as a page like the others: its title and the install's name, then its items' icons as a page's are drawn,
// you first, as your picture. Each opens its Settings screen (AppScreen), the same on every client.
using System;
using System.IO;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace One;

public static class SettingsView
{
    public static FrameworkElement Home(MainWindow window)
    {
        var store = Store.Shared;
        var grid = new VariableSizedWrapGrid { Orientation = Orientation.Horizontal, ItemWidth = 96, ItemHeight = 96 };
        foreach (var item in store.SettingsItems)
            grid.Children.Add(PageView.Tile(Icon(item, 60), item.Title, item.Badge ?? 0, () => window.Go(Route.SettingOf(item.Name))));
        var content = new StackPanel { Padding = new Thickness(32, 8, 32, 48) };
        content.Children.Add(Look.Header("Settings", store.Install?.Name ?? ""));
        content.Children.Add(grid);
        return new ScrollViewer { Content = content, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    }

    /// An item's icon: yours is your picture; the rest, its symbol on its colour, as an app's tile.
    public static FrameworkElement Icon(SettingsItem item, double size) =>
        item.Name == "account" && Store.Shared.Me is { } me ? Look.Avatar(me.Id, me.Name, size) : Look.Tile(item.Icon, item.Name, item.Color, null, null, size);
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
