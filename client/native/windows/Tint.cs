// The user's own Windows accent colour, which a widget's `accent` tone draws in on the Widgets board.
using Windows.UI;
using Windows.UI.ViewManagement;

namespace One;

public static class Tint
{
    // Kept for as long as the program runs: its event goes with it.
    static readonly UISettings settings = new();

    /// On light, Windows' darker shade, as its own accent buttons and links are; on dark, the accent itself, so white
    /// still reads on it.
    public static Color Of(bool dark) => settings.GetColorValue(dark ? UIColorType.Accent : UIColorType.AccentDark1);

    public static string Hex(bool dark)
    {
        var c = Of(dark);
        return $"#{c.R:x2}{c.G:x2}{c.B:x2}";
    }
}
