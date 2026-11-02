// A page, as the Mac app draws one (apple/Shared/Views/PageView.swift): its title, then one grid as a Home Screen's,
// the widgets its person put there and its apps' icons in their order, then its Services folder. Square cells an icon
// wide, as many across as fit; each item in the first place it fits, row by row, widgets on an even column; an xl
// widget where there's no room is drawn large.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;

namespace One;

public sealed class PageView : ContentControl
{
    const double Cell = 82, Gap = 14;
    static readonly Dictionary<string, (int w, int h)> cells = new() { ["small"] = (2, 2), ["medium"] = (4, 2), ["large"] = (4, 4), ["xl"] = (8, 4) };
    static readonly Dictionary<string, string> sizeNames = new() { ["small"] = "Small", ["medium"] = "Medium", ["large"] = "Large", ["xl"] = "Extra Large" };

    readonly Page page;
    readonly MainWindow window;
    readonly Canvas grid = new();
    int columns;

    public PageView(Page page, MainWindow window)
    {
        this.page = page;
        this.window = window;
        var content = new StackPanel { Padding = new Thickness(32, 8, 32, 48), HorizontalAlignment = HorizontalAlignment.Left };
        content.Children.Add(Look.Header(page.Title, page.Subtitle));
        var store = Store.Shared;
        if (store.AppsOn(page).Count == 0 && store.WidgetsOn(page).Count == 0)
        {
            var empty = new StackPanel { Spacing = 6, Margin = new Thickness(0, 0, 0, 24) };
            empty.Children.Add(Look.Text("Nothing here yet", 17, semibold: true));
            empty.Children.Add(Look.Text(store.CanArrange(page.Scope) ? $"Apps you add to {page.Title} show up here." : $"When its admins add apps to {page.Title}, they show up here.", 14, Look.Secondary, wrap: true));
            if (store.CanArrange(page.Scope) && store.StoreApp() is { } shop)
            {
                var open = Look.Accented("Open the App Store", () => window.Go(store.ScreenRoute(shop, null)));
                open.Margin = new Thickness(0, 10, 0, 0);
                empty.Children.Add(open);
            }
            content.Children.Add(empty);
        }
        content.Children.Add(grid);
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        VerticalContentAlignment = VerticalAlignment.Stretch;
        Content = new ScrollViewer { Content = content, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled };
        SizeChanged += (_, e) =>
        {
            var across = Math.Max(4, (int)Math.Floor((e.NewSize.Width - 64 + Gap) / (Cell + Gap)));
            if (across != columns) Lay(across);
        };
    }

    /// Places each item in the first place it fits, row by row.
    void Lay(int across)
    {
        columns = across;
        grid.Children.Clear();
        var taken = new List<bool[]>();
        bool Free(int x, int y, int w, int h)
        {
            for (var row = y; row < y + h; row++)
                for (var col = x; col < x + w; col++)
                    if (row < taken.Count && taken[row][col]) return false;
            return true;
        }
        var bottom = 0;
        foreach (var (el, w, h) in Elements())
        {
            for (var y = 0; ; y++)
            {
                var found = -1;
                for (var x = 0; x <= columns - w; x += w > 1 ? 2 : 1)
                    if (Free(x, y, w, h)) { found = x; break; }
                if (found < 0) continue;
                for (var row = y; row < y + h; row++)
                {
                    while (taken.Count <= row) taken.Add(new bool[columns]);
                    for (var col = found; col < found + w; col++) taken[row][col] = true;
                }
                Canvas.SetLeft(el, found * (Cell + Gap));
                Canvas.SetTop(el, y * (Cell + Gap));
                el.Width = w * Cell + (w - 1) * Gap;
                el.Height = h * Cell + (h - 1) * Gap;
                grid.Children.Add(el);
                bottom = Math.Max(bottom, y + h);
                break;
            }
        }
        grid.Width = columns * Cell + (columns - 1) * Gap;
        grid.Height = Math.Max(0, bottom * (Cell + Gap) - Gap);
    }

