// A widget's tree as an Adaptive Card, for the Widgets board (Widgets.cs), drawing each node as src/screen/widgets.js draws it
// on a page: text as the card's own text, in the board's font and colours; rings, sparks, bars, charts and icons
// as pictures (WidgetArt.cs), since a card draws nothing else. Nodes it doesn't know draw nothing.
//
// The board fills a card in from data, so every text goes in the data and the card names it (`${s3}`): nothing a
// widget says is read as the card's own `${…}`. A picture coloured as text is drawn twice, one for each theme.
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace One;

public sealed class WidgetCard
{
    /// The room inside a card, in the board's pixels (every size is as wide): what pictures as wide as their place are
    /// drawn at, so a stretched one keeps its height.
    const double Width = 208;

    readonly JsonObject data = new();
    readonly string accent;
    readonly string app;
    readonly string? scope;

    WidgetCard(string accent, string app, string? scope)
    {
        this.accent = accent;
        this.app = app;
        this.scope = scope;
    }

    /// The card for one widget at `size`, and its data.
    public static (string template, string data) Of(WidgetCache.Item w, string size, string accent, string? problem)
    {
        var card = new WidgetCard(accent, w.App, w.Scope);
        var tree = w.Tree(size);
        var body = new JsonArray();
        if (problem != null) body.Add(card.Note(problem, "attention"));
        if (w.Error != null)
        {
            body.Add(card.Text(w.Title, "small", subtle: true));
            body.Add(card.Text("This widget stopped working.", "default", color: "warning", weight: "bolder"));
            body.Add(card.Text($"Open {WhiteLabel.Name} to see why.", "small", subtle: true, wrap: true));
        }
        else
        {
            // Small is a short strip on the board, below the app's name: the widget's own title would leave its content
            // no room.
            if (size != "small" && (tree.ValueKind != JsonValueKind.Object || Bool(tree, "header") != false)) body.Add(card.Text(w.Title, "small", subtle: true));
            if (tree.ValueKind == JsonValueKind.Object && card.Node(tree, new Place(Width, false, false)) is { } root)
            {
                root["height"] = "stretch";
                body.Add(root);
            }
        }
        var link = tree.ValueKind == JsonValueKind.Object && tree.TryGetProperty("link", out var l) && l.ValueKind != JsonValueKind.Null ? l : default;
        return (Card(body, card.Open(link) ?? (w.Opens ? card.OpenScreen("main", null) : card.OpenPage())).ToJsonString(), card.data.ToJsonString());
    }

    /// A card that only says something, quietly (the board names the app above it), and opens the app when it's tapped.
    public static (string template, string data) Saying(string text)
    {
        var card = new WidgetCard("", "", null);
        var body = new JsonArray { card.Text(text, "small", subtle: true, wrap: true) };
        return (Card(body, new JsonObject { ["type"] = "Action.OpenUrl", ["url"] = $"{WhiteLabel.Cli}://open" }).ToJsonString(), card.data.ToJsonString());
    }

    /// Choosing which widget it shows: every widget on the person's pages, by page.
    public static (string template, string data) Chooser(IReadOnlyList<WidgetCache.Item> items, string? chosen)
    {
        var card = new WidgetCard("", "", null);
        var body = new JsonArray { card.Text("Choose a Widget", "default", weight: "bolder") };
        if (items.Count == 0)
        {
            body.Add(card.Text($"Open {WhiteLabel.Name} to see your pages, and their widgets show up here.", "small", subtle: true, wrap: true));
            return (Card(body, null).ToJsonString(), card.data.ToJsonString());
        }
        body.Add(card.Text("Pick which of your pages’ widgets this shows.", "small", subtle: true, wrap: true));
        var choices = new JsonArray();
        foreach (var item in items)
            choices.Add(new JsonObject { ["title"] = card.Bind(item.Page is { Length: > 0 } page ? $"{item.Title} · {page}" : item.Title), ["value"] = item.Id });
        body.Add(new JsonObject
        {
            ["type"] = "Input.ChoiceSet", ["id"] = "widget", ["style"] = "compact", ["choices"] = choices,
            ["value"] = items.Any(i => i.Id == chosen) ? chosen : items[0].Id,
        });
        var root = Card(body, null);
        root["actions"] = new JsonArray
        {
            new JsonObject { ["type"] = "Action.Execute", ["title"] = "Done", ["verb"] = "choose", ["style"] = "positive" },
            new JsonObject { ["type"] = "Action.Execute", ["title"] = "Cancel", ["verb"] = "cancel" },
        };
        return (root.ToJsonString(), card.data.ToJsonString());
    }

