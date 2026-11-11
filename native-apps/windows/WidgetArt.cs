// The pictures a widget's card shows (WidgetCard.cs), drawn as src/screen/widgets.js draws them on a page: rings, sparks,
// bars, charts, dots, switches and icons, each as an image's data, since a card draws nothing but text and images.
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;
using System.Text.Json;

namespace One;

public static class WidgetArt
{
    /// Phosphor's icons, each a path on a 256-point grid, drawn filled: those the widgets name, as the app keeps them
    /// beside the widgets (native-apps/src/widgets.rs's icons.json), read with them.
    public static Dictionary<string, string> Icons { get; set; } = [];

    public static bool Has(string name) => Icons.ContainsKey(name);

    /// Text's colour on the board, light or dark; and muted things', either way.
    public static string Text(bool dark) => dark ? "#ffffff" : "#1b1b1b";
    public const string Muted = "#8a8a8a";
    /// A track under a ring or a bar: grey, faint on either theme.
    const string Track = "#80808040";

    static string F(double v) => v.ToString("0.##", CultureInfo.InvariantCulture);

    static string Image(double w, double h, string body) =>
        Uri($"<svg xmlns='http://www.w3.org/2000/svg' width='{F(w)}' height='{F(h)}' viewBox='0 0 {F(w)} {F(h)}'>{body}</svg>");

    static string Uri(string svg) => "data:image/svg+xml;base64," + Convert.ToBase64String(Encoding.UTF8.GetBytes(svg));

    /// `#rrggbbaa` as SVG's colour and opacity.
    static string Fill(string color, string attribute = "fill")
    {
        if (color.Length == 9 && int.TryParse(color[7..], NumberStyles.HexNumber, null, out var a))
            return $"{attribute}='{color[..7]}' {attribute}-opacity='{F(a / 255.0)}'";
        return $"{attribute}='{color}'";
    }

    public static string Bar(double fraction, string color, double w, double h)
    {
        var r = h / 2;
        var filled = fraction <= 0 ? 0 : Math.Max(h, w * fraction);
        return Image(w, h, $"<rect width='{F(w)}' height='{F(h)}' rx='{F(r)}' {Fill(Track)}/>" + (filled > 0 ? $"<rect width='{F(filled)}' height='{F(h)}' rx='{F(r)}' {Fill(color)}/>" : ""));
    }

    /// A line through numbers from 0 to `max`, shaded under; a dashed line when there are none.
    public static string Spark(List<double> values, double? max, string color, double w, double h)
    {
        if (values.Count == 0)
            return Image(w, h, $"<line x1='4' y1='{F(h / 2)}' x2='{F(w - 4)}' y2='{F(h / 2)}' stroke='{Muted}' stroke-width='1.5' stroke-dasharray='2 3'/>");
        var data = values.Count == 1 ? [values[0], values[0]] : values;
        var top = max ?? Math.Max(1, data.Max());
        var points = data.Select((v, i) => (x: i / (double)(data.Count - 1) * w, y: h - 1 - Math.Clamp(v, 0, top) / top * (h - 2))).ToList();
        var line = string.Join(" ", points.Select(p => $"{F(p.x)},{F(p.y)}"));
        return Image(w, h, $"<polygon points='{line} {F(w)},{F(h)} 0,{F(h)}' {Fill(color)} fill-opacity='0.2'/><polyline points='{line}' fill='none' {Fill(color, "stroke")} stroke-width='1.4' stroke-linejoin='round'/>");
    }

    /// A ring filled `fraction` of the way round, clockwise from the top.
    public static string Ring(double fraction, string color, double size)
    {
        var line = Math.Min(5, size / 6);
        var r = (size - line * 2) / 2;
        var c = size / 2;
        var body = $"<circle cx='{F(c)}' cy='{F(c)}' r='{F(r)}' fill='none' {Fill(Track, "stroke")} stroke-width='{F(line)}'/>";
        if (fraction > 0)
        {
            var angle = Math.Min(fraction, 0.9999) * 2 * Math.PI;
            var (x, y) = (c + r * Math.Sin(angle), c - r * Math.Cos(angle));
            body += $"<path d='M{F(c)},{F(c - r)} A{F(r)},{F(r)} 0 {(angle > Math.PI ? 1 : 0)} 1 {F(x)},{F(y)}' fill='none' {Fill(color, "stroke")} stroke-width='{F(line)}' stroke-linecap='round'/>";
        }
        return Image(size, size, body);
    }

    /// Bars, or a line, across `w`.
    public static string Chart(List<double> values, bool line, string color, double w, double h)
    {
        if (values.Count == 0) return Image(w, h, "");
        var top = Math.Max(values.Max(), 1e-9);
        var step = w / values.Count;
        if (line)
        {
            var points = string.Join(" ", values.Select((v, i) => $"{F(step * (i + 0.5))},{F(h - 1 - v / top * (h - 2))}"));
            return Image(w, h, $"<polyline points='{points}' fill='none' {Fill(color, "stroke")} stroke-width='2' stroke-linejoin='round' stroke-linecap='round'/>");
        }
        var bars = new StringBuilder();
        for (var i = 0; i < values.Count; i++)
        {
            var bh = Math.Max(1, values[i] / top * h);
            bars.Append($"<path d='M{F(step * i + 4)},{F(h)} V{F(h - bh + 3)} q0,-3 3,-3 H{F(step * (i + 1) - 7)} q3,0 3,3 V{F(h)} Z' {Fill(color)}/>");
        }
        return Image(w, h, bars.ToString());
    }

    public static string Dot(string color, double size) => Image(size, size, $"<circle cx='{F(size / 2)}' cy='{F(size / 2)}' r='{F(size / 2)}' {Fill(color)}/>");

    /// A switch, on (the accent, its knob at the end) or off (grey, its knob at the start).
    public static string Switch(bool on, string accent) => Image(40, 20,
        $"<rect width='40' height='20' rx='10' {Fill(on ? accent : "#8a8a8a99")}/><circle cx='{(on ? 30 : 10)}' cy='10' r='7' fill='#ffffff'/>");

    /// A badge (an SVG path on a 24-point grid) at the bottom trailing corner of an icon `w` across.
    static string Badge(string? badge, string color, double w) =>
        badge == null ? "" : $"<g transform='translate({F(w * 13 / 24)},{F(w * 13 / 24)}) scale({F(w * 11 / 24 / 24)})'><path d='{Escape(badge)}' {Fill(color)}/></g>";

    /// An SVG path on a 24-point grid, outlined.
    public static string Outline(string path, string? badge, string color, double w) => Image(w, w,
        $"<g transform='scale({F(w / 24)})'><path d='{Escape(path)}' fill='none' {Fill(color, "stroke")} stroke-width='1.5' stroke-linejoin='round' stroke-linecap='round'/></g>" + Badge(badge, color, w));

    /// A named icon, filled.
    public static string Glyph(string name, string? badge, string color, double w) => Image(w, w,
        $"<g transform='scale({F(w / 256)})'><path d='{Escape(Icons.GetValueOrDefault(name, ""))}' {Fill(color)}/></g>" + Badge(badge, color, w));

    static string Escape(string text) => System.Security.SecurityElement.Escape(text) ?? "";
}
