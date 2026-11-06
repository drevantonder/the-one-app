// The window, as the web client's (cloudflare/web/web.js): a sidebar with a section per page, listing its apps and its
// Accounts, then Settings', listing its items; beside it, where the window is: a screen the node serves (AppScreen), a
// page, Accounts, a Settings screen or an app's. A sidebar row replaces where the window is; what a screen opens is
// pushed on top, so Back returns to it, or slides up over it as a sheet. Back is ← in the title bar, shown once something
// is pushed, beside the path there ("Backups › Archive disk"), each part going back to it. The title bar and sidebar
// are on Mica; where the window is, on the kit's page colour. Signed out, it welcomes the person and signs them in.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json.Nodes;
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
    /// Where the window is in signing in: the welcome, starting (asking the node, or joining, showing its code until
    /// it's approved), or the person's pages, keeping up with the node.
    enum Phase { SignedOut, Starting, Joining, Ready }
    Phase phase = Phase.Starting;
    /// The current Start's: cancelled by whatever starts again or signs out, so no Start before it writes the window.
    CancellationTokenSource? started;
    /// The sign-in waiting for the browser, which waits until it's cancelled: a passkey can take a while.
    CancellationTokenSource? signingIn;

    /// Where the window is, and what screens opened over it, each kept as drawn so Back returns to it as it was.
    Route selection = Route.PageOf("user");
    readonly List<(Route route, FrameworkElement view)> pushed = [];
    FrameworkElement? shown;
    /// The last few places the window was, kept as drawn (and keeping up by themselves), hidden, so going back to one
    /// is at once: the latest last.
    readonly List<(Route route, FrameworkElement view)> recent = [];
    const int Recent = 4;

    public MainWindow()
    {
        Title = CustomBuild.Name;
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
        bar.Children.Add(new TextBlock { Text = CustomBuild.Name, FontSize = 12, VerticalAlignment = VerticalAlignment.Center });
        // What's dragged: all of the title bar but Back and the path, which are pressed.
        var titleBar = new Grid { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        titleBar.Children.Add(bar);
        var top = new Grid();
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        back.Click += (_, _) => Back();
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(back, "Back");
        path.ItemClicked += (_, e) => BackTo(e.Index);
        Grid.SetColumn(path, 1);
        Grid.SetColumn(titleBar, 2);
        top.Children.Add(back);
        top.Children.Add(path);
        top.Children.Add(titleBar);
        // Developer mode's </>, in the bar of every screen, beside the window's buttons (three of 46, which it keeps clear).
        Grid.SetColumn(devButton, 3);
        devButton.Margin = new Thickness(0, 0, 144, 0);
        devButton.Click += (_, _) => ToggleDock();
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(devButton, "Developer");
        ToolTipService.SetToolTip(devButton, "Developer");
        top.Children.Add(devButton);
        root.Children.Add(top);
        SetTitleBar(titleBar);
        logo = bar;
        Grid.SetRow(body, 1);
        root.Children.Add(body);
        root.RequestedTheme = Appearance.Theme;
        Content = root;
        // Windows turning dark or light while it's open: the sidebar and the title bar's buttons are drawn again; the
        // screens follow by themselves.
        root.ActualThemeChanged += (_, _) =>
        {
            Buttons();
            drawn = [];
            DrawSidebar();
            detail.Background = Look.Page;
            detail.BorderBrush = Look.CardStroke;
        };
        // The person's accent colour, changed in Windows' Settings: the sidebar's and every screen's tint.
        Tint.Changed += () => DispatcherQueue.TryEnqueue(() =>
        {
            AppScreen.Restyle();
            DrawSidebar();
            _ = System.Threading.Tasks.Task.Run(() => Widgets.Changed(WidgetCache.Of(Store.Shared)));
        });
        Buttons();
        // The screens keep up by themselves; the sidebar and the taskbar's count, with what the store reads.
        Store.Shared.Changed += DrawSidebar;
        Store.Shared.Changed += () => Taskbar(Store.Shared.IconBadge);
        // In front again after a minute or more away: what events may have missed.
        var away = DateTime.MaxValue;
        Activated += (_, e) =>
        {
            if (e.WindowActivationState == WindowActivationState.Deactivated)
            {
                away = DateTime.Now;
                return;
            }
            if (nav != null && DateTime.Now - away > TimeSpan.FromMinutes(1)) _ = Store.Shared.CaughtUp();
            away = DateTime.MaxValue;
        };
        // Windows' text size, from Settings > Accessibility, as each screen's.
        texts.TextScaleFactorChanged += (_, _) => DispatcherQueue.TryEnqueue(AppScreen.Restyle);
        // The Widgets board's widgets keep up with the window's.
        Store.Shared.WidgetsRead += () =>
        {
            var list = WidgetCache.Of(Store.Shared);
            _ = System.Threading.Tasks.Task.Run(() => Widgets.Changed(list));
        };
        Closed += (_, _) =>
        {
            started?.Cancel();
            signingIn?.Cancel();
        };
        Client.Lost += removed => DispatcherQueue.TryEnqueue(() => Lost(removed));
        _ = Start();
    }

    readonly Windows.UI.ViewManagement.UISettings texts = new();
    readonly Button back = new()
    {
        Content = new FontIcon { Glyph = "\uE72B", FontSize = 12 }, Width = 40, Height = 32, Margin = new Thickness(4, 0, 0, 0),
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderThickness = new Thickness(0), Visibility = Visibility.Collapsed,
    };
    readonly Button devButton = new()
    {
        Content = new TextBlock { Text = "</>", FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Cascadia Mono, Consolas"), FontSize = 13, FontWeight = FontWeights.SemiBold },
        Height = 32, Padding = new Thickness(10, 0, 10, 0), VerticalAlignment = VerticalAlignment.Center,
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderThickness = new Thickness(0), Visibility = Visibility.Collapsed,
    };
    readonly BreadcrumbBar path = new() { VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0), Visibility = Visibility.Collapsed };
    FrameworkElement? logo;

    /// The count on the taskbar's button, or none.
    static void Taskbar(int count)
    {
        try
        {
            var updater = Windows.UI.Notifications.BadgeUpdateManager.CreateBadgeUpdaterForApplication();
            if (count <= 0)
            {
                updater.Clear();
                return;
            }
            var badge = Windows.UI.Notifications.BadgeUpdateManager.GetTemplateContent(Windows.UI.Notifications.BadgeTemplateType.BadgeNumber);
            ((Windows.Data.Xml.Dom.XmlElement)badge.SelectSingleNode("/badge")).SetAttribute("value", count.ToString());
            updater.Update(new Windows.UI.Notifications.BadgeNotification(badge));
        }
        catch
        {
            // Run from its folder, not installed, it has no badge.
        }
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
    /// or a page at your One App network's address, as the web client has them (cloudflare/web/web.js): `/`, `/page/<scope>`,
    /// `/accounts/<scope>`, `/app/<app>/<screen>?params=&scope=`, `/settings`, `/settings/<item>?params=`, and
    /// `/vault?id=&scope=` (or `<cli>://vault?…`), a request for an item, in the Vault's sheet; `<cli>://page` and
    /// `<cli>://settings` as below. An invite link
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
                "accounts" when At(1) is { } scope => Route.Accounts(scope),
                "app" when At(1) is { } app => Route.ScreenOf(app, q["scope"], At(2) ?? "main", q["params"] ?? "{}"),
                "settings" => At(1) is { } item ? Route.OwnOf(item, q["params"] ?? "{}") : Route.Settings,
                _ => Route.PageOf("user"),
            };
        }
        else if (link.Host == "open")
        {
            var scope = q["scope"];
            opening = q["app"] is { Length: > 0 } app ? Route.ScreenOf(app, scope, q["screen"] ?? "main", q["params"] ?? "{}") : Route.PageOf(scope ?? "user");
        }
        else if (link.Host == "vault" && q["id"] is { Length: > 0 } id && q["scope"] is { Length: > 0 } s) asking = (s, id);
        // As the iPhone takes them: `<cli>://page?scope=…` (its Accounts with `folder=accounts`), `<cli>://settings[/<item>]`.
        else if (link.Host == "page" && q["scope"] is { Length: > 0 } page) opening = q["folder"] == "accounts" ? Route.Accounts(page) : Route.PageOf(page);
        else if (link.Host == "settings") opening = link.AbsolutePath.Trim('/') is { Length: > 0 } item ? Route.OwnOf(item) : Route.Settings;
        if (nav != null) Opening();
    }

    void Opening()
    {
        if (opening is { } r)
        {
            opening = null;
            // Its page, if this computer opens it; an app's screen on a page it isn't on still opens.
            if (!OnNoPage(r)) Go(r);
        }
        if (asking is { } a)
        {
            asking = null;
            _ = Present(Route.Request(a.scope, a.id));
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

    /// Whether it's a page, or a page's Accounts, that this computer wasn't let into.
    static bool OnNoPage(Route r) => r.Own && r.Screen is "pages" or "accounts" && r.Scope != "settings" && Store.Shared.PageFor(r.Scope) == null;

    public void Restyle()
    {
        root.RequestedTheme = Appearance.Theme;
        AppScreen.Restyle();
        drawn = [];
        DrawSidebar();
    }

    // MARK: Starting

    /// This computer isn't one of the network's: taken out, it's the sign-in; not yet in, it starts again at its approval
    /// screen, whatever the window showed, and comes back to the person's own page once approved.
    void Lost(bool removed)
    {
        if (removed)
        {
            Welcome(Client.Removed);
            return;
        }
        // Already starting again, or signed out.
        if (phase != Phase.Ready) return;
        Sheet.CloseAll();
        pushed.Clear();
        selection = Route.PageOf("user");
        _ = Start();
    }

    async Task Start()
    {
        started?.Cancel();
        var mine = started = new CancellationTokenSource();
        var stop = mine.Token;
        phase = Phase.Starting;
        _ = Shell.Claim(Auth.Shared.Address);
        if (Auth.Shared.Address == null || !Auth.Shared.SignedIn)
        {
            Welcome(null);
            return;
        }
        // Starting… only if it's slow, and Can't Reach Your Devices only once it's more than a blip: a second miss, or
        // 3 seconds of them.
        var starting = Centered(new StackPanel
        {
            Spacing = 12, Visibility = Visibility.Collapsed,
            Children = { new ProgressRing { IsActive = true, Width = 24, Height = 24 }, Look.Text($"Starting {CustomBuild.Name}…", 13, Look.Secondary) },
        });
        body.Content = starting;
        _ = Task.Delay(400).ContinueWith(_ => starting.Visibility = Visibility.Visible, TaskScheduler.FromCurrentSynchronizationContext());
        var since = DateTime.Now;
        for (var misses = 1; ; misses++)
        {
            try
            {
                await Join(stop);
                await Store.Shared.RefreshState();
                stop.ThrowIfCancellationRequested();
                break;
            }
            catch (OperationCanceledException) when (stop.IsCancellationRequested)
            {
                return;
            }
            catch (SignedOutException e)
            {
                if (!stop.IsCancellationRequested) Welcome(e.Message == Client.Removed ? e.Message : null);
                return;
            }
            catch (NotLetInException)
            {
                // Asked before it was let in: it joins again.
                continue;
            }
            catch (Exception e)
            {
                if (stop.IsCancellationRequested) return;
                if (misses >= 2 || DateTime.Now - since >= TimeSpan.FromSeconds(3))
                    body.Content = Look.Unavailable("Can’t Reach Your Devices", e.Message, Look.Text("Trying again…", 13, Look.Secondary));
                try { await Task.Delay(Node.RetryWait(misses), stop); }
                catch (OperationCanceledException) { return; }
            }
        }
        phase = Phase.Ready;
        if (Store.Shared.Install == null)
        {
            body.Content = Look.Unavailable("Not Set Up Yet", "Set it up on one of your computers first.");
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
        _ = Store.Shared.Listen(() => DispatcherQueue.TryEnqueue(() => { if (!stop.IsCancellationRequested) Welcome(null); }), stop);
        _ = Notes.Register();
    }

    /// This computer let in as its person's device: at once if it is, else it asks, shows its code, and waits for one of
    /// their devices (or, for someone new, their inviter's) to approve it.
    async Task Join(CancellationToken stop)
    {
        if (await Client.Connect()) return;
        stop.ThrowIfCancellationRequested();
        if (await Client.RemovedNow()) throw new SignedOutException(Client.Removed);
        var code = await Client.Ask();
        stop.ThrowIfCancellationRequested();
        phase = Phase.Joining;
        var panel = new StackPanel { Spacing = 14, MaxWidth = 460 };
        panel.Children.Add(new TextBlock { Text = "Waiting for Approval", FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center });
        panel.Children.Add(Look.Text(Auth.Shared.Invite != null ? "Whoever invited you approves it. Show them this code." : "Your other devices ask you to approve it. It shows this code there too.", 14, Look.Secondary, wrap: true));
        panel.Children.Add(new TextBlock { Text = code, FontSize = 40, FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Cascadia Mono, Consolas"), CharacterSpacing = 200, IsTextSelectionEnabled = true, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 14, 0, 0) });
        // Their browser where they're signed in, at Approve with this code.
        if (Auth.Shared.Invite == null && Auth.Shared.Address is { } address)
        {
            var inBrowser = Look.Link("Approve in Your Browser", () => Shell.Open($"{address.TrimEnd('/')}/approve#{code.Replace(" ", "")}"));
            inBrowser.HorizontalAlignment = HorizontalAlignment.Center;
            panel.Children.Add(inBrowser);
        }
        var signOut = new Button { Content = "Sign Out", HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 24, 0, 0) };
        signOut.Click += async (_, _) => await SignOut();
        panel.Children.Add(signOut);
        body.Content = Centered(panel);
        while (!await Client.Waited())
        {
            stop.ThrowIfCancellationRequested();
            await Task.Delay(2000, stop);
        }
        stop.ThrowIfCancellationRequested();
        phase = Phase.Starting;
        Auth.Shared.Invite = null;
        body.Content = Centered(new ProgressRing { IsActive = true, Width = 24, Height = 24 });
    }

    static FrameworkElement Centered(FrameworkElement el)
    {
        el.HorizontalAlignment = HorizontalAlignment.Center;
        el.VerticalAlignment = VerticalAlignment.Center;
        return el;
    }

    /// Welcome: the network's address (or an invite link) the first time, then Sign In, in the person's browser:
    /// at once when `signIn` is true, as it is after Continue.
    void Welcome(string? error, bool invited = false, bool signIn = false)
    {
        started?.Cancel();
        started = null;
        signingIn?.Cancel();
        phase = Phase.SignedOut;
        Sheet.CloseAll();
        nav = null;
        DeveloperOn(false);
        docked = false;
        Forget();
        Taskbar(0);
        var panel = new StackPanel { Spacing = 14, MaxWidth = 520, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var logo = new Image { Source = new BitmapImage(new Uri(System.IO.Path.Combine(AppContext.BaseDirectory, "Assets", "Logo.png"))), Width = 112, Height = 112, HorizontalAlignment = HorizontalAlignment.Center };
        panel.Children.Add(logo);
        panel.Children.Add(new TextBlock { Text = CustomBuild.Name, FontSize = 32, FontWeight = FontWeights.SemiBold, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords, Margin = new Thickness(0, 0, 0, 14) });
        var problem = new TextBlock { Text = error ?? "", Foreground = Look.Critical, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords, Visibility = error == null ? Visibility.Collapsed : Visibility.Visible };

        if (Auth.Shared.Address == null)
        {
            var whose = char.ToUpperInvariant(CustomBuild.Yours[0]) + CustomBuild.Yours[1..];
            var address = new TextBox { PlaceholderText = "one.example.com, or an invite link", Width = 340, HorizontalAlignment = HorizontalAlignment.Center, IsSpellCheckEnabled = false };
            // The touch keyboard's for web addresses.
            address.InputScope = new Microsoft.UI.Xaml.Input.InputScope { Names = { new Microsoft.UI.Xaml.Input.InputScopeName(Microsoft.UI.Xaml.Input.InputScopeNameValue.Url) } };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(address, $"{whose}’s address or invite link");
            panel.Children.Add(new TextBlock { Text = $"{whose}’s address, or the invite link you were sent.", FontSize = 14, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords });
            panel.Children.Add(address);
            void Go()
            {
                try
                {
                    var invite = Auth.Shared.Take(address.Text);
                    if (invite != null) Shell.Open(invite.AbsoluteUri);
                    Welcome(null, invite != null, invite == null);
                }
                catch (Exception e)
                {
                    problem.Text = e.Message;
                    problem.Visibility = Visibility.Visible;
                }
            }
            address.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) Go(); };
            var go = Look.Tinted("Continue", Go);
            go.HorizontalAlignment = HorizontalAlignment.Center;
            panel.Children.Add(go);
            panel.Children.Add(problem);
        }
        else
        {
            if (invited)
                panel.Children.Add(new TextBlock { Text = "Make your passkey in your browser, where your invite opened. Then sign in here.", FontSize = 14, HorizontalAlignment = HorizontalAlignment.Center, TextAlignment = TextAlignment.Center, TextWrapping = TextWrapping.WrapWholeWords });
            var signInButton = Look.Tinted("Sign In", () => { });
            signInButton.HorizontalAlignment = HorizontalAlignment.Center;
            signInButton.Padding = new Thickness(28, 8, 28, 9);
            signInButton.FontSize = 15;
            var waiting = new TextBlock { Text = "Finish signing in in your browser.", Foreground = Look.Secondary, HorizontalAlignment = HorizontalAlignment.Center, Visibility = Visibility.Collapsed };
            CancellationTokenSource? cancel = null;
            var stop = Look.Link("Cancel", () => cancel?.Cancel());
            stop.HorizontalAlignment = HorizontalAlignment.Center;
            stop.Visibility = Visibility.Collapsed;
            async Task Run()
            {
                signInButton.IsEnabled = false;
                waiting.Visibility = stop.Visibility = Visibility.Visible;
                problem.Visibility = Visibility.Collapsed;
                var mine = cancel = signingIn = new CancellationTokenSource();
                try
                {
                    await Auth.Shared.SignIn(mine.Token);
                }
                catch (Exception e)
                {
                    if (e is not OperationCanceledException)
                    {
                        problem.Text = e.Message;
                        problem.Visibility = Visibility.Visible;
                    }
                    signInButton.IsEnabled = true;
                    waiting.Visibility = stop.Visibility = Visibility.Collapsed;
                    return;
                }
                SetForegroundWindow(WinRT.Interop.WindowNative.GetWindowHandle(this));
                await Start();
            }
            signInButton.Click += async (_, _) => await Run();
            if (signIn) _ = Run();
            panel.Children.Add(signInButton);
            panel.Children.Add(waiting);
            panel.Children.Add(stop);
            panel.Children.Add(problem);
            panel.Children.Add(new TextBlock { Text = new Uri(Auth.Shared.Address).Host, FontSize = 12, Foreground = Look.Tertiary, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 16, 0, 0) });
            if (string.IsNullOrEmpty(CustomBuild.Address))
            {
                var other = Look.Link("Use another address…", () =>
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

    /// Signed out: nothing of the person's is left on this computer, and the Widgets board's widgets say to sign in.
    void Forget()
    {
        Store.Shared.Clear();
        AppScreen.Forget();
        _ = ForgetWebData();
        _ = Task.Run(Widgets.SignedOut);
    }

    /// What screens' web views kept (their cache, storage and service workers), cleared by a web view of the same
    /// folder, out of sight.
    async Task ForgetWebData()
    {
        var web = new WebView2 { Width = 0, Height = 0, Visibility = Visibility.Collapsed };
        root.Children.Add(web);
        try
        {
            await web.EnsureCoreWebView2Async(await App.Web);
            await web.CoreWebView2.Profile.ClearBrowsingDataAsync();
        }
        catch (Exception e)
        {
            App.Problem(e);
        }
        root.Children.Remove(web);
        web.Close();
    }

    public async Task SignOut()
    {
        started?.Cancel();
        await Client.SignOut();
        await Auth.Shared.SignOut();
        pushed.Clear();
        selection = Route.PageOf("user");
        Welcome(null);
    }

    // MARK: The window

    void ShowMain()
    {
        // A window of its own each sign-in: nothing of the last one's.
        detail = new Grid { Background = Look.Page, CornerRadius = new CornerRadius(8, 0, 0, 0), BorderBrush = Look.CardStroke, BorderThickness = new Thickness(1, 1, 0, 0) };
        pushed.Clear();
        recent.Clear();
        shown = null;
        pane = new Grid();
        pane.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        pane.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        pane.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        pane.Children.Add(detail);
        pane.SizeChanged += (_, _) => FitDock();
        dock = null;
        dockRoute = null;
        // A page this computer wasn't let into opens the first it was.
        if (OnNoPage(selection) && Store.Shared.Pages().FirstOrDefault() is { } first) selection = Route.PageOf(first.Scope);
        nav = new NavigationView
        {
            PaneDisplayMode = NavigationViewPaneDisplayMode.Left, IsSettingsVisible = false, IsPaneToggleButtonVisible = false,
            OpenPaneLength = 250, IsBackButtonVisible = NavigationViewBackButtonVisible.Collapsed, IsTitleBarAutoPaddingEnabled = false,
            Content = pane, AlwaysShowHeader = false,
        };
        nav.ItemInvoked += (_, e) =>
        {
            if (e.InvokedItemContainer?.Tag is Route r) Go(r);
            else if (e.InvokedItemContainer?.Tag is string url) Shell.Open(url);
        };
        body.Content = nav;
        drawn = [];
        DrawSidebar();
        DrawDetail();
        Opening();
        _ = ReadDeveloper();
    }

    /// Goes somewhere in place of where the window is, as a sidebar row does.
    public void Go(Route r)
    {
        if (nav == null) return;
        selection = r;
        pushed.Clear();
        shown = recent.FirstOrDefault(v => v.route == r).view;
        DrawDetail();
        DrawSidebar();
    }

    /// Opens something over where the window is, as a screen opening another does: Back returns to it.
    public void Push(Route r)
    {
        if (nav == null) return;
        pushed.Add((r, Screen(r)));
        DrawDetail();
    }

    void Back()
    {
        if (pushed.Count == 0) return;
        pushed.RemoveAt(pushed.Count - 1);
        DrawDetail();
    }

    /// Back to a part of the path: 0 is what the sidebar picked.
    void BackTo(int part)
    {
        if (part >= pushed.Count) return;
        pushed.RemoveRange(part, pushed.Count - part);
        DrawDetail();
    }

    /// A screen's `one.close`, where it was pushed: back to what's under it.
    public void Back(AppScreen screen)
    {
        // The Developer panel's `one.close` closes the dock.
        if (screen == dock)
        {
            docked = false;
            DrawDock();
            return;
        }
        if (pushed.Count > 0 && pushed[^1].view == screen.Parent) Back();
    }

    /// A screen over the window, as a sheet → what it gave `one.close` once it's gone, else null.
    public Task<JsonNode?> Present(Route r)
    {
        // A request is answered in it, or not: Cancel, not Done.
        var sheet = new Sheet(root, r, TitleOf(r), r.Own && r.Screen == "request" ? "Cancel" : "Done");
        sheet.Show();
        return sheet.Closed;
    }

    /// Shows the screen on top; those under it stay in the window, hidden, so each keeps its place.
    void DrawDetail()
    {
        var view = pushed.Count > 0 ? pushed[^1].view : shown ??= Screen(selection);
        if (shown != null)
        {
            recent.RemoveAll(v => v.view == shown || v.route == selection);
            recent.Add((selection, shown));
            if (recent.Count > Recent) recent.RemoveAt(0);
        }
        var kept = pushed.Select(p => p.view).Concat(recent.Select(v => v.view)).ToHashSet();
        foreach (var gone in detail.Children.OfType<FrameworkElement>().Where(c => !kept.Contains(c)).ToList()) detail.Children.Remove(gone);
        foreach (var child in detail.Children.OfType<FrameworkElement>()) child.Visibility = child == view ? Visibility.Visible : Visibility.Collapsed;
        if (!detail.Children.Contains(view)) detail.Children.Add(view);
        DrawPath();
        var front = pushed.Count > 0 ? pushed[^1].route : selection;
        if (front != fronted || (pushed.Count > 0 ? pushed[^1].view : shown) != frontView) since = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        (fronted, frontView) = (front, pushed.Count > 0 ? pushed[^1].view : shown);
        DrawDock();
    }

    // MARK: Developer

    /// Developer mode (Settings › Developer), on this computer alone: while it's on, the title bar's </> opens Developer
    /// (src/screen/devtools.html) docked under the screen in front, where it stays from screen to screen until closed
    /// (its `one.close`). It's told the screen's app, screen and title, and for the window's own, since when it showed.
    bool developer, docked;
    Grid? pane;
    AppScreen? dock;
    Route? dockRoute;
    Route? fronted;
    FrameworkElement? frontView;
    long since = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
    /// Developer's panel, and the edge on its top that drags it taller or shorter.
    Border? dockBox;
    DockEdge? dockEdge;
    /// Its height, kept on this computer for next time, and used on every screen; none kept is 45% of the window's, to 440.
    double? dockHeight = KeptDockHeight();
    static string DockHeightFile => System.IO.Path.Combine(CustomBuild.Folder, "developer-height");

    static double? KeptDockHeight()
    {
        try { return double.TryParse(System.IO.File.ReadAllText(DockHeightFile).Trim(), System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out var h) && h > 0 ? h : null; }
        catch { return null; }
    }

    async Task ReadDeveloper()
    {
        try { DeveloperOn(await Client.Developer(null)); } catch { }
    }

    /// Developer mode turned on or off: the button is there or not, and the panel goes with it.
    public void DeveloperOn(bool on)
    {
        developer = on;
        devButton.Visibility = on && nav != null ? Visibility.Visible : Visibility.Collapsed;
        DrawDock();
    }

    void ToggleDock()
    {
        docked = !docked;
        DrawDock();
    }

    void DrawDock()
    {
        if (pane == null) return;
        if (!developer || !docked || nav == null)
        {
            RemoveDock();
            return;
        }
        var front = pushed.Count > 0 ? pushed[^1] : (selection, shown);
        var title = front.Item2 != null && titles.TryGetValue(front.Item2, out var t) && t.Length > 0 ? t : TitleOf(front.Item1);
        var asked = Route.OwnOf("devtools", (front.Item1.Own
            ? new JsonObject { ["title"] = title, ["since"] = since }
            : new JsonObject { ["app"] = front.Item1.App, ["screen"] = front.Item1.Screen, ["title"] = title }).ToJsonString());
        if (dockRoute == asked && dockBox != null) return;
        RemoveDock();
        dockRoute = asked;
        dock = new AppScreen(asked, (_, _) => { });
        dockBox = new Border { Background = Look.Page, BorderBrush = Look.CardStroke, BorderThickness = new Thickness(1, 0, 0, 0), Child = dock };
        Grid.SetRow(dockBox, 2);
        pane.Children.Add(dockBox);
        // The edge is a strip of its own above the panel, as a splitter is, holding the panel's top line; the screens'
        // WebViews would take the pointer if it lay over them.
        var edge = dockEdge = new DockEdge { Height = 8 };
        edge.Children.Add(new Border { Height = 1, VerticalAlignment = VerticalAlignment.Bottom, Background = Look.CardStroke });
        Grid.SetRow(edge, 1);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(edge, "Resize Developer");
        double start = 0, from = 0;
        edge.PointerPressed += (_, e) =>
        {
            if (dockBox == null || pane == null || !edge.CapturePointer(e.Pointer)) return;
            (start, from) = (e.GetCurrentPoint(pane).Position.Y, dockBox.ActualHeight);
            edge.Dragging = true;
            e.Handled = true;
        };
        edge.PointerMoved += (_, e) =>
        {
            if (!edge.Dragging || pane == null) return;
            dockHeight = from + start - e.GetCurrentPoint(pane).Position.Y;
            FitDock();
        };
        void drop(Microsoft.UI.Xaml.Input.PointerRoutedEventArgs e)
        {
            if (!edge.Dragging) return;
            edge.Dragging = false;
            edge.ReleasePointerCapture(e.Pointer);
            try
            {
                System.IO.Directory.CreateDirectory(CustomBuild.Folder);
                System.IO.File.WriteAllText(DockHeightFile, dockBox?.Height.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "");
            }
            catch { }
        }
        edge.PointerReleased += (_, e) => drop(e);
        edge.PointerCanceled += (_, e) => drop(e);
        edge.PointerCaptureLost += (_, _) => edge.Dragging = false;
        pane.Children.Add(edge);
        FitDock();
    }

    void RemoveDock()
    {
        if (pane != null)
        {
            if (dockBox != null) pane.Children.Remove(dockBox);
            if (dockEdge != null) pane.Children.Remove(dockEdge);
        }
        (dock, dockBox, dockEdge, dockRoute) = (null, null, null, null);
    }

    /// Between 120 and the window's height less 120 (and the edge), whatever was kept or dragged to.
    void FitDock()
    {
        if (dockBox == null || pane == null) return;
        var most = Math.Max(120, pane.ActualHeight - dockEdge!.Height - 120);
        dockBox.Height = Math.Clamp(dockHeight ?? Math.Min(440, pane.ActualHeight * 0.45), 120, most);
    }

    /// The title bar: the window's name, or once something is pushed, Back and the path to it.
    void DrawPath()
    {
        var deep = nav != null && pushed.Count > 0;
        back.Visibility = path.Visibility = deep ? Visibility.Visible : Visibility.Collapsed;
        if (logo != null) logo.Visibility = deep ? Visibility.Collapsed : Visibility.Visible;
        if (!deep) return;
        string Named(FrameworkElement? view, Route r) => view != null && titles.TryGetValue(view, out var t) && t.Length > 0 ? t : TitleOf(r);
        var parts = new List<string> { Named(shown, selection) };
        parts.AddRange(pushed.Select(p => Named(p.view, p.route)));
        path.ItemsSource = parts;
    }

    /// What a route is called: a page's name, Accounts, Vault, a Settings item's, or an app screen's.
    static string TitleOf(Route r)
    {
        var store = Store.Shared;
        if (r.Own)
            return r.Screen switch
            {
                "pages" => r.Scope == "settings" ? "Settings" : store.PageFor(r.Scope)?.Title ?? "",
                "accounts" => "Accounts",
                "request" => "Vault",
                "devtools" => "Developer",
                _ => store.SettingsItems.FirstOrDefault(i => i.Name == r.Screen)?.Title ?? "Settings",
            };
        var app = store.App(r.App);
        return app?.Screens?.FirstOrDefault(s => s.Name == r.Screen)?.Title ?? app?.Label ?? r.App ?? "";
    }

    /// A screen under a bar with its title, which the screen may change; a page shows its own large title, so its bar
    /// has none. The path names it by what the screen says it's called, which it gives while its bar's title is empty.
    readonly System.Runtime.CompilerServices.ConditionalWeakTable<FrameworkElement, string> titles = new();

    FrameworkElement Screen(Route r)
    {
        var title = new TextBlock { FontSize = 14, FontWeight = FontWeights.SemiBold, Margin = new Thickness(20, 6, 20, 6), Height = 22 };
        var grid = new Grid();
        void Retitle(string text, string? name)
        {
            title.Text = text;
            title.Visibility = text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            titles.AddOrUpdate(grid, text.Length > 0 ? text : name ?? "");
            if (pushed.Count > 0) DrawPath();
        }
        Retitle(r.Own && r.Screen == "pages" ? "" : TitleOf(r), null);
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(title);
        var screen = new AppScreen(r, Retitle);
        Grid.SetRow(screen, 1);
        grid.Children.Add(screen);
        return grid;
    }

    /// One section per page, listing its apps and its Accounts; then Settings', listing its items with their counts.
    /// Its rows are made again only when they change (an app added, renamed or with a new icon); otherwise they stay,
    /// with only their counts and which is chosen changed, so nothing flickers.
    void DrawSidebar()
    {
        if (nav == null) return;
        var store = Store.Shared;
        var at = selection;
        // Each row: what it is and looks like, where it goes, and its count.
        var rows = new List<(string look, object tag, Func<FrameworkElement> content, int badge, bool head)>();
        static string Of(AppInfo a) => $"{a.Name}|{a.Label}|{a.Icon}|{a.Color}|{a.ImageKind}|{a.Image?.Length}|{a.Image?.GetHashCode()}";
        FrameworkElement Labeled(FrameworkElement icon, string text)
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
            icon.VerticalAlignment = VerticalAlignment.Center;
            row.Children.Add(icon);
            row.Children.Add(new TextBlock { Text = text, VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis });
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, text);
            return row;
        }
        void Head(Route route, string title)
        {
            var top = rows.Count == 0 ? 0 : 10;
            rows.Add(($"head|{title}|{top}", route, () => new TextBlock { Text = title, FontSize = 12, FontWeight = FontWeights.SemiBold, Foreground = Look.Secondary, TextWrapping = TextWrapping.WrapWholeWords, MaxLines = 3, Margin = new Thickness(0, top, 0, 0) }, 0, true));
        }
        foreach (var page in store.Pages())
        {
            Head(Route.PageOf(page.Scope), page.Title);
            // Not an app Settings lists (the App Store): the sidebar shows it once, under Settings.
            foreach (var app in store.AppsOn(page).Where(a => !store.SettingsItems.Any(i => i.App == a.Name)))
                rows.Add(($"app|{Of(app)}", Route.ScreenOf(app.Name, page.Scope), () => Labeled(Look.Tile(app, 20), app.Label), store.Badge(app.Name, page), false));
            var accounts = store.AccountsOn(page);
            rows.Add(($"accounts|{string.Join(",", accounts.Take(9).Select(Of))}", Route.Accounts(page.Scope), () => Labeled(Look.Folder(accounts, 20), "Accounts"), 0, false));
        }
        Head(Route.Settings, "Settings");
        // An item with an app (the App Store) opens that app; one with a url (Passkeys), the browser.
        foreach (var item in store.SettingsItems)
            rows.Add(($"setting|{item.Title}|{item.Icon}|{item.Color}|{item.Url}", item.Url ?? (object)(item.App is { } a ? Route.ScreenOf(a, null) : Route.OwnOf(item.Name)),
                () => Labeled(Look.Setting(item, 20), item.Title), item.Badge ?? 0, false));

        var looks = rows.Select(r => $"{r.look}|{r.tag}").ToList();
        if (!looks.SequenceEqual(drawn) || nav.MenuItems.Count != rows.Count)
        {
            nav.MenuItems.Clear();
            nav.FooterMenuItems.Clear();
            foreach (var r in rows)
            {
                var item = new NavigationViewItem { Content = r.content(), Tag = r.tag };
                if (r.tag is string) item.SelectsOnInvoked = false;
                if (r.head)
                {
                    item.MinHeight = 28;
                    Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, ((TextBlock)item.Content).Text);
                }
                nav.MenuItems.Add(item);
            }
            drawn = looks;
        }
        object? selected = null;
        for (var i = 0; i < rows.Count; i++)
        {
            var item = (NavigationViewItem)nav.MenuItems[i];
            var badge = rows[i].badge;
            if (badge <= 0) item.InfoBadge = null;
            else if (item.InfoBadge is { } b) b.Value = badge;
            else item.InfoBadge = new InfoBadge { Value = badge };
            if (rows[i].tag is Route route && route == at) selected = item;
            if (rows[i].head && item.Content is TextBlock head) head.Foreground = Look.Secondary;
        }
        if (nav.SelectedItem != selected) nav.SelectedItem = selected;
    }

    /// The sidebar's rows as last made (DrawSidebar); emptied to have them made again, as the theme changes.
    List<string> drawn = [];
}

/// The thin strip on the Developer panel's top edge: the resize cursor, and whether it's being dragged.
sealed class DockEdge : Grid
{
    public bool Dragging;

    public DockEdge()
    {
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        ProtectedCursor = Microsoft.UI.Input.InputSystemCursor.Create(Microsoft.UI.Input.InputSystemCursorShape.SizeNorthSouth);
    }
}
