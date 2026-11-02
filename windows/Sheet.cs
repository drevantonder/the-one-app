// A screen over the window, as a sheet: a card on the dimmed window, its title and Done (Cancel for a Vault request,
// which is answered in it, or not), holding the screen in a WebView2 of its own. A screen in it may open another sheet
// over it, or alerts, which are Windows' own dialogs. Closed gives what the screen gave `one.close`, or null if the
// person closed it.
using System.Collections.Generic;
using System.Numerics;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace One;

public sealed class Sheet
{
    static readonly List<Sheet> open = [];

    readonly Panel host;
    readonly Grid layer = new() { Background = new SolidColorBrush(Color.FromArgb(0x50, 0, 0, 0)) };
    readonly TextBlock title = new() { FontSize = 14, FontWeight = FontWeights.SemiBold, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis };
    readonly string named;
    readonly TaskCompletionSource<JsonNode?> closed = new();

    public Task<JsonNode?> Closed => closed.Task;

    /// Over `host`'s row 1 (the window below its title bar).
    public Sheet(Panel host, Route route, string named, string done)
    {
        this.host = host;
        this.named = named;
        title.Text = named;
        var card = new Grid
        {
            MaxWidth = 620, MaxHeight = 780, Margin = new Thickness(40), CornerRadius = new CornerRadius(8),
            Background = Look.Themed("SolidBackgroundFillColorBaseBrush"), BorderBrush = Look.Themed("SurfaceStrokeColorDefaultBrush"), BorderThickness = new Thickness(1),
            Shadow = new ThemeShadow(), Translation = new Vector3(0, 0, 32),
        };
        card.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        card.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        var bar = new Grid { Height = 48, Padding = new Thickness(16, 0, 10, 0) };
        bar.Children.Add(title);
        var button = new Button { Content = done, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center };
        button.Click += (_, _) => Close(null);
        bar.Children.Add(button);
        card.Children.Add(bar);
        var screen = new AppScreen(route, Retitle, this);
        Grid.SetRow(screen, 1);
        card.Children.Add(screen);
        layer.Children.Add(card);
        // A click beside it closes it, as Done does.
        layer.Tapped += (_, e) =>
        {
            if (e.OriginalSource == layer) Close(null);
        };
        Grid.SetRow(layer, 1);
    }

    public void Show()
    {
        open.Add(this);
        host.Children.Add(layer);
    }

    /// Its title: what the screen says, else what it is.
    public void Retitle(string text) => title.Text = text.Length > 0 ? text : named;

    public void Close(JsonNode? value)
    {
        if (closed.Task.IsCompleted) return;
        open.Remove(this);
        host.Children.Remove(layer);
        closed.TrySetResult(value?.DeepClone());
    }

    /// Every sheet, as the person signs out.
    public static void CloseAll()
    {
        foreach (var sheet in open.ToArray()) sheet.Close(null);
    }
}
