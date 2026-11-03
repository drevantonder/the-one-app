// The tint, for what a person taps: their own Windows accent colour, followed as they change it.
using System;
using Windows.UI;
using Windows.UI.ViewManagement;

namespace One;

public static class Tint
{
    // Kept for as long as the program runs: its event goes with it.
    static readonly UISettings settings = new();

    /// The person changed their accent colour, on a thread of Windows'.
    public static event Action? Changed;

    static Tint() => settings.ColorValuesChanged += (_, _) => Changed?.Invoke();

    /// On light, Windows' darker shade, as its own accent buttons and links are; on dark, the accent itself, so white
    /// still reads on it.
    public static Color Of(bool dark) => settings.GetColorValue(dark ? UIColorType.Accent : UIColorType.AccentDark1);

    public static string Hex(bool dark)
    {
        var c = Of(dark);
        return $"#{c.R:x2}{c.G:x2}{c.B:x2}";
    }

    public static string Css(bool dark)
    {
        var c = Of(dark);
        return $"rgb({c.R}, {c.G}, {c.B})";
    }
}
