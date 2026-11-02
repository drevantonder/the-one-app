// Draws a widget's tree natively, as the Apple apps do (apple/Shared/Views/WidgetTree.swift) and the web draws it as
// HTML (src/screen/widgets.js). Fields of the wrong shape are left out, and nodes it doesn't know draw nothing.
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;

namespace One;

/// What a widget's buttons, toggles and links act through: its app's operations → the error's text if one failed,
/// and following a link (a web address, or `{ screen, params }`).
public sealed record WidgetHost(Func<string, JsonNode?, Task<string?>> Run, Action<JsonElement> Follow);

public static class WidgetTree
{
    static string? Str(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
    static double? Num(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.Number ? v.GetDouble() : null;
    static bool? Bool(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind is JsonValueKind.True or JsonValueKind.False ? v.GetBoolean() : null;
    static bool Has(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind != JsonValueKind.Null && v.ValueKind != JsonValueKind.Undefined;

    static IEnumerable<JsonElement> Children(JsonElement n) =>
        n.TryGetProperty("c", out var c) && c.ValueKind == JsonValueKind.Array ? c.EnumerateArray().Where(x => x.ValueKind == JsonValueKind.Object && Str(x, "t") != null) : [];

    /// A value as a stat shows it: numbers without a needless ".0".
    static string Value(JsonElement n, string key = "value") => n.TryGetProperty(key, out var v) ? v.ValueKind switch
    {
        JsonValueKind.Number => v.GetDouble().ToString("#,0.##", CultureInfo.CurrentCulture),
        JsonValueKind.String => v.GetString() ?? "",
        JsonValueKind.True => "Yes",
        JsonValueKind.False => "No",
        _ => "",
    } : "";

    static double Number(JsonElement n, string key = "value") => n.TryGetProperty(key, out var v)
        ? v.ValueKind == JsonValueKind.Number ? v.GetDouble() : double.TryParse(v.ValueKind == JsonValueKind.String ? v.GetString() : "", out var d) ? d : 0
        : 0;

    static double Fraction(JsonElement n)
    {
        var max = Num(n, "max") ?? 1;
        return max > 0 ? Math.Clamp(Number(n) / max, 0, 1) : 0;
    }

    /// Text sizes, as the web's widgets draw them on a computer.
    static double FontSize(string? size) => size switch { "xl" => 22, "lg" => 15, "sm" => 11, "xs" => 10, _ => 12.5 };

    /// A widget, as a card: its tree under its title (unless the tree labels itself); a broken one says so, and why.
    public static FrameworkElement Card(string title, JsonElement tree, string? error, WidgetHost host, string appName)
    {
        var panel = new Grid { RowSpacing = 6 };
        panel.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        panel.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        if (tree.ValueKind != JsonValueKind.Object || Bool(tree, "header") != false || error != null)
            panel.Children.Add(Look.Text(title, 11, Look.Secondary, semibold: false));
        FrameworkElement body;
        if (error != null)
        {
            var broken = new StackPanel { Spacing = 4 };
            broken.Children.Add(Look.Text("This widget stopped working.", 13, Look.Toned("warn"), semibold: true));
            broken.Children.Add(new TextBlock { Text = error, FontSize = 11, Foreground = Look.Secondary, TextWrapping = TextWrapping.Wrap, MaxLines = 3, IsTextSelectionEnabled = true });
            broken.Children.Add(Look.Text($"Right-click to copy it for {appName}’s maker.", 11, Look.Tertiary, wrap: true));
            body = broken;
        }
        else body = tree.ValueKind == JsonValueKind.Object ? Draw(tree, host, false) : new Grid();
        Grid.SetRow(body, 1);
        panel.Children.Add(body);
        return new Border
        {
            Child = panel, Padding = new Thickness(16), CornerRadius = new CornerRadius(20),
            Background = Look.Card, BorderBrush = Look.CardStroke, BorderThickness = new Thickness(1),
        };
    }

    /// A node of a tree; `inRow` keeps text to one line, as in a row.
    public static FrameworkElement Draw(JsonElement n, WidgetHost host, bool inRow)
    {
        var el = Content(n, host, inRow);
        var kind = Str(n, "t");
        if (kind != "item" && kind != "link" && n.TryGetProperty("link", out var link) && link.ValueKind != JsonValueKind.Null) return Linked(el, link, host);
        return el;
    }

    static FrameworkElement Content(JsonElement n, WidgetHost host, bool inRow)
    {
        var kind = Str(n, "t");
        switch (kind)
        {
            case "col":
                return Stack(n, host, Orientation.Vertical);
            case "row":
                return Stack(n, host, Orientation.Horizontal);
            case "text":
                {
                    var t = new TextBlock
                    {
                        Text = Str(n, "text") ?? "", FontSize = FontSize(Str(n, "size")), Foreground = Look.Toned(Str(n, "tone")),
                        TextTrimming = TextTrimming.CharacterEllipsis, TextWrapping = inRow && Num(n, "lines") is null ? TextWrapping.NoWrap : TextWrapping.WrapWholeWords,
                    };
                    if (Num(n, "lines") is { } lines) t.MaxLines = (int)lines;
                    var weight = Str(n, "weight");
                    if (weight == "bold") t.FontWeight = FontWeights.Bold;
                    else if (weight == "medium" || Str(n, "size") is "xl" or "lg") t.FontWeight = FontWeights.SemiBold;
                    return t;
                }
            case "time":
                return new TextBlock { Text = Time(n), FontSize = FontSize(Str(n, "size")), Foreground = Look.Toned(Str(n, "tone")), TextWrapping = TextWrapping.NoWrap };
            case "stat":
                {
                    var panel = new StackPanel();
                    var line = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 3 };
                    var value = Value(n);
                    line.Children.Add(new TextBlock { Text = value.Length == 0 ? "–" : value, FontSize = 26, FontWeight = FontWeights.SemiBold, Foreground = Look.Toned(Str(n, "tone")) });
                    if (Str(n, "unit") is { } unit) line.Children.Add(new TextBlock { Text = unit, FontSize = 13, FontWeight = FontWeights.Medium, Foreground = Look.Secondary, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(0, 0, 0, 4) });
                    panel.Children.Add(line);
                    if (Str(n, "label") is { } label) panel.Children.Add(Look.Text(label, 11, Look.Secondary));
                    return panel;
                }
            case "gauge":
                return Ring(n);
            case "progress":
                {
                    var panel = new StackPanel { Spacing = 4 };
                    if (Num(n, "w") is { } w) panel.Width = w;
                    if (Str(n, "label") is { } label) panel.Children.Add(Look.Text(label, 11, Look.Secondary));
                    var track = new Grid { Height = 6, CornerRadius = new CornerRadius(3), Background = new SolidColorBrush(Windows.UI.Color.FromArgb(56, 128, 128, 128)) };
                    var fill = new Border { Background = Look.Toned(Str(n, "tone"), Look.AccentBrush), HorizontalAlignment = HorizontalAlignment.Left, CornerRadius = new CornerRadius(3) };
                    var fraction = Fraction(n);
                    track.SizeChanged += (_, e) => fill.Width = Math.Max(6, e.NewSize.Width * fraction);
                    track.Children.Add(fill);
                    panel.Children.Add(track);
                    return panel;
                }
            case "spark":
                return Spark(n);
            case "chart":
                return Chart(n);
            case "list":
                {
                    var items = Children(n).ToList();
                    if (items.Count == 0) return Look.Text(Str(n, "empty") ?? "Nothing here", 12.5, Look.Secondary);
                    var panel = new StackPanel();
                    for (var i = 0; i < items.Count; i++)
                    {
                        if (i > 0) panel.Children.Add(new Border { Height = 1, Background = Look.Divider, Margin = new Thickness(Has(items[i], "icon") ? 30 : 0, 0, 0, 0) });
                        var row = Draw(items[i], host, false);
                        row.Margin = new Thickness(0, 5, 0, 5);
                        panel.Children.Add(row);
                    }
                    return panel;
                }
            case "item":
                {
                    var grid = new Grid { ColumnSpacing = 10, Background = new SolidColorBrush(Colors.Transparent) };
                    grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                    grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                    grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                    if (Look.Icon(Str(n, "icon"), 18, Look.Toned(Str(n, "tone"), Look.AccentBrush)) is { } icon)
                    {
                        icon.Margin = new Thickness(1, 0, 1, 0);
                        icon.VerticalAlignment = VerticalAlignment.Center;
                        grid.Children.Add(icon);
                    }
                    var texts = new StackPanel { Spacing = 1, VerticalAlignment = VerticalAlignment.Center };
                    texts.Children.Add(Look.Text(Str(n, "title") ?? "", 12.5));
                    if (Str(n, "subtitle") is { } sub) texts.Children.Add(Look.Text(sub, 11, Look.Secondary));
                    Grid.SetColumn(texts, 1);
                    grid.Children.Add(texts);
                    string? end = Num(n, "at") is not null ? Time(n) : Str(n, "trailing");
                    if (end != null)
                    {
                        var t = Look.Text(end, Num(n, "at") is not null ? 11 : 12, Look.Secondary);
                        t.VerticalAlignment = VerticalAlignment.Center;
                        Grid.SetColumn(t, 2);
                        grid.Children.Add(t);
                    }
                    return n.TryGetProperty("link", out var link) && link.ValueKind != JsonValueKind.Null ? Linked(grid, link, host) : grid;
                }
            case "image":
                {
                    var w = Num(n, "w");
                    var h = Num(n, "h") ?? w ?? 120;
                    var image = new Image { Stretch = Stretch.UniformToFill };
                    if (Uri.TryCreate(Str(n, "url") ?? "", UriKind.Absolute, out var url) && url.Scheme is "https" or "http") image.Source = new BitmapImage(url);
                    var frame = new Border { Child = image, Height = h, CornerRadius = new CornerRadius(Bool(n, "round") == true ? 999 : 8), Background = Look.Card };
                    if (w is { } width) frame.Width = width;
                    return frame;
                }
            case "icon":
                return Icon(n);
            case "dot":
                return new Ellipse { Width = 8, Height = 8, Fill = Look.Toned(Str(n, "tone"), Look.Toned("good")), VerticalAlignment = VerticalAlignment.Center };
            case "link":
                {
                    var t = new TextBlock { Text = Str(n, "text") ?? "", FontSize = FontSize(Str(n, "size")), Foreground = Look.AccentBrush, TextTrimming = TextTrimming.CharacterEllipsis, MaxLines = (int)(Num(n, "lines") ?? 1) };
                    return n.TryGetProperty("link", out var link) && link.ValueKind != JsonValueKind.Null ? Linked(t, link, host) : t;
                }
            case "button":
                return ActionButton(n, host);
            case "toggle":
                return ActionToggle(n, host);
            case "divider":
                return new Border { Height = 1, Background = Look.Divider, Margin = new Thickness(0, 4, 0, 4) };
            default:
                return new Grid();
        }
    }

    /// A column or a row of nodes: in a row, a node that grows takes what's left, the rest their own room.
    static FrameworkElement Stack(JsonElement n, WidgetHost host, Orientation orientation)
    {
        var row = orientation == Orientation.Horizontal;
        var gap = Num(n, "gap") ?? (row ? 6 : 4);
        var kids = Children(n).ToList();
        FrameworkElement el;
        if (row)
        {
            var grid = new Grid { ColumnSpacing = gap };
            for (var i = 0; i < kids.Count; i++)
            {
                var grows = Bool(kids[i], "grow") == true || Str(kids[i], "t") == "text" && !kids.Any(k => Bool(k, "grow") == true) && i == kids.FindIndex(k => Str(k, "t") == "text");
                grid.ColumnDefinitions.Add(new ColumnDefinition { Width = grows ? new GridLength(1, GridUnitType.Star) : GridLength.Auto });
                var child = Draw(kids[i], host, true);
                child.VerticalAlignment = VerticalAlignment.Center;
                Grid.SetColumn(child, i);
                grid.Children.Add(child);
            }
            el = grid;
        }
        else
        {
            var grid = new Grid { RowSpacing = gap };
            for (var i = 0; i < kids.Count; i++)
            {
                grid.RowDefinitions.Add(new RowDefinition { Height = Bool(kids[i], "grow") == true ? new GridLength(1, GridUnitType.Star) : GridLength.Auto });
                var child = Draw(kids[i], host, false);
                if (Str(n, "align") == "center") child.HorizontalAlignment = HorizontalAlignment.Center;
                Grid.SetRow(child, i);
                grid.Children.Add(child);
            }
            el = grid;
        }
        if (Num(n, "w") is { } w) el.Width = w;
        if (Bool(n, "dim") == true) el.Opacity = 0.5;
        return el;
    }

    /// "now", "5 min", "3 h", then the weekday, then the date; or as `style` says.
    static string Time(JsonElement n)
    {
        var at = DateTimeOffset.FromUnixTimeMilliseconds((long)(Num(n, "at") ?? 0)).ToLocalTime();
        var s = (DateTimeOffset.Now - at).TotalSeconds;
        switch (Str(n, "style"))
        {
            case "date": return at.ToString("d MMM", CultureInfo.CurrentCulture);
            case "clock": return at.ToString("t", CultureInfo.CurrentCulture);
            case "timer":
                var a = (long)Math.Abs(Math.Round(s));
                return $"{a / 60}:{a % 60:00}";
            case "relative":
                var ahead = s < 0;
                var x = Math.Abs(s);
                var said = x < 60 ? "now" : x < 3600 ? $"{(int)(x / 60)} min." : x < 86400 ? $"{(int)(x / 3600)} hr." : $"{(int)(x / 86400)} days";
                return said == "now" ? said : ahead ? $"in {said}" : $"{said} ago";
        }
        if (s < 60) return "now";
        if (s < 3600) return $"{(int)(s / 60)} min";
        if (s < 86400) return $"{(int)(s / 3600)} h";
        return s < 7 * 86400 ? at.ToString("ddd", CultureInfo.CurrentCulture) : at.ToString("d MMM", CultureInfo.CurrentCulture);
    }

    /// A ring filled `value / max` of the way round, with its label in it.
    static FrameworkElement Ring(JsonElement n)
    {
        var size = Num(n, "w") ?? 46;
        var line = Math.Min(5, size / 6);
        var fraction = Fraction(n);
        var grid = new Grid { Width = size, Height = size };
        grid.Children.Add(new Ellipse { Stroke = new SolidColorBrush(Windows.UI.Color.FromArgb(64, 128, 128, 128)), StrokeThickness = line, Margin = new Thickness(line / 2) });
        if (fraction > 0)
        {
            var r = (size - line * 2) / 2;
            var c = size / 2;
            var angle = Math.Min(fraction, 0.9999) * 2 * Math.PI;
            var end = new Point(c + r * Math.Sin(angle), c - r * Math.Cos(angle));
            var figure = new PathFigure { StartPoint = new Point(c, c - r), IsClosed = false };
            figure.Segments.Add(new ArcSegment { Point = end, Size = new Size(r, r), IsLargeArc = angle > Math.PI, SweepDirection = SweepDirection.Clockwise });
            var geometry = new PathGeometry();
            geometry.Figures.Add(figure);
            grid.Children.Add(new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, Stroke = Look.Toned(Str(n, "tone"), Look.AccentBrush), StrokeThickness = line, StrokeStartLineCap = PenLineCap.Round, StrokeEndLineCap = PenLineCap.Round });
        }
        var label = Str(n, "label");
        if (label != "")
            grid.Children.Add(new TextBlock { Text = label ?? $"{Math.Round(fraction * 100)}%", FontSize = Math.Max(9, size / 4.6), FontWeight = FontWeights.SemiBold, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center });
        return grid;
    }

    static List<double> Values(JsonElement n) => n.TryGetProperty("values", out var v) && v.ValueKind == JsonValueKind.Array
        ? v.EnumerateArray().Where(x => x.ValueKind == JsonValueKind.Number).Select(x => x.GetDouble()).ToList()
        : [];

    /// A line through numbers from 0 to `max`, shaded under; a dashed line when there are none.
    static FrameworkElement Spark(JsonElement n)
    {
        var values = Values(n);
        var canvas = new Grid { Height = Num(n, "h") ?? 28 };
        if (Num(n, "w") is { } w) canvas.Width = w;
        var brush = Look.Toned(Str(n, "tone"), Look.AccentBrush);
        canvas.SizeChanged += (_, e) =>
        {
            canvas.Children.Clear();
            double width = e.NewSize.Width, height = e.NewSize.Height;
            if (values.Count == 0)
            {
                canvas.Children.Add(new Line { X1 = 4, Y1 = height / 2, X2 = width - 4, Y2 = height / 2, Stroke = Look.Tertiary, StrokeThickness = 1.5, StrokeDashArray = new DoubleCollection { 2, 3 } });
                return;
            }
            var data = values.Count == 1 ? new List<double> { values[0], values[0] } : values;
            var top = Num(n, "max") ?? Math.Max(1, data.Max());
            var points = data.Select((v, i) => new Point(i / (double)(data.Count - 1) * width, height - 1 - Math.Clamp(v, 0, top) / top * (height - 2))).ToList();
            var area = new Polygon { Fill = brush, Opacity = 0.2 };
            foreach (var p in points) area.Points.Add(p);
            area.Points.Add(new Point(width, height));
            area.Points.Add(new Point(0, height));
            var stroke = new Polyline { Stroke = brush, StrokeThickness = 1.4, StrokeLineJoin = PenLineJoin.Round };
            foreach (var p in points) stroke.Points.Add(p);
            canvas.Children.Add(area);
            canvas.Children.Add(stroke);
        };
        return canvas;
    }

    /// Bars, or a line, with each value's label under it.
    static FrameworkElement Chart(JsonElement n)
    {
        var values = Values(n);
        var labels = n.TryGetProperty("labels", out var l) && l.ValueKind == JsonValueKind.Array ? l.EnumerateArray().Select(x => x.ValueKind == JsonValueKind.String ? x.GetString() ?? "" : "").ToList() : [];
        var height = Num(n, "h") ?? 80;
        var brush = Look.Toned(Str(n, "tone"), Look.AccentBrush);
        var grid = new Grid { Height = height, ColumnSpacing = 4 };
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var top = Math.Max(values.DefaultIfEmpty(0).Max(), 1e-9);
        if (Str(n, "kind") == "line")
        {
            var area = new Grid();
            area.SizeChanged += (_, e) =>
            {
                area.Children.Clear();
                if (values.Count == 0) return;
                var step = e.NewSize.Width / values.Count;
                var line = new Polyline { Stroke = brush, StrokeThickness = 2, StrokeLineJoin = PenLineJoin.Round };
                for (var i = 0; i < values.Count; i++) line.Points.Add(new Point(step * (i + 0.5), e.NewSize.Height - values[i] / top * (e.NewSize.Height - 2)));
                area.Children.Add(line);
            };
            Grid.SetColumnSpan(area, Math.Max(1, values.Count));
            grid.Children.Add(area);
        }
        for (var i = 0; i < values.Count; i++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition());
            if (Str(n, "kind") != "line")
            {
                var bar = new Border { Background = brush, CornerRadius = new CornerRadius(3, 3, 0, 0), VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(2, 0, 2, 0) };
                var v = values[i];
                grid.SizeChanged += (_, e) => bar.Height = Math.Max(1, v / top * Math.Max(0, e.NewSize.Height - 16));
                Grid.SetColumn(bar, i);
                grid.Children.Add(bar);
            }
            var label = new TextBlock { Text = i < labels.Count ? labels[i] : "", FontSize = 10, Foreground = Look.Secondary, HorizontalAlignment = HorizontalAlignment.Center };
            Grid.SetRow(label, 1);
            Grid.SetColumn(label, i);
            grid.Children.Add(label);
        }
        return grid;
    }

    /// An icon, `w` across or as big as its text: named, or an SVG path outlined; with its badge (an SVG path) at its
    /// bottom trailing corner.
    static FrameworkElement Icon(JsonElement n)
    {
        var path = Str(n, "path");
        var brush = Look.Toned(Str(n, "tone"), path == null ? Look.AccentBrush : null);
        var w = Num(n, "w") ?? (path == null ? FontSize(Str(n, "size")) * 1.3 : 22);
        var grid = new Grid { Width = w, Height = w };
        if (path != null && PathData(path) is { } geometry)
            grid.Children.Add(new Viewbox { Child = new Canvas { Width = 24, Height = 24, Children = { new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, Stroke = brush, StrokeThickness = 1.5, StrokeLineJoin = PenLineJoin.Round } } } });
        else if (Look.Icon(Str(n, "name"), w, brush) is { } named) grid.Children.Add(named);
        if (Str(n, "badge") is { } badge && PathData(badge) is { } mark)
        {
            var size = w * 11 / 24;
            grid.Children.Add(new Viewbox
            {
                Width = size, Height = size, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom,
                Child = new Canvas { Width = 24, Height = 24, Children = { new Microsoft.UI.Xaml.Shapes.Path { Data = mark, Fill = brush } } },
            });
        }
        return grid;
    }

