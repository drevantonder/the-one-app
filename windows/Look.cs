// The look of the window's own parts (the sidebar, the welcome): the install's accent, app icons as tiles, people's
// initials, the icons apps name (Phosphor's), and light or dark.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.UI;

namespace One;

public static class Look
{
    /// A colour from `#rrggbb`, lifted a little toward white for dark mode, as the Apple apps do, so it keeps its contrast.
    public static Color Hex(string? hex, bool dark = false)
    {
        var text = (hex ?? "").TrimStart('#');
        if (text.Length == 3) text = string.Concat(text.Select(c => $"{c}{c}"));
        if (text.Length != 6 || !uint.TryParse(text, System.Globalization.NumberStyles.HexNumber, null, out var v)) v = 0x808080;
        byte Lift(uint c) => (byte)(dark ? c + (255 - c) * 0.2 : c);
        return Color.FromArgb(255, Lift(v >> 16 & 0xFF), Lift(v >> 8 & 0xFF), Lift(v & 0xFF));
    }

    public static bool Dark => (App.Window?.Content as FrameworkElement)?.ActualTheme == ElementTheme.Dark;
    /// The window's theme, for what's drawn outside it (sheets), which otherwise take Windows' own.
    public static ElementTheme Theme => Dark ? ElementTheme.Dark : ElementTheme.Light;

    public static Color Accent => Hex(Store.Shared.Config?.Accent ?? WhiteLabel.Accent, Dark);
    public static SolidColorBrush AccentBrush => new(Accent);

    // Fluent's text fills, as its light and dark themes have them: its own brushes keep the theme the window started
    // in when Windows changes it while it's open.
    public static Brush Primary => new SolidColorBrush(Dark ? Color.FromArgb(0xFF, 255, 255, 255) : Color.FromArgb(0xE4, 0, 0, 0));
    public static Brush Secondary => new SolidColorBrush(Dark ? Color.FromArgb(0xC5, 255, 255, 255) : Color.FromArgb(0x9E, 0, 0, 0));
    public static Brush Tertiary => new SolidColorBrush(Dark ? Color.FromArgb(0x87, 255, 255, 255) : Color.FromArgb(0x72, 0, 0, 0));
    public static Brush Card => Themed("CardBackgroundFillColorDefaultBrush");
    public static Brush CardStroke => Themed("CardStrokeColorDefaultBrush");
    public static Brush Critical => Themed("SystemFillColorCriticalBrush");

    /// A brush of the Fluent styles, as the window's theme (the person's choice, or Windows') has it.
    public static Brush Themed(string key)
    {
        foreach (var d in Application.Current.Resources.MergedDictionaries)
            foreach (var theme in Dark ? new[] { "Dark", "Default" } : ["Light"])
                if (d.ThemeDictionaries.TryGetValue(theme, out var t) && t is ResourceDictionary r && r.TryGetValue(key, out var brush)) return (Brush)brush;
        return (Brush)Application.Current.Resources[key];
    }

    /// Phosphor's icons (vendor/phosphor, beside the program), which apps name for their tiles, screens and widgets, so
    /// every device draws the same ones: each a path on a 256-point grid, drawn filled. `git-branch` is the regular one,
    /// `git-branch-fill` filled.
    public static Dictionary<string, string> Icons => WidgetArt.Icons;