    IEnumerable<(FrameworkElement el, int w, int h)> Elements()
    {
        var store = Store.Shared;
        foreach (var item in store.Items(page))
        {
            if (item.Widget is { } widget)
            {
                var size = item.Size;
                if (cells[size].w > columns) size = "large";
                var (w, h) = cells.GetValueOrDefault(size, cells["medium"]);
                yield return (Widget(widget, item.Kept!, size), w, h);
            }
            else yield return (Icon(item.App!), 1, 1);
        }
        yield return (Folder(), 1, 1);
    }

    /// An app's icon with its name under it and its count, opening its first screen; right-click for what it offers.
    FrameworkElement Icon(AppInfo app)
    {
        var store = Store.Shared;
        var badge = store.Badge(app.Name, page);
        var button = Tile(Look.Tile(app, 60), app.Label, badge, () => window.Go(store.ScreenRoute(app, page)));
        button.ContextFlyout = Menu(Actions(app, page, window));
        return button;
    }

    FrameworkElement Folder() => Tile(Look.Folder(Store.Shared.ServicesOn(page), 60), "Services", 0, () => window.Go(Route.Services(page.Scope)));

    static Button Tile(FrameworkElement icon, string name, int badge, Action click)
    {
        var face = new Grid { Width = 60, Height = 60, HorizontalAlignment = HorizontalAlignment.Center };
        face.Children.Add(icon);
        if (badge > 0)
        {
            var count = Look.Count(badge);
            count.HorizontalAlignment = HorizontalAlignment.Right;
            count.VerticalAlignment = VerticalAlignment.Top;
            count.Margin = new Thickness(0, -6, -8, 0);
            face.Children.Add(count);
        }
        var panel = new StackPanel { Spacing = 5 };
        panel.Children.Add(face);
        var label = Look.Text(name, 12);
        label.HorizontalAlignment = HorizontalAlignment.Center;
        label.TextAlignment = TextAlignment.Center;
        label.MaxWidth = 80;
        panel.Children.Add(label);
        var button = new Button
        {
            Content = panel, Padding = new Thickness(0, 2, 0, 0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0), VerticalAlignment = VerticalAlignment.Top, HorizontalContentAlignment = HorizontalAlignment.Center,
            VerticalContentAlignment = VerticalAlignment.Top,
        };
        AutomationName(button, name);
        button.Click += (_, _) => click();
        return button;
    }

    static void AutomationName(DependencyObject el, string name) => Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(el, name);

    /// A widget: clicking it opens where it links, or its app; right-click for its sizes and removing it.
    FrameworkElement Widget(WidgetInfo w, ArrangedItem kept, string size)
    {
        var store = Store.Shared;
        var app = store.App(w.App);
        var tree = Store.Tree(w, size, kept);
        var error = Store.PlacedError(w, kept);
        var host = new WidgetHost(
            async (op, input) =>
            {
                try { await Node.Call(w.App, op, input); }
                catch (Exception e) { return e.Message; }
                try { await store.RefreshWidgets(); } catch { }
                return null;
            },
            link => Follow(link, w));
        var card = WidgetTree.Card(w.Title ?? app?.Label ?? w.Widget, tree, error, host, app?.Label ?? w.App);
        AutomationName(card, w.Title ?? app?.Label ?? w.Widget);
        card.Tapped += (_, e) =>
        {
            if (e.OriginalSource is DependencyObject source && Inside<ButtonBase>(source, card)) return;
            if (tree.ValueKind == JsonValueKind.Object && tree.TryGetProperty("link", out var link) && link.ValueKind != JsonValueKind.Null) Follow(link, w);
            else if (app?.HasScreens == true) window.Go(store.ScreenRoute(app, page));
        };
        var menu = new MenuFlyout();
        foreach (var s in w.Supported.Where(s => cells[s].w <= columns))
        {
            var choice = new ToggleMenuFlyoutItem { Text = sizeNames[s], IsChecked = s == size };
            choice.Click += async (_, _) =>
            {
                if (s != size) await Change(() => store.Arrange(page, i => i.Id == kept.Id ? Resized(i, s) : i));
            };
            menu.Items.Add(choice);
        }
        if (error != null)
        {
            menu.Items.Add(new MenuFlyoutSeparator());
            var copy = new MenuFlyoutItem { Text = "Copy What Went Wrong" };
            copy.Click += (_, _) => Copy(error);
            menu.Items.Add(copy);
        }
        menu.Items.Add(new MenuFlyoutSeparator());
        var remove = new MenuFlyoutItem { Text = "Remove Widget", Foreground = Look.Toned("bad") };
        remove.Click += async (_, _) => await Change(() => store.Arrange(page, i => i.Id == kept.Id ? null : i));
        menu.Items.Add(remove);
        card.ContextFlyout = menu;
        return card;
    }