    static Geometry? PathData(string path) => Look.PathData(path);

    /// A node that opens its link when clicked.
    static FrameworkElement Linked(FrameworkElement content, JsonElement link, WidgetHost host)
    {
        var button = new Button
        {
            Content = content, Padding = new Thickness(0), Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
        };
        var kept = link.Clone();
        button.Click += (_, _) => host.Follow(kept);
        return button;
    }

    static async Task Failed(FrameworkElement at, string error)
    {
        if (at.XamlRoot == null) return;
        await new ContentDialog { Title = "Couldn’t Do That", Content = error, CloseButtonText = "OK", XamlRoot = at.XamlRoot }.ShowAsync();
    }

    static JsonNode? Input(JsonElement n) => n.TryGetProperty("input", out var i) && i.ValueKind != JsonValueKind.Null ? JsonNode.Parse(i.GetRawText()) : new JsonObject();

    static FrameworkElement ActionButton(JsonElement n, WidgetHost host)
    {
        var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        if (Look.Icon(Str(n, "icon"), 14, Look.Toned(Str(n, "tone"), Look.AccentBrush)) is { } icon) label.Children.Add(icon);
        label.Children.Add(new TextBlock { Text = Str(n, "label") ?? Str(n, "text") ?? "", FontSize = 12 });
        var button = new Button { Content = label, Padding = new Thickness(11, 3, 11, 4), CornerRadius = new CornerRadius(999), Foreground = Look.Toned(Str(n, "tone"), Look.AccentBrush) };
        var op = Str(n, "op");
        button.IsEnabled = op != null;
        var input = Input(n);
        button.Click += async (_, _) =>
        {
            button.IsEnabled = false;
            var error = await host.Run(op!, input?.DeepClone());
            button.IsEnabled = true;
            if (error != null) await Failed(button, error);
        };
        return button;
    }

    static FrameworkElement ActionToggle(JsonElement n, WidgetHost host)
    {
        var toggle = new ToggleSwitch { IsOn = Bool(n, "on") == true, OnContent = "", OffContent = "", MinWidth = 0 };
        var grid = new Grid { ColumnSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var label = Look.Text(Str(n, "label") ?? "", 12.5);
        label.VerticalAlignment = VerticalAlignment.Center;
        grid.Children.Add(label);
        Grid.SetColumn(toggle, 1);
        grid.Children.Add(toggle);
        var op = Str(n, "op");
        toggle.IsEnabled = op != null;
        var input = Input(n) as JsonObject ?? new JsonObject();
        toggle.Toggled += async (_, _) =>
        {
            var given = (JsonObject)input.DeepClone();
            given["on"] = toggle.IsOn;
            var error = await host.Run(op!, given);
            if (error != null) await Failed(toggle, error);
        };
        return grid;
    }
}