    /// A named icon, `size` across, filled with `brush`; nothing if there's none by that name.
    public static FrameworkElement? Icon(string? name, double size, Brush? brush = null)
    {
        if (name == null || !Icons.TryGetValue(name, out var d) || PathData(d) is not { } geometry) return null;
        return new Viewbox
        {
            Width = size, Height = size,
            Child = new Canvas { Width = 256, Height = 256, Children = { new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, Fill = brush ?? AccentBrush } } },
        };
    }

    /// An SVG path's data as a geometry.
    public static Geometry? PathData(string path)
    {
        try { return (Geometry)Microsoft.UI.Xaml.Markup.XamlBindingHelper.ConvertValue(typeof(Geometry), path); } catch { return null; }
    }

    /// The system's colours, for apps that don't declare one: the Apple apps' and the web's.
    static readonly string[] colors = ["#007aff", "#34c759", "#ff9500", "#ff3b30", "#af52de", "#5856d6", "#30b0c7", "#ff2d55", "#a2845e", "#32ade6", "#00c7be", "#8e8e93"];

    /// A steady number for a name, the same on every device (the Apple apps' steadyHash).
    public static int SteadyHash(string text) => text.Aggregate(5381, (h, c) => (h * 33 + c) & 0x7FFF_FFFF);

    /// An app's icon tile, as the Apple apps draw one: a finished app icon edge to edge, a logo inset on white, or its
    /// symbol in white on its colour (the one it declares, else one picked by its name).
    public static FrameworkElement Tile(AppInfo? app, double size, string? fallbackName = null) =>
        Tile(app?.Icon, app?.Name ?? fallbackName ?? "", app?.Color, app?.Image, app?.ImageKind, size);

    public static FrameworkElement Tile(string? icon, string seed, string? color, string? image, string? imageKind, double size)
    {
        var radius = new CornerRadius(size * 0.225);
        var tile = new Border { Width = size, Height = size, CornerRadius = radius };
        if (Picture(image) is { } picture)
        {
            var pic = new Image { Source = picture, Stretch = imageKind == "icon" ? Stretch.UniformToFill : Stretch.Uniform };
            if (imageKind == "icon") tile.Child = pic;
            else
            {
                tile.Background = new SolidColorBrush(Colors.White);
                tile.BorderBrush = new SolidColorBrush(Color.FromArgb(26, 0, 0, 0));
                tile.BorderThickness = new Thickness(0.5);
                pic.Margin = new Thickness(size * 0.14);
                tile.Child = pic;
            }
            return tile;
        }
        var c = TileColor(color, seed);
        Color Mix(Color a, double t) => Color.FromArgb(255, (byte)(a.R + (255 - a.R) * t), (byte)(a.G + (255 - a.G) * t), (byte)(a.B + (255 - a.B) * t));
        tile.Background = new LinearGradientBrush
        {
            StartPoint = new(0, 0), EndPoint = new(0, 1),
            GradientStops = { new GradientStop { Color = Mix(c, 0.12), Offset = 0 }, new GradientStop { Color = c, Offset = 1 } },
        };
        // Its icon filled, else its first letter, in white.
        tile.Child = Icon(icon == null ? null : $"{icon}-fill", size * 0.5, new SolidColorBrush(Colors.White)) ?? new TextBlock
        {
            Text = seed.Length > 0 ? seed[..1].ToUpperInvariant() : "?", FontSize = size * 0.46, FontWeight = FontWeights.SemiBold,
            Foreground = new SolidColorBrush(Colors.White), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
        };
        return tile;
    }

    /// An app's tile colour: the one it declares, else one picked by its name, so apps side by side differ.
    public static Color TileColor(string? color, string seed) => Hex(color ?? colors[SteadyHash(seed) % colors.Length]);

    static readonly Dictionary<string, BitmapImage?> pictures = [];

    /// A picture from the data URL a node gives.
    static BitmapImage? Picture(string? url)
    {
        if (string.IsNullOrEmpty(url)) return null;
        if (pictures.TryGetValue(url, out var kept)) return kept;
        BitmapImage? image = null;
        try
        {
            var comma = url.IndexOf(',');
            if (url.StartsWith("data:") && comma > 0)
            {
                var bytes = Convert.FromBase64String(url[(comma + 1)..]);
                image = new BitmapImage();
                var stream = new MemoryStream(bytes).AsRandomAccessStream();
                _ = image.SetSourceAsync(stream);
            }
            else if (Uri.TryCreate(url, UriKind.Absolute, out var uri) && uri.Scheme == "https") image = new BitmapImage(uri);
        }
        catch { image = null; }
        pictures[url] = image;
        return image;
    }

    /// A page's Services folder: its services' icons, nine at most, small, on frosted glass; with none, faint places.
    public static FrameworkElement Folder(IEnumerable<AppInfo> apps, double size)
    {
        var grid = new Grid { Width = size, Height = size, Padding = new Thickness(size * 0.12), CornerRadius = new CornerRadius(size * 0.225), Background = Card, BorderBrush = CardStroke, BorderThickness = new Thickness(0.5) };
        for (var i = 0; i < 3; i++)
        {
            grid.RowDefinitions.Add(new RowDefinition());
            grid.ColumnDefinitions.Add(new ColumnDefinition());
        }
        var list = apps.Take(9).ToList();
        var cell = size * 0.76 / 3 - size * 0.04;
        for (var i = 0; i < 9; i++)
        {
            FrameworkElement el = i < list.Count
                ? Tile(list[i], cell)
                : new Border { Width = cell, Height = cell, CornerRadius = new CornerRadius(cell * 0.225), Background = new SolidColorBrush(Color.FromArgb(list.Count == 0 ? (byte)30 : (byte)0, 128, 128, 128)) };
            el.HorizontalAlignment = HorizontalAlignment.Center;
            el.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetRow(el, i / 3);
            Grid.SetColumn(el, i % 3);
            grid.Children.Add(el);
        }
        return grid;
    }

    static readonly uint[] people = [0x5e5ce6, 0xff9f0a, 0x30b0c7, 0xff375f, 0x34c759, 0xaf52de, 0x0a84ff, 0xa2845e];

    /// A person: their initials on a colour picked by their id, the same one app screens draw (`one.avatar`).
    public static FrameworkElement Avatar(string id, string name, double size)
    {
        uint h = 0;
        foreach (var c in id) h = unchecked(h * 31 + c);
        var rgb = people[h % people.Length];
        var initials = string.Concat(name.Split(' ', StringSplitOptions.RemoveEmptyEntries).Select(w => w[0]).Take(2)).ToUpperInvariant();
        return new Border
        {
            Width = size, Height = size, CornerRadius = new CornerRadius(size / 2),
            Background = new SolidColorBrush(Color.FromArgb(255, (byte)(rgb >> 16), (byte)(rgb >> 8 & 0xFF), (byte)(rgb & 0xFF))),
            Child = new TextBlock
            {
                Text = initials.Length == 0 ? "?" : initials, FontSize = size * 0.4, FontWeight = FontWeights.SemiBold, Foreground = new SolidColorBrush(Colors.White),
                HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            },
        };
    }

    /// A Settings item's icon: yours is your picture; the rest, its symbol on its colour, as an app's tile.
    public static FrameworkElement Setting(SettingsItem item, double size) =>
        item.Name == "account" && Store.Shared.Me is { } me ? Avatar(me.Id, me.Name, size) : Tile(item.Icon, item.Name, item.Color, null, null, size);

    public static TextBlock Text(string text, double size = 14, Brush? brush = null, bool semibold = false, int lines = 0, bool wrap = false)
    {
        var t = new TextBlock { Text = text, FontSize = size, TextTrimming = TextTrimming.CharacterEllipsis, TextWrapping = wrap || lines > 1 ? TextWrapping.WrapWholeWords : TextWrapping.NoWrap };
        if (brush != null) t.Foreground = brush;
        if (semibold) t.FontWeight = FontWeights.SemiBold;
        if (lines > 0) t.MaxLines = lines;
        return t;
    }

    /// A message filling the place it's in: a title and what to do.
    public static FrameworkElement Unavailable(string title, string text, FrameworkElement? extra = null)
    {
        var panel = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, MaxWidth = 440 };
        panel.Children.Add(new TextBlock { Text = title, FontSize = 20, FontWeight = FontWeights.SemiBold, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center });
        panel.Children.Add(new TextBlock { Text = text, FontSize = 14, Foreground = Secondary, TextWrapping = TextWrapping.WrapWholeWords, TextAlignment = TextAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center });
        if (extra != null)
        {
            extra.HorizontalAlignment = HorizontalAlignment.Center;
            extra.Margin = new Thickness(0, 8, 0, 0);
            panel.Children.Add(extra);
        }
        return panel;
    }

    public static Button Accented(string text, Action click)
    {
        var b = new Button { Content = text, Style = (Microsoft.UI.Xaml.Style)Application.Current.Resources["AccentButtonStyle"] };
        b.Click += (_, _) => click();
        return b;
    }

    public static HyperlinkButton Link(string text, Action click)
    {
        var b = new HyperlinkButton { Content = text, Padding = new Thickness(4, 2, 4, 2) };
        b.Click += (_, _) => click();
        return b;
    }
}

/// Light, Dark or Automatic, as chosen on this computer.
public static class Appearance
{
    static string File => Path.Combine(WhiteLabel.Folder, "appearance");

    public static string Chosen
    {
        get
        {
            try { return System.IO.File.ReadAllText(File).Trim() is "light" or "dark" ? System.IO.File.ReadAllText(File).Trim() : "automatic"; }
            catch { return "automatic"; }
        }
    }

    public static ElementTheme Theme => Chosen switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };

    public static void Choose(string a, MainWindow window)
    {
        Directory.CreateDirectory(WhiteLabel.Folder);
        System.IO.File.WriteAllText(File, a);
        window.Restyle();
    }
}