    static ArrangedItem Resized(ArrangedItem i, string size) => new() { App = i.App, Widget = i.Widget, Id = i.Id, Size = size, Options = i.Options };

    static bool Inside<T>(DependencyObject el, DependencyObject until)
    {
        for (var at = el; at != null && at != until; at = VisualTreeHelper.GetParent(at))
            if (at is T) return true;
        return false;
    }

    /// A link: a web page in the browser, or one of its app's screens.
    void Follow(JsonElement link, WidgetInfo w)
    {
        if (link.ValueKind == JsonValueKind.String)
        {
            if (Uri.TryCreate(link.GetString(), UriKind.Absolute, out var url) && url.Scheme is "https" or "http") Shell.Open(url.AbsoluteUri);
        }
        else if (link.ValueKind == JsonValueKind.Object && link.TryGetProperty("screen", out var screen) && screen.ValueKind == JsonValueKind.String)
        {
            var parameters = link.TryGetProperty("params", out var p) && p.ValueKind == JsonValueKind.Object ? p.GetRawText() : "{}";
            window.Go(Route.ScreenOf(w.App, page.Scope, screen.GetString()!, parameters));
        }
    }

    public static void Copy(string text)
    {
        var data = new DataPackage();
        data.SetText(text);
        Clipboard.SetContent(data);
    }

    async Task Change(Func<Task> work)
    {
        try { await work(); }
        catch (Exception e) { await window.Alert(e.Message); }
    }

    /// What an app's menu offers: its settings, its page in the App Store, taking it off this page, and deleting it.
    public static List<(string label, bool danger, Func<Task> run)> Actions(AppInfo app, Page page, MainWindow window)
    {
        var store = Store.Shared;
        var out_ = new List<(string, bool, Func<Task>)>();
        if (store.SettingsRoute(app, page) is { } settings) out_.Add(("Settings", false, () => { window.Go(settings); return Task.CompletedTask; }));
        if (store.StoreRoute(app) is { } inStore) out_.Add(("Open in App Store", false, () => { window.Go(inStore); return Task.CompletedTask; }));
        if (store.CanArrange(page.Scope) && (app.Deploys ?? 0) > 0)
            out_.Add(($"Remove from {page.Title}", true, () => window.Changing(() => Node.Install(app.Name, page.Scope, false))));
        if (store.Owns(app) && (app.Deploys ?? 0) > 0)
            out_.Add(("Delete App…", true, async () =>
            {
                var sure = new ContentDialog
                {
                    Title = $"Delete {app.Label}?", XamlRoot = window.Content.XamlRoot, PrimaryButtonText = "Delete App", CloseButtonText = "Cancel",
                    Content = $"{app.Label} leaves every page, for everyone, and stops running. It waits in the App Store’s Recently Deleted, with its data, until its owners erase it.",
                };
                if (await sure.ShowAsync() == ContentDialogResult.Primary) await window.Changing(() => Node.Retire(app.Name, "deleted"));
            }));
        return out_;
    }

    public static MenuFlyout Menu(List<(string label, bool danger, Func<Task> run)> actions)
    {
        var menu = new MenuFlyout();
        foreach (var (label, danger, run) in actions)
        {
            if (danger && menu.Items.Count > 0 && menu.Items.Last() is not MenuFlyoutSeparator && !menu.Items.OfType<MenuFlyoutItem>().Any(i => i.Tag is true))
                menu.Items.Add(new MenuFlyoutSeparator());
            var item = new MenuFlyoutItem { Text = label, Tag = danger };
            if (danger) item.Foreground = Look.Toned("bad");
            item.Click += async (_, _) => await run();
            menu.Items.Add(item);
        }
        return menu;
    }
}
