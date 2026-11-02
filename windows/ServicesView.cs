// A page's Services, as the Apple apps' and the web's: a section per app that keeps accounts, its accounts each with a
// dot saying whether it's signed in, then Add Account; then the rest, a row each opening its page in the App Store;
// then Add a Service.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Shapes;

namespace One;

public sealed class ServicesView : ContentControl
{
    readonly Page page;
    readonly MainWindow window;
    /// Each app's accounts, listed once: a list, or what went wrong listing them.
    static readonly Dictionary<string, object> accounts = [];

    public ServicesView(Page page, MainWindow window)
    {
        this.page = page;
        this.window = window;
        Draw();
    }

    /// An app's change (a sign-in finished on its settings screen) lists its accounts again.
    public static void Stale(IEnumerable<string>? apps)
    {
        foreach (var a in apps?.ToList() ?? accounts.Keys.ToList()) accounts.Remove(a);
    }

    void Draw()
    {
        var store = Store.Shared;
        var all = store.ServicesOn(page);
        var withAccounts = all.Where(a => a.HasAccounts).ToList();
        var others = all.Except(withAccounts).ToList();
        var panel = new StackPanel { Spacing = 22 };
        panel.Children.Add(Look.Header("Services", page.Whose));
        foreach (var app in withAccounts)
        {
            var rows = new List<FrameworkElement>();
            var list = accounts.GetValueOrDefault(app.Name);
            if (list == null)
            {
                rows.Add(Look.Row(new ProgressRing { IsActive = true, Width = 16, Height = 16 }, "Loading…", titleBrush: Look.Secondary));
                _ = List(app);
            }
            else if (list is string error) rows.Add(Look.Row(null, error, titleBrush: Look.Toned("warn")));
            else if (list is JsonArray { Count: 0 }) rows.Add(Look.Row(null, "No accounts yet", titleBrush: Look.Secondary));
            else
            {
                foreach (var x in (JsonArray)list)
                {
                    var ok = x?["ok"]?.GetValue<bool>() == true;
                    var name = x?["account"]?.ToString() ?? "";
                    var dot = new Ellipse { Width = 8, Height = 8, Fill = ok ? Look.Toned("good") : Look.Toned("warn") };
                    Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(dot, ok ? "Signed in" : "Needs signing in again");
                    var signIn = !ok && store.SettingsRoute(app, page, true) is { } to ? Look.Plain("Sign In", () => window.Go(to)) : null;
                    rows.Add(Look.Row(dot, name, ok ? null : x?["why"]?.ToString() ?? "Needs signing in again", signIn));
                }
            }
            if (store.SettingsRoute(app, page, true) is { } add)
                rows.Add(Look.Row(new FontIcon { Glyph = "", FontFamily = Look.Symbols, FontSize = 14, Foreground = Look.AccentBrush }, "Add Account", click: () => _ = AddAccount(app, add), titleBrush: Look.AccentBrush));
            var head = new Grid { ColumnSpacing = 10 };
            head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            head.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            head.Children.Add(Look.Tile(app, 28));
            var title = Look.Text(app.Label, 15, semibold: true);
            title.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(title, 1);
            head.Children.Add(title);
            var actions = PageView.Actions(app, page, window);
            if (actions.Count > 0)
            {
                var more = new Button { Content = new FontIcon { Glyph = "", FontFamily = Look.Symbols, FontSize = 14 }, Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderThickness = new Thickness(0), Flyout = PageView.Menu(actions) };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(more, $"More for {app.Label}");
                Grid.SetColumn(more, 2);
                head.Children.Add(more);
            }
            var section = new StackPanel { Spacing = 8 };
            section.Children.Add(head);
            section.Children.Add(Look.Section(rows: rows.ToArray()));
            panel.Children.Add(section);
        }
        if (others.Count > 0)
        {
            panel.Children.Add(Look.Section(rows: others.Select(a =>
            {
                var to = store.StoreRoute(a) ?? store.SettingsRoute(a, page);
                return Look.Row(Look.Tile(a, 28), a.Label, a.Description, click: to == null ? null : () => window.Go(to), chevron: to != null);
            }).ToArray()));
        }
        if (all.Count == 0) panel.Children.Add(Look.Text("No services yet.", 14, Look.Secondary));
        if (store.CanArrange(page.Scope) && store.ServiceStoreRoute() is { } shop)
            panel.Children.Add(Look.Section(rows: Look.Row(new FontIcon { Glyph = "", FontFamily = Look.Symbols, FontSize = 14, Foreground = Look.AccentBrush }, "Add a Service", click: () => window.Go(shop), titleBrush: Look.AccentBrush)));
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        VerticalContentAlignment = VerticalAlignment.Stretch;
        Content = Look.Column(panel);
    }

    async Task List(AppInfo app)
    {
        object out_;
        try
        {
            out_ = await Node.Call(app.Name, "accounts") is JsonArray list ? list : $"{app.Label} doesn’t list its accounts yet.";
        }
        catch (Exception e)
        {
            out_ = e.Message;
        }
        accounts[app.Name] = out_;
        if (IsLoaded) Draw();
    }

    /// Add Account: the person's items in the Vault this app wants and doesn't use yet, to switch it on for one; with
    /// none, its sign-in.
    async Task AddAccount(AppInfo app, Route signIn)
    {
        var there = new List<VaultItem>();
        if (app.KeepsAccounts)
        {
            try { there = (await Node.Vault()).Where(i => i.WantedBy?.Contains(app.Name) == true && !i.Apps.Contains(app.Name)).ToList(); } catch { }
        }
        if (there.Count == 0)
        {
            window.Go(signIn);
            return;
        }
        var dialog = new ContentDialog { Title = "Add Account", CloseButtonText = "Cancel", XamlRoot = XamlRoot };
        VaultItem? chosen = null;
        var other = false;
        var rows = there.Select(i => Look.Row(Look.Tile(null, i.Title, null, i.Image, "icon", 30), i.Title, i.Type is { } t && t != i.Title ? t : null, click: () => { chosen = i; dialog.Hide(); })).ToList();
        var panel = new StackPanel { Spacing = 12, Width = 400 };
        panel.Children.Add(Look.Section(rows: rows.ToArray()));
        panel.Children.Add(Look.Section(rows: Look.Row(null, "Sign In to Another Account", click: () => { other = true; dialog.Hide(); }, titleBrush: Look.AccentBrush)));
        dialog.Content = panel;
        await dialog.ShowAsync();
        if (other) window.Go(signIn);
        if (chosen == null) return;
        try
        {
            await Node.Share(chosen, chosen.Apps.Append(app.Name));
        }
        catch (Exception e)
        {
            await window.Alert(e.Message);
        }
        accounts.Remove(app.Name);
        Draw();
    }
}
