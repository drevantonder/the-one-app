// The window, as the Mac app's (apple/Mac/RootView.swift, Sidebar.swift): a sidebar with a section per page, listing
// its apps and its Services, then Settings', listing its items; beside it, where the window is. A sidebar row replaces where the
// window is; what a screen opens is pushed on top, so Back returns to it. Signed out, it welcomes the person and signs
// them in.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Text;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;

namespace One;

public sealed class MainWindow : Window
{
    readonly Grid root = new();
    readonly ContentControl body = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    NavigationView? nav;
    Grid detail = new();
    CancellationTokenSource? listening;

    /// Where the window is, and what screens opened over it, each kept as drawn so Back returns to it as it was.
    Route selection = Route.PageOf("user");
    readonly List<(Route route, FrameworkElement view)> pushed = [];
    FrameworkElement? shown;

    public MainWindow()
    {
        Title = Brand.Name;
        SystemBackdrop = new MicaBackdrop();
        ExtendsContentIntoTitleBar = true;
        AppWindow.SetIcon(System.IO.Path.Combine(AppContext.BaseDirectory, "Assets", "App.ico"));
        // As big as a Mac app's window, on a screen with room; a smaller one, most of it.
        var dpi = GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(this)) / 96.0;
        var room = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
        var (w, h) = (Math.Min((int)(1180 * dpi), room.Width * 92 / 100), Math.Min((int)(780 * dpi), room.Height * 92 / 100));
        AppWindow.MoveAndResize(new Windows.Graphics.RectInt32(room.X + (room.Width - w) / 2, room.Y + (room.Height - h) / 2, w, h));

        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(40) });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        var bar = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, Padding = new Thickness(16, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };
        bar.Children.Add(new Image { Source = new BitmapImage(new Uri(System.IO.Path.Combine(AppContext.BaseDirectory, "Assets", "Logo.png"))), Width = 16, Height = 16 });
        bar.Children.Add(new TextBlock { Text = Brand.Name, FontSize = 12, VerticalAlignment = VerticalAlignment.Center });
        var titleBar = new Grid { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        titleBar.Children.Add(bar);
        root.Children.Add(titleBar);
        SetTitleBar(titleBar);
        Grid.SetRow(body, 1);
        root.Children.Add(body);
        root.RequestedTheme = Appearance.Theme;
        Content = root;
        // Windows turning dark or light while it's open: what's drawn with the other's colours is drawn again.
        root.ActualThemeChanged += (_, _) =>
        {
            Buttons();
            Redraw();
        };
        Buttons();
        Store.Shared.Changed += Redraw;
        // An app's own change (a sign-in finished on its settings screen) lists its accounts again.
        Store.Shared.Event += e =>
        {
            if (e["type"]?.GetValue<string>() == "changed") ServicesView.Stale((e["apps"] as System.Text.Json.Nodes.JsonArray)?.Select(a => a?.ToString() ?? ""));
        };
        // The Widgets board's widgets keep up with the window's.
        Store.Shared.WidgetsRead += () =>
        {
            var list = WidgetCache.Of(Store.Shared);
            var accent = Store.Shared.Config?.Accent ?? Brand.Accent;
            _ = System.Threading.Tasks.Task.Run(() => Widgets.Changed(list, accent));
        };
        Closed += (_, _) => listening?.Cancel();
        _ = Start();
    }

    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);

    /// The title bar's buttons in the window's light or dark, which Windows leaves as they were when it changes.
    void Buttons()
    {
        var dark = root.ActualTheme == ElementTheme.Dark;
        Windows.UI.Color Ink(byte a) => dark ? Windows.UI.Color.FromArgb(a, 255, 255, 255) : Windows.UI.Color.FromArgb(a, 0, 0, 0);
        var bar = AppWindow.TitleBar;
        bar.ButtonBackgroundColor = bar.ButtonInactiveBackgroundColor = Microsoft.UI.Colors.Transparent;
        bar.ButtonForegroundColor = bar.ButtonHoverForegroundColor = bar.ButtonPressedForegroundColor = Ink(228);
        bar.ButtonInactiveForegroundColor = Ink(110);
        bar.ButtonHoverBackgroundColor = Ink(20);
        bar.ButtonPressedBackgroundColor = Ink(36);
    }

    /// Brings the window in front of the others, as a widget's link opens it.
    public void Bring()
    {
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(this);
        if (AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } p) p.Restore();
        SetForegroundWindow(hwnd);
    }

    /// Where a link opens, once the window shows the pages: a widget's (`<cli>://open?app=…&screen=…&params=…&scope=…`),
    /// or a page at your One App's address, as the web client has them (kernel/web/web.js): `/`, `/page/<scope>`,
    /// `/services/<scope>`, `/app/<app>/<screen>?params=&scope=`, `/settings`, `/settings/<item>?params=`, and
    /// `/vault?id=&scope=` (or `<cli>://vault?…`), a request for an item, in the Vault's sheet. An invite link
    /// (`<address>/invite#…`) welcomes the person with it, as pasting it does. `<cli>://signed-in`, where the browser
    /// comes back after signing in, only brings the window to the front.
    Route? opening;
    (string scope, string id)? asking;

    public void Open(Uri link)
    {
        var q = System.Web.HttpUtility.ParseQueryString(link.Query);
        if (link.Scheme == "https")
        {
            if (link.AbsolutePath == "/invite" && link.Fragment.Length > 1)
            {
                Invited(link);
                return;
            }
            // Another One App's, while this computer is signed in to yours, stays in the browser.
            var address = link.GetLeftPart(UriPartial.Authority);
            if (Auth.Shared.Address != null && address != Auth.Shared.Address)
            {
                Shell.Open(link.AbsoluteUri);
                return;
            }
            if (Auth.Shared.Address == null)
            {
                Auth.Shared.Take(address);
                Welcome(null);
            }
            var path = link.AbsolutePath.Split('/', StringSplitOptions.RemoveEmptyEntries).Select(Uri.UnescapeDataString).ToArray();
            string? At(int i) => path.Length > i ? path[i] : null;
            if (At(0) == "vault")
            {
                if (q["id"] is { Length: > 0 } id && q["scope"] is { Length: > 0 } s) asking = (s, id);
            }
            else opening = At(0) switch
            {
                "page" when At(1) is { } scope => Route.PageOf(scope),
                "services" when At(1) is { } scope => Route.Services(scope),
                "app" when At(1) is { } app => Route.ScreenOf(app, q["scope"], At(2) ?? "main", q["params"] ?? "{}"),
                "settings" => At(1) is { } item ? Route.SettingOf(item, q["params"] ?? "{}") : Route.Settings,
                _ => Route.PageOf("user"),
            };
        }
        else if (link.Host == "open")
        {
            var scope = q["scope"];
            opening = q["app"] is { Length: > 0 } app ? Route.ScreenOf(app, scope, q["screen"] ?? "main", q["params"] ?? "{}") : Route.PageOf(scope ?? "user");
        }
        else if (link.Host == "vault" && q["id"] is { Length: > 0 } id && q["scope"] is { Length: > 0 } s) asking = (s, id);
        if (nav != null) Opening();
    }

    void Opening()
    {
        if (opening is { } r)
        {
            opening = null;
            // Its page, if this computer opens it; an app's screen on a page it isn't on still opens.
            if (r.Kind is not ("page" or "services") || Store.Shared.PageFor(r.Scope) != null) Go(r);
        }
        if (asking is { } a)
        {
            asking = null;
            DispatcherQueue.TryEnqueue(async () => await VaultSheet.Show(Content.XamlRoot, a.scope, a.id));
        }
    }

    /// An invite link: signed out, the welcome with it, its page in the browser, where the person makes their passkey;
    /// signed in, only its page, so this computer stays signed in.
    void Invited(Uri invite)
    {
        if (Auth.Shared.SignedIn)
        {
            Shell.Open(invite.AbsoluteUri);
            return;
        }
        Auth.Shared.Take(invite.AbsoluteUri);
        Shell.Open(invite.AbsoluteUri);
        Welcome(null, invited: true);
    }

    public void Restyle()
    {
        root.RequestedTheme = Appearance.Theme;
        Redraw();
    }

    // MARK: Starting

    async Task Start()
    {
        _ = Shell.Claim(Auth.Shared.Address);
        if (Auth.Shared.Address == null || !Auth.Shared.SignedIn)
        {
            Welcome(null);
            return;
        }
        body.Content = Centered(new StackPanel
        {
            Spacing = 12,
            Children = { new ProgressRing { IsActive = true, Width = 24, Height = 24 }, Look.Text($"Starting {Brand.Name}…", 13, Look.Secondary) },
        });
        for (var wait = 1000; ; wait = Math.Min(wait * 2, 15_000))
        {
            try
            {
                await Store.Shared.RefreshState();
                break;
            }
            catch (SignedOutException)
            {
                Welcome(null);
                return;
            }
            catch (Exception e)
            {
                body.Content = Look.Unavailable($"Can’t Reach {Brand.Name}", e.Message, Look.Text("It keeps trying.", 13, Look.Secondary));
                await Task.Delay(wait);
            }
        }
        if (Store.Shared.Install == null)
        {
            body.Content = Look.Unavailable("Nothing Here Yet", $"It isn’t set up yet. Set it up from the {Brand.Name} app on one of your devices.");
            return;
        }
        try
        {
            ShowMain();
        }
        catch (Exception e)
        {
            App.Problem(e);
            body.Content = Look.Unavailable("Something Went Wrong", e.Message);
            return;
        }
        listening = new CancellationTokenSource();
        _ = Store.Shared.Listen(() => DispatcherQueue.TryEnqueue(() => Welcome(null)), listening.Token);
    }

    static FrameworkElement Centered(FrameworkElement el)
    {
        el.HorizontalAlignment = HorizontalAlignment.Center;
        el.VerticalAlignment = VerticalAlignment.Center;
        return el;
    }

    /// Welcome: the install's address (or an invite link) the first time, then Sign In, in the person's browser.
    void Welcome(string? error, bool invited = false)
    {
        listening?.Cancel();
        nav = null;
        Store.Shared.Clear();
        var panel = new StackPanel { Spacing = 14, MaxWidth = 520, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var logo = new Image { Source = new BitmapImage(new Uri(System.IO.Path.Combine(AppContext.BaseDirectory, "Assets", "Logo.png"))), Width = 112, Height = 112, HorizontalAlignment = HorizontalAlignment.Center };
        panel.Children.Add(logo);
        panel.Children.Add(new TextBlock { Text = Brand.Name, FontSize = 32, FontWeight = FontWeights.SemiBold, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords, Margin = new Thickness(0, 0, 0, 14) });
        var problem = new TextBlock { Text = error ?? "", Foreground = Look.Toned("bad"), HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords, Visibility = error == null ? Visibility.Collapsed : Visibility.Visible };

        if (Auth.Shared.Address == null)
        {
            var address = new TextBox { PlaceholderText = "one.example.com, or an invite link", Width = 340, HorizontalAlignment = HorizontalAlignment.Center, IsSpellCheckEnabled = false };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(address, "Your One App’s address or invite link");
            panel.Children.Add(new TextBlock { Text = "Your One App’s address, or the invite link you were sent.", FontSize = 14, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords });
            panel.Children.Add(address);
            void Go()
            {
                try
                {
                    var invite = Auth.Shared.Take(address.Text);
                    if (invite != null) Shell.Open(invite.AbsoluteUri);
                    Welcome(null, invite != null);
                }
                catch (Exception e)
                {
                    problem.Text = e.Message;
                    problem.Visibility = Visibility.Visible;
                }
            }
            address.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) Go(); };
            var go = Look.Accented("Continue", Go);
            go.HorizontalAlignment = HorizontalAlignment.Center;
            panel.Children.Add(go);
            panel.Children.Add(problem);
        }
        else
        {
            if (invited)
                panel.Children.Add(new TextBlock { Text = "Make your passkey in your browser, where your invite opened. Then sign in here.", FontSize = 14, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords });
            var signIn = Look.Accented("Sign In", () => { });
            signIn.HorizontalAlignment = HorizontalAlignment.Center;
            signIn.Padding = new Thickness(28, 8, 28, 9);
            signIn.FontSize = 15;
            var waiting = new TextBlock { Text = "Finish signing in in your browser.", Foreground = Look.Secondary, HorizontalAlignment = HorizontalAlignment.Center, Visibility = Visibility.Collapsed };
            CancellationTokenSource? cancel = null;
            var stop = Look.Link("Cancel", () => cancel?.Cancel());
            stop.HorizontalAlignment = HorizontalAlignment.Center;
            stop.Visibility = Visibility.Collapsed;
            signIn.Click += async (_, _) =>
            {
                signIn.IsEnabled = false;
                waiting.Visibility = stop.Visibility = Visibility.Visible;
                problem.Visibility = Visibility.Collapsed;
                cancel = new CancellationTokenSource(TimeSpan.FromMinutes(10));
                try
                {
                    await Auth.Shared.SignIn(cancel.Token);
                }
                catch (Exception e)
                {
                    if (e is not OperationCanceledException)
                    {
                        problem.Text = e.Message;
                        problem.Visibility = Visibility.Visible;
                    }
                    signIn.IsEnabled = true;
                    waiting.Visibility = stop.Visibility = Visibility.Collapsed;
                    return;
                }
                SetForegroundWindow(WinRT.Interop.WindowNative.GetWindowHandle(this));
                await Start();
            };
            panel.Children.Add(signIn);
            panel.Children.Add(waiting);
            panel.Children.Add(stop);
            panel.Children.Add(problem);
            panel.Children.Add(new TextBlock { Text = new Uri(Auth.Shared.Address).Host, FontSize = 12, Foreground = Look.Tertiary, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 16, 0, 0) });
            if (string.IsNullOrEmpty(Brand.Address))
            {
                var other = Look.Link("Use another One App…", () =>
                {
                    Auth.Shared.Leave();
                    Welcome(null);
                });
                other.HorizontalAlignment = HorizontalAlignment.Center;
                panel.Children.Add(other);
            }
        }
        body.Content = new ScrollViewer { Content = new Grid { Padding = new Thickness(40), Children = { panel } }, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    }

    public async Task SignOut()
    {
        listening?.Cancel();
        await Auth.Shared.SignOut();
        pushed.Clear();
        selection = Route.PageOf("user");
        Welcome(null);
    }

    // MARK: The window

    void ShowMain()
    {
        // A window of its own each sign-in: nothing of the last one's.
        detail = new Grid();
        pushed.Clear();
        shown = null;
        nav = new NavigationView
        {
            PaneDisplayMode = NavigationViewPaneDisplayMode.Left, IsSettingsVisible = false, IsPaneToggleButtonVisible = false,
            OpenPaneLength = 250, IsBackButtonVisible = NavigationViewBackButtonVisible.Visible, IsTitleBarAutoPaddingEnabled = false,
            Content = detail, AlwaysShowHeader = false,
        };
        nav.ItemInvoked += (_, e) =>
        {
            if (e.InvokedItemContainer?.Tag is Route r) Go(r);
        };
        nav.BackRequested += (_, _) => Back();
        body.Content = nav;
        Redraw();
        Opening();
    }

    /// Goes somewhere in place of where the window is, as a sidebar row does.
    public void Go(Route r)
    {
        if (nav == null) return;
        selection = r;
        pushed.Clear();
        shown = null;
        DrawDetail();
        DrawSidebar();
    }

    /// Opens something over where the window is, as a screen opening another does: Back returns to it.
    public void Push(Route r)
    {
        if (nav == null) return;
        pushed.Add((r, View(r)));
        DrawDetail();
    }

    void Back()
    {
        if (pushed.Count == 0) return;
        pushed.RemoveAt(pushed.Count - 1);
        DrawDetail();
    }

    void Redraw()
    {
        if (nav == null) return;
        DrawSidebar();
        // An open screen stays as it is while the rest of the window redraws around it.
        if ((Top.Kind == "screen" || Top.Setting != null) && shown != null) return;
        if (pushed.Count > 0) pushed[^1] = (pushed[^1].route, View(pushed[^1].route));
        else shown = null;
        DrawDetail();
    }

    Route Top => pushed.Count > 0 ? pushed[^1].route : selection;

    /// Shows the view on top; those under it stay in the window, hidden, so a screen under another keeps its page.
    void DrawDetail()
    {
        var view = pushed.Count > 0 ? pushed[^1].view : shown ??= View(selection);
        var kept = pushed.Select(p => p.view).Append(shown).ToHashSet();
        foreach (var gone in detail.Children.OfType<FrameworkElement>().Where(c => !kept.Contains(c)).ToList()) detail.Children.Remove(gone);
        foreach (var child in detail.Children.OfType<FrameworkElement>()) child.Visibility = child == view ? Visibility.Visible : Visibility.Collapsed;
        if (!detail.Children.Contains(view)) detail.Children.Add(view);
        nav!.IsBackEnabled = pushed.Count > 0;
    }

    /// What a route shows.
    FrameworkElement View(Route r)
    {
        var store = Store.Shared;
        switch (r.Kind)
        {
            case "page":
                return store.PageFor(r.Scope) is { } p ? new PageView(p, this) : store.Pages().FirstOrDefault() is { } first ? new PageView(first, this)
                    : Look.Unavailable("Nothing Here Yet", "This computer was let into none of your pages. Sign out in Settings and sign in again to choose them.");
            case "services":
                return store.PageFor(r.Scope) is { } sp ? new ServicesView(sp, this) : new Grid();
            case "screen":
                return Screen(r);
            default:
                if (r.Setting == null) return SettingsView.Home(this);
                // An item with an app (the App Store) opens that app.
                return store.SettingsItemOf(r.Setting)?.App is { } a ? Screen(Route.ScreenOf(a, null)) : Screen(r);
        }
    }

    /// An app's screen, or a Settings screen, under a bar with its title, which the screen may change.
    FrameworkElement Screen(Route r)
    {
        var app = Store.Shared.App(r.App);
        var named = r.Setting != null ? Store.Shared.SettingsItemOf(r.Setting)?.Title ?? "Settings" : app?.Screens?.FirstOrDefault(s => s.Name == r.Screen)?.Title ?? app?.Label ?? r.App;
        var title = new TextBlock { Text = named, FontSize = 14, FontWeight = FontWeights.SemiBold, Margin = new Thickness(20, 6, 20, 6), Height = 22 };
        var grid = new Grid();
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(title);
        var screen = new AppScreen(r, Push, text => title.Text = text);
        Grid.SetRow(screen, 1);
        grid.Children.Add(screen);
        return grid;
    }

    /// One section per page, listing its apps and its Services; then Settings', listing its items with their counts.
    void DrawSidebar()
    {
        if (nav == null) return;
        var store = Store.Shared;
        nav.MenuItems.Clear();
        nav.FooterMenuItems.Clear();
        object? selected = null;
        var at = selection;
        NavigationViewItem Item(Route route, FrameworkElement content, int badge = 0)
        {
            var item = new NavigationViewItem { Content = content, Tag = route };
            if (badge > 0) item.InfoBadge = new InfoBadge { Value = badge };
            if (route == at) selected = item;
            return item;
        }
        FrameworkElement Labeled(FrameworkElement icon, string text)
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
            icon.VerticalAlignment = VerticalAlignment.Center;
            row.Children.Add(icon);
            row.Children.Add(new TextBlock { Text = text, VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis });
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, text);
            return row;
        }
        var first = true;
        void Head(Route route, string title)
        {
            var head = new TextBlock { Text = title, FontSize = 12, FontWeight = FontWeights.SemiBold, Foreground = route == at ? Look.AccentBrush : Look.Secondary, TextWrapping = TextWrapping.WrapWholeWords, MaxLines = 3, Margin = new Thickness(0, first ? 0 : 10, 0, 0) };
            first = false;
            var headItem = Item(route, head);
            headItem.MinHeight = 28;
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(headItem, title);
            nav.MenuItems.Add(headItem);
        }
        foreach (var page in store.Pages())
        {
            Head(Route.PageOf(page.Scope), page.Title);
            foreach (var app in store.AppsOn(page))
                nav.MenuItems.Add(Item(store.ScreenRoute(app, page), Labeled(Look.Tile(app, 20), app.Label), store.Badge(app.Name, page)));
            nav.MenuItems.Add(Item(Route.Services(page.Scope), Labeled(Look.Folder(store.ServicesOn(page), 20), "Services")));
        }
        Head(Route.Settings, "Settings");
        foreach (var item in store.SettingsItems)
            nav.MenuItems.Add(Item(Route.SettingOf(item.Name), Labeled(SettingsView.Icon(item, 20), item.Title), item.Badge ?? 0));
        nav.SelectedItem = selected;
    }

    /// Runs a change, then takes in what it changed; says what went wrong if it failed.
    public async Task Changing(Func<Task> work)
    {
        try { await Store.Shared.Change(work); }
        catch (Exception e) { await Alert(e.Message); }
    }

    public async Task Alert(string message)
    {
        if (Content.XamlRoot == null) return;
        await new ContentDialog { RequestedTheme = Look.Theme, Title = "Couldn’t Do That", Content = message, CloseButtonText = "OK", XamlRoot = Content.XamlRoot }.ShowAsync();
    }
}