    static JsonObject Card(JsonArray body, JsonObject? tap)
    {
        var card = new JsonObject
        {
            ["type"] = "AdaptiveCard", ["$schema"] = "http://adaptivecards.io/schemas/adaptive-card.json", ["version"] = "1.5", ["body"] = body,
        };
        if (tap != null) card["selectAction"] = tap;
        return card;
    }

    // MARK: Reading a tree

    static string? Str(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
    static double? Num(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.Number ? v.GetDouble() : null;
    static bool? Bool(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind is JsonValueKind.True or JsonValueKind.False ? v.GetBoolean() : null;
    static bool Has(JsonElement n, string key) => n.TryGetProperty(key, out var v) && v.ValueKind != JsonValueKind.Null && v.ValueKind != JsonValueKind.Undefined;

    static List<JsonElement> Children(JsonElement n) =>
        n.TryGetProperty("c", out var c) && c.ValueKind == JsonValueKind.Array ? c.EnumerateArray().Where(x => x.ValueKind == JsonValueKind.Object && Str(x, "t") != null).ToList() : [];

    static string Value(JsonElement n) => n.TryGetProperty("value", out var v) ? v.ValueKind switch
    {
        JsonValueKind.Number => v.GetDouble().ToString("#,0.##", CultureInfo.CurrentCulture),
        JsonValueKind.String => v.GetString() ?? "",
        JsonValueKind.True => "Yes",
        JsonValueKind.False => "No",
        _ => "",
    } : "";

    static double Number(JsonElement n) => n.TryGetProperty("value", out var v)
        ? v.ValueKind == JsonValueKind.Number ? v.GetDouble() : double.TryParse(v.ValueKind == JsonValueKind.String ? v.GetString() : "", out var d) ? d : 0
        : 0;

    static double Fraction(JsonElement n)
    {
        var max = Num(n, "max") ?? 1;
        return max > 0 ? Math.Clamp(Number(n) / max, 0, 1) : 0;
    }

    static List<double> Values(JsonElement n) => n.TryGetProperty("values", out var v) && v.ValueKind == JsonValueKind.Array
        ? v.EnumerateArray().Where(x => x.ValueKind == JsonValueKind.Number).Select(x => x.GetDouble()).ToList()
        : [];

    /// Where a node is drawn: how wide its place is, whether it's in a row (text on one line), and dimmed.
    readonly record struct Place(double Width, bool InRow, bool Dim, bool Center = false);

    // MARK: Card elements

    /// A text, bound by name in the card's data.
    string Bind(string text)
    {
        var key = $"s{data.Count}";
        data[key] = text;
        return "${" + key + "}";
    }

    JsonObject Text(string text, string size, bool subtle = false, string? color = null, string? weight = null, bool wrap = false, int? lines = null, bool center = false)
    {
        var t = new JsonObject { ["type"] = "TextBlock", ["text"] = Bind(text), ["size"] = size, ["wrap"] = wrap };
        if (subtle) t["isSubtle"] = true;
        if (color != null) t["color"] = color;
        if (weight != null) t["weight"] = weight;
        if (lines is { } n) t["maxLines"] = n;
        if (center) t["horizontalAlignment"] = "center";
        return t;
    }

    JsonObject Note(string text, string color) => Text(text, "small", color: color, wrap: true, lines: 2);

    /// Text sizes, as the window's are to the board's.
    static string Size(string? size) => size switch { "xl" => "large", "lg" => "medium", "sm" or "xs" => "small", _ => "default" };

    /// A tone as the card's text colours it: → its colour, and whether it's muted.
    static (string? color, bool subtle) Tone(string? tone) => tone switch
    {
        "muted" => (null, true),
        "good" => ("good", false),
        "warn" => ("warning", false),
        "bad" => ("attention", false),
        "accent" or "blue" or "purple" => ("accent", false),
        _ => (null, false),
    };

    /// A picture: its data, drawn once, or once for each theme if it's coloured as text.
    JsonNode Picture(Func<string, string> draw, bool themed, double? w, double? h, bool stretch = false, bool center = false)
    {
        JsonObject One(string fg, string? when)
        {
            var image = new JsonObject { ["type"] = "Image", ["url"] = draw(fg) };
            if (stretch) image["size"] = "stretch";
            else
            {
                if (w is { } width) image["width"] = $"{Math.Round(width)}px";
                if (h is { } height) image["height"] = $"{Math.Round(height)}px";
            }
            if (center) image["horizontalAlignment"] = "center";
            if (when != null) image["$when"] = when;
            return image;
        }
        if (!themed) return One(WidgetArt.Text(false), null);
        return new JsonObject
        {
            ["type"] = "Container",
            ["items"] = new JsonArray(One(WidgetArt.Text(false), "${$host.hostTheme != 'dark'}"), One(WidgetArt.Text(true), "${$host.hostTheme == 'dark'}")),
        };
    }

    /// The colour a tone draws a picture in; none, `otherwise` (null: the text's).
    string? Paint(string? tone, string? otherwise) => tone switch
    {
        "muted" => WidgetArt.Muted,
        "good" => "#34c759",
        "warn" => "#ff9f0a",
        "bad" => "#ff3b30",
        "blue" => "#0a9edc",
        "purple" => "#a550d9",
        "accent" => accent,
        _ => otherwise,
    };

    // MARK: Opening

    /// A link's action: a web page, or one of the app's screens, through this app's own links.
    JsonObject? Open(JsonElement link)
    {
        if (link.ValueKind == JsonValueKind.String)
            return Uri.TryCreate(link.GetString(), UriKind.Absolute, out var url) && url.Scheme is "https" or "http"
                ? new JsonObject { ["type"] = "Action.OpenUrl", ["url"] = url.AbsoluteUri } : null;
        if (link.ValueKind == JsonValueKind.Object && Str(link, "screen") is { } screen)
            return OpenScreen(screen, link.TryGetProperty("params", out var p) && p.ValueKind == JsonValueKind.Object ? p.GetRawText() : null);
        return null;
    }

    JsonObject? OpenScreen(string screen, string? parameters)
    {
        if (app.Length == 0) return new JsonObject { ["type"] = "Action.OpenUrl", ["url"] = $"{WhiteLabel.Cli}://open" };
        var q = $"app={Uri.EscapeDataString(app)}&screen={Uri.EscapeDataString(screen)}&params={Uri.EscapeDataString(parameters ?? "{}")}";
        if (scope != null) q += $"&scope={Uri.EscapeDataString(scope)}";
        return new JsonObject { ["type"] = "Action.OpenUrl", ["url"] = $"{WhiteLabel.Cli}://open?{q}" };
    }

    /// The widget's page.
    JsonObject OpenPage() => new() { ["type"] = "Action.OpenUrl", ["url"] = scope == null ? $"{WhiteLabel.Cli}://open" : $"{WhiteLabel.Cli}://open?scope={Uri.EscapeDataString(scope)}" };

    static JsonObject Wrap(JsonNode el, JsonObject action) => new() { ["type"] = "Container", ["items"] = new JsonArray(el), ["selectAction"] = action };

    // MARK: Nodes

    JsonNode? Node(JsonElement n, Place at)
    {
        var el = Content(n, at);
        var kind = Str(n, "t");
        if (el != null && kind is not ("item" or "link" or "toggle" or "button") && n.TryGetProperty("link", out var link) && Open(link) is { } open)
            return el is JsonObject o && o["selectAction"] == null && o["type"]?.GetValue<string>() is "Container" or "ColumnSet" ? With(o, "selectAction", open) : Wrap(el, open);
        return el;
    }

    static JsonObject With(JsonObject o, string key, JsonNode value)
    {
        o[key] = value;
        return o;
    }

    JsonNode? Content(JsonElement n, Place at)
    {
        switch (Str(n, "t"))
        {
            case "col":
                return Column(n, at);
            case "row":
                return Row(n, at);
            case "text":
                {
                    var (color, subtle) = Tone(Str(n, "tone"));
                    var weight = Str(n, "weight") == "bold" || Str(n, "weight") == "medium" || Str(n, "size") is "xl" or "lg" ? "bolder" : null;
                    var lines = Num(n, "lines") is { } l ? (int?)l : null;
                    return Text(Str(n, "text") ?? "", Size(Str(n, "size")), subtle || at.Dim, color, weight, wrap: !(at.InRow && lines == null) && lines != 1, lines: lines, center: at.Center);
                }
            case "time":
                {
                    var (color, subtle) = Tone(Str(n, "tone"));
                    return Text(Time(n), Size(Str(n, "size")), subtle || at.Dim, color, center: at.Center);
                }
            case "stat":
                {
                    var (color, subtle) = Tone(Str(n, "tone"));
                    var value = Value(n);
                    var line = new JsonArray(new JsonObject
                    {
                        ["type"] = "Column", ["width"] = "auto",
                        ["items"] = new JsonArray(Text(value.Length == 0 ? "–" : value, "extraLarge", subtle || at.Dim, color, "bolder")),
                    });
                    if (Str(n, "unit") is { } unit)
                        line.Add(new JsonObject { ["type"] = "Column", ["width"] = "auto", ["verticalContentAlignment"] = "bottom", ["spacing"] = "small", ["items"] = new JsonArray(Text(unit, "small", true)) });
                    var stat = new JsonArray(new JsonObject { ["type"] = "ColumnSet", ["columns"] = line });
                    if (Str(n, "label") is { } label) stat.Add(With(Text(label, "small", true), "spacing", "none"));
                    return new JsonObject { ["type"] = "Container", ["items"] = stat };
                }
            case "gauge":
                return Ring(n, at);
            case "progress":
                {
                    var items = new JsonArray();
                    if (Str(n, "label") is { } label) items.Add(Text(label, "small", true));
                    var fraction = Fraction(n);
                    var color = Paint(Str(n, "tone"), accent)!;
                    var w = Num(n, "w") ?? at.Width;
                    var bar = Picture(_ => WidgetArt.Bar(fraction, color, w, 6), false, w, 6, stretch: Num(n, "w") == null, center: at.Center);
                    if (items.Count == 0) return bar;
                    ((JsonObject)bar)["spacing"] = "small";
                    items.Add(bar);
                    return new JsonObject { ["type"] = "Container", ["items"] = items };
                }
            case "spark":
                {
                    var values = Values(n);
                    var color = Paint(Str(n, "tone"), accent)!;
                    var w = Num(n, "w") ?? at.Width;
                    var h = Num(n, "h") ?? 28;
                    return Picture(_ => WidgetArt.Spark(values, Num(n, "max"), color, w, h), false, w, h, stretch: Num(n, "w") == null, center: at.Center);
                }
            case "chart":
                return Chart(n, at);
            case "list":
                {
                    var items = Children(n);
                    if (items.Count == 0) return Text(Str(n, "empty") ?? "Nothing here", "default", true);
                    var list = new JsonArray();
                    foreach (var item in items)
                        if (Node(item, at with { InRow = false }) is JsonObject el)
                        {
                            if (list.Count > 0)
                            {
                                el["separator"] = true;
                                el["spacing"] = "small";
                            }
                            list.Add(el);
                        }
                    return new JsonObject { ["type"] = "Container", ["items"] = list };
                }
            case "item":
                return Item(n, at);
            case "image":
                {
                    if (!Uri.TryCreate(Str(n, "url") ?? "", UriKind.Absolute, out var url) || url.Scheme is not ("https" or "http")) return null;
                    var image = new JsonObject { ["type"] = "Image", ["url"] = url.AbsoluteUri };
                    if (Num(n, "w") is { } w) image["width"] = $"{Math.Round(w)}px";
                    image["height"] = $"{Math.Round(Num(n, "h") ?? Num(n, "w") ?? 120)}px";
                    if (Bool(n, "round") == true) image["style"] = "person";
                    if (at.Center) image["horizontalAlignment"] = "center";
                    return image;
                }
            case "icon":
                return Icon(n, at);
            case "dot":
                {
                    var color = Paint(Str(n, "tone"), "#34c759")!;
                    return Picture(_ => WidgetArt.Dot(color, 8), false, 8, 8, center: at.Center);
                }
            case "link":
                {
                    var t = Text(Str(n, "text") ?? "", Size(Str(n, "size")), color: "accent", lines: (int)(Num(n, "lines") ?? 1), center: at.Center);
                    return n.TryGetProperty("link", out var link) && Open(link) is { } open ? Wrap(t, open) : t;
                }
            case "button":
                return Button(n);
            case "toggle":
                return Toggle(n, at);
            default:
                return null;
        }
    }

    /// Nodes one above another; a node that grows takes the height that's left, and a divider is a line above the next.
    JsonNode Column(JsonElement n, Place at)
    {
        var gap = Num(n, "gap") ?? 4;
        var items = new JsonArray();
        var line = false;
        var inner = at with { InRow = false, Dim = at.Dim || Bool(n, "dim") == true, Center = Str(n, "align") == "center" };
        foreach (var kid in Children(n))
        {
            if (Str(kid, "t") == "divider")
            {
                line = true;
                continue;
            }
            var el = Node(kid, inner) as JsonObject ?? (Bool(kid, "grow") == true ? new JsonObject { ["type"] = "Container", ["items"] = new JsonArray() } : null);
            if (el == null) continue;
            if (items.Count > 0) el["spacing"] = Spacing(gap);
            if (line) el["separator"] = true;
            line = false;
            if (Bool(kid, "grow") == true) el["height"] = "stretch";
            items.Add(el);
        }
        return new JsonObject { ["type"] = "Container", ["items"] = items };
    }

    /// The board's spacings, nearest a tree's gap.
    static string Spacing(double gap) => gap switch { <= 1 => "none", <= 5 => "small", <= 11 => "default", _ => "medium" };

    /// Nodes side by side: one that grows (or else the first text) takes what's left, the rest their own room.
    JsonNode Row(JsonElement n, Place at)
    {
        var gap = Num(n, "gap") ?? 6;
        var kids = Children(n).Where(k => Str(k, "t") != "divider").ToList();
        var grower = kids.FindIndex(k => Bool(k, "grow") == true);
        if (grower < 0) grower = kids.FindIndex(k => Str(k, "t") == "text");
        // What the fixed ones take, so the one that grows knows its room.
        double Own(JsonElement k) => Num(k, "w") ?? Str(k, "t") switch
        {
            "icon" => Num(k, "w") ?? 16,
            "dot" => 8,
            "gauge" => Num(k, "w") ?? 46,
            "toggle" => 44,
            "row" => Children(k).Sum(Own) + (Num(k, "gap") ?? 6) * Math.Max(0, Children(k).Count - 1),
            _ => 40,
        };
        var fixedRoom = kids.Where((k, i) => i != grower).Sum(Own) + gap * Math.Max(0, kids.Count - 1);
        var columns = new JsonArray();
        for (var i = 0; i < kids.Count; i++)
        {
            var kid = kids[i];
            var width = i == grower ? Math.Max(24, at.Width - fixedRoom) : Own(kid);
            var el = Node(kid, at with { Width = width, InRow = true, Dim = at.Dim || Bool(n, "dim") == true, Center = false });
            var column = new JsonObject
            {
                ["type"] = "Column", ["verticalContentAlignment"] = "center",
                ["width"] = Num(kid, "w") is { } w ? $"{Math.Round(w)}px" : i == grower ? "stretch" : "auto",
                ["items"] = el == null ? new JsonArray() : new JsonArray(el),
            };
            if (columns.Count > 0) column["spacing"] = Spacing(gap);
            columns.Add(column);
        }
        return new JsonObject { ["type"] = "ColumnSet", ["columns"] = columns };
    }

    /// "now", "5 min", "3 h", then the weekday, then the date; or as `style` says (src/screen/widgets.js's).
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

    /// A ring filled `value / max` of the way round, with its label in it: the label is the card's text, over the ring.
    JsonNode Ring(JsonElement n, Place at)
    {
        var size = Num(n, "w") ?? 46;
        var fraction = Fraction(n);
        var color = Paint(Str(n, "tone"), accent)!;
        var label = Str(n, "label");
        if (label == "") return Picture(_ => WidgetArt.Ring(fraction, color, size), false, size, size, center: at.Center);
        var face = new JsonObject
        {
            ["type"] = "Container", ["minHeight"] = $"{Math.Round(size)}px", ["verticalContentAlignment"] = "center",
            ["backgroundImage"] = new JsonObject { ["url"] = WidgetArt.Ring(fraction, color, size), ["fillMode"] = "cover", ["horizontalAlignment"] = "center", ["verticalAlignment"] = "center" },
            ["items"] = new JsonArray(Text(label ?? $"{Math.Round(fraction * 100)}%", size >= 60 ? "default" : "small", weight: "bolder", center: true)),
        };
        return new JsonObject
        {
            ["type"] = "ColumnSet", ["horizontalAlignment"] = at.Center ? "center" : "left",
            ["columns"] = new JsonArray(new JsonObject { ["type"] = "Column", ["width"] = $"{Math.Round(size)}px", ["items"] = new JsonArray(face) }),
        };
    }

    /// Bars, or a line, with each value's label under it.
    JsonNode Chart(JsonElement n, Place at)
    {
        var values = Values(n);
        var labels = n.TryGetProperty("labels", out var l) && l.ValueKind == JsonValueKind.Array ? l.EnumerateArray().Select(x => x.ValueKind == JsonValueKind.String ? x.GetString() ?? "" : "").ToList() : [];
        var color = Paint(Str(n, "tone"), accent)!;
        var h = (Num(n, "h") ?? 80) - (labels.Count > 0 ? 16 : 0);
        var w = at.Width;
        var items = new JsonArray(Picture(_ => WidgetArt.Chart(values, Str(n, "kind") == "line", color, w, h), false, w, h, stretch: true));
        if (labels.Count > 0)
        {
            var columns = new JsonArray();
            for (var i = 0; i < values.Count; i++)
                columns.Add(new JsonObject { ["type"] = "Column", ["width"] = "1", ["spacing"] = "none", ["items"] = new JsonArray(Text(i < labels.Count ? labels[i] : "", "small", true, center: true)) });
            items.Add(new JsonObject { ["type"] = "ColumnSet", ["spacing"] = "none", ["columns"] = columns });
        }
        return new JsonObject { ["type"] = "Container", ["items"] = items };
    }

    /// An icon: named (Phosphor's, filled), or an SVG path outlined, with its badge.
    JsonNode? Icon(JsonElement n, Place at)
    {
        var path = Str(n, "path");
        var w = Num(n, "w") ?? (path == null ? Text(Str(n, "size")) * 1.3 : 22);
        var tone = Paint(Str(n, "tone"), path == null ? accent : null);
        var badge = Str(n, "badge");
        if (path != null) return Picture(fg => WidgetArt.Outline(path, badge, tone ?? fg, w), tone == null, w, w, center: at.Center);
        if (Str(n, "name") is not { } name || !WidgetArt.Has(name)) return null;
        return Picture(_ => WidgetArt.Glyph(name, badge, tone!, w), false, w, w, center: at.Center);
    }

    /// The window's text sizes, in pixels, for an icon as big as its text.
    static double Text(string? size) => size switch { "xl" => 22, "lg" => 15, "sm" => 11, "xs" => 10, _ => 12.5 };

    /// A row: its icon, title and subtitle, and what's at its end (a time, or text).
    JsonNode Item(JsonElement n, Place at)
    {
        var columns = new JsonArray();
        if (Str(n, "icon") is { } name && WidgetArt.Has(name))
        {
            var color = Paint(Str(n, "tone"), accent)!;
            columns.Add(new JsonObject { ["type"] = "Column", ["width"] = "auto", ["verticalContentAlignment"] = "center", ["items"] = new JsonArray(Picture(_ => WidgetArt.Glyph(name, null, color, 18), false, 18, 18)) });
        }
        var texts = new JsonArray(Text(Str(n, "title") ?? "", "default", at.Dim, lines: 1));
        if (Str(n, "subtitle") is { } sub) texts.Add(With(Text(sub, "small", true, lines: 1), "spacing", "none"));
        columns.Add(new JsonObject { ["type"] = "Column", ["width"] = "stretch", ["verticalContentAlignment"] = "center", ["items"] = texts });
        var end = Num(n, "at") is not null ? Time(n) : Str(n, "trailing");
        if (end != null) columns.Add(new JsonObject { ["type"] = "Column", ["width"] = "auto", ["verticalContentAlignment"] = "center", ["items"] = new JsonArray(Text(end, "small", true)) });
        var row = new JsonObject { ["type"] = "ColumnSet", ["columns"] = columns };
        if (n.TryGetProperty("link", out var link) && Open(link) is { } open) row["selectAction"] = open;
        return row;
    }

    static JsonNode Input(JsonElement n) => n.TryGetProperty("input", out var i) && i.ValueKind == JsonValueKind.Object ? JsonNode.Parse(i.GetRawText())! : new JsonObject();

    /// A button: the board's own, running its operation.
    JsonNode? Button(JsonElement n)
    {
        if (Str(n, "op") is not { } op) return null;
        var action = new JsonObject
        {
            ["type"] = "Action.Execute", ["title"] = Bind(Str(n, "label") ?? Str(n, "text") ?? ""), ["verb"] = "run",
            ["data"] = new JsonObject { ["app"] = app, ["op"] = op, ["input"] = Input(n) },
        };
        if (Str(n, "tone") == "bad") action["style"] = "destructive";
        if (Str(n, "icon") is { } icon && WidgetArt.Has(icon)) action["iconUrl"] = WidgetArt.Glyph(icon, null, Paint(Str(n, "tone"), accent)!, 16);
        return new JsonObject { ["type"] = "ActionSet", ["actions"] = new JsonArray(action) };
    }

    /// A toggle: its label, and a switch drawn on or off; tapping the row flips it, running its operation with `on`.
    JsonNode Toggle(JsonElement n, Place at)
    {
        var on = Bool(n, "on") == true;
        var row = new JsonObject
        {
            ["type"] = "ColumnSet",
            ["columns"] = new JsonArray(
                new JsonObject { ["type"] = "Column", ["width"] = "stretch", ["verticalContentAlignment"] = "center", ["items"] = new JsonArray(Text(Str(n, "label") ?? "", "default", at.Dim, lines: 1)) },
                new JsonObject { ["type"] = "Column", ["width"] = "auto", ["verticalContentAlignment"] = "center", ["items"] = new JsonArray(Picture(_ => WidgetArt.Switch(on, accent), false, 40, 20)) }),
        };
        if (Str(n, "op") is { } op)
        {
            var input = Input(n);
            input["on"] = !on;
            row["selectAction"] = new JsonObject { ["type"] = "Action.Execute", ["verb"] = "run", ["data"] = new JsonObject { ["app"] = app, ["op"] = op, ["input"] = input } };
        }
        return row;
    }
}
