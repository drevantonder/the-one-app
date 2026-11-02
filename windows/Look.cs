// The look: the install's accent, app icons as tiles, people's initials, and the icons apps name (Phosphor's).
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
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

    public static Color Accent => Hex(Store.Shared.Config?.Accent ?? Brand.Accent, Dark);
    public static SolidColorBrush AccentBrush => new(Accent);

    public static Brush Secondary => Themed("TextFillColorSecondaryBrush");
    public static Brush Tertiary => Themed("TextFillColorTertiaryBrush");
    public static Brush Card => Themed("CardBackgroundFillColorDefaultBrush");
    public static Brush CardStroke => Themed("CardStrokeColorDefaultBrush");
    public static Brush Divider => Themed("DividerStrokeColorDefaultBrush");

    /// A brush of the Fluent styles, as the window's theme (the person's choice, or Windows') has it.
    public static Brush Themed(string key)
    {
        foreach (var d in Application.Current.Resources.MergedDictionaries)
            foreach (var theme in Dark ? new[] { "Dark", "Default" } : ["Light"])
                if (d.ThemeDictionaries.TryGetValue(theme, out var t) && t is ResourceDictionary r && r.TryGetValue(key, out var brush)) return (Brush)brush;
        return (Brush)Application.Current.Resources[key];
    }

    /// What a tone colours: `muted` is secondary text; none, `otherwise`.
    public static Brush Toned(string? tone, Brush? otherwise = null) => tone switch
    {
        "muted" => Secondary,
        "accent" => AccentBrush,
        "good" => new SolidColorBrush(Hex("#34c759")),
        "warn" => new SolidColorBrush(Hex("#ff9f0a")),
        "bad" => new SolidColorBrush(Hex("#ff3b30")),
        "blue" => new SolidColorBrush(Hex("#0a9edc")),
        "purple" => new SolidColorBrush(Hex("#a550d9")),
        _ => otherwise ?? Themed("TextFillColorPrimaryBrush"),
    };

    /// Windows' own symbols, for the window's own controls.
    public static readonly FontFamily Symbols = new("Segoe Fluent Icons,Segoe MDL2 Assets");

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

    /// A red count, as an icon's badge.
    public static FrameworkElement Count(int n, bool red = true) => new Border
    {
        MinWidth = 20, Height = 20, Padding = new Thickness(6, 0, 6, 0), CornerRadius = new CornerRadius(10),
        Background = red ? new SolidColorBrush(Hex("#ff3b30")) : Themed("ControlFillColorTertiaryBrush"),
        Child = new TextBlock
        {
            Text = n > 99 ? "99+" : n.ToString(), FontSize = 12, FontWeight = FontWeights.SemiBold,
            Foreground = red ? new SolidColorBrush(Colors.White) : Secondary, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
        },
    };

    public static TextBlock Text(string text, double size = 14, Brush? brush = null, bool semibold = false, int lines = 0, bool wrap = false)
    {
        var t = new TextBlock { Text = text, FontSize = size, TextTrimming = TextTrimming.CharacterEllipsis, TextWrapping = wrap || lines > 1 ? TextWrapping.WrapWholeWords : TextWrapping.NoWrap };
        if (brush != null) t.Foreground = brush;
        if (semibold) t.FontWeight = FontWeights.SemiBold;
        if (lines > 0) t.MaxLines = lines;
        return t;
    }

    /// A page's large title and its line under it, as the Mac app's pages start.
    public static FrameworkElement Header(string title, string subtitle)
    {
        var panel = new StackPanel { Spacing = 2, Margin = new Thickness(0, 0, 0, 20) };
        panel.Children.Add(Text(title, 28, semibold: true));
        if (subtitle.Length > 0) panel.Children.Add(Text(subtitle, 13, Secondary));
        return panel;
    }

    /// A grouped list, as a list's inset section: rows on a card, with hairlines between.
    public static StackPanel Section(string? heading = null, string? footer = null, params FrameworkElement[] rows)
    {
        var outer = new StackPanel { Spacing = 6 };
        if (heading != null) outer.Children.Add(new TextBlock { Text = heading, FontSize = 13, FontWeight = FontWeights.SemiBold, Margin = new Thickness(4, 0, 0, 2) });
        var card = new StackPanel { Background = Card, BorderBrush = CardStroke, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) };
        for (var i = 0; i < rows.Length; i++)
        {
            if (i > 0) card.Children.Add(new Border { Height = 1, Background = Divider, Margin = new Thickness(16, 0, 0, 0) });
            card.Children.Add(rows[i]);
        }
        outer.Children.Add(card);
        if (footer != null) outer.Children.Add(Text(footer, 12, Secondary, wrap: true));
        return outer;
    }

    /// A row of a grouped list: an icon, a title and line under it, something at its end; with `click`, a button.
    public static FrameworkElement Row(FrameworkElement? icon, string title, string? sub = null, FrameworkElement? end = null, Action? click = null, bool chevron = false, Brush? titleBrush = null)
    {
        var grid = new Grid { ColumnSpacing = 12, Padding = new Thickness(16, 10, 14, 10), MinHeight = 48 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        if (icon != null)
        {
            icon.VerticalAlignment = VerticalAlignment.Center;
            grid.Children.Add(icon);
        }
        var texts = new StackPanel { VerticalAlignment = VerticalAlignment.Center, Spacing = 1 };
        texts.Children.Add(Text(title, 14, titleBrush));
        if (!string.IsNullOrEmpty(sub)) texts.Children.Add(Text(sub, 12, Secondary, lines: 3));
        Grid.SetColumn(texts, 1);
        grid.Children.Add(texts);
        var trailing = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, VerticalAlignment = VerticalAlignment.Center };
        if (end != null) trailing.Children.Add(end);
        if (chevron) trailing.Children.Add(new FontIcon { Glyph = "", FontFamily = Symbols, FontSize = 12, Foreground = Tertiary });
        Grid.SetColumn(trailing, 2);
        grid.Children.Add(trailing);
        if (click == null) return grid;
        var button = new Button
        {
            Content = grid, Padding = new Thickness(0), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), CornerRadius = new CornerRadius(0),
        };
        button.Click += (_, _) => click();
        return button;
    }

    /// A column as wide as the window allows, up to a readable width, scrolling down: Settings' and Services' pages.
    public static ScrollViewer Column(StackPanel panel)
    {
        panel.Padding = new Thickness(32, 8, 32, 48);
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star), MaxWidth = 760 });
        grid.Children.Add(panel);
        return new ScrollViewer { Content = grid, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled };
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

    public static Button Plain(string text, Action click)
    {
        var b = new Button { Content = text };
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
