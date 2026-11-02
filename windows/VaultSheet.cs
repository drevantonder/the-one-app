// A request for an item an app may use, as the Apple apps' and the web's Vault sheet: who asks and why, then the
// person's items that have the fields it wants, to pick one, or a new one, whose values go to the node and nowhere
// else. The asker never sees a secret. Answered somewhere else (another device, an agent's CLI), the sheet goes.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace One;

public static class VaultSheet
{
    /// A field's name as a person reads it: "api_key" → "API key".
    public static string FieldLabel(string name)
    {
        var words = name.Replace('_', ' ').Replace('-', ' ').Split(' ', StringSplitOptions.RemoveEmptyEntries);
        return string.Join(" ", words.Select((w, i) =>
            new[] { "api", "url", "id", "ssh", "pem", "aws", "s3", "ip" }.Contains(w.ToLowerInvariant()) ? w.ToUpperInvariant()
            : i > 0 ? w.ToLowerInvariant() : char.ToUpperInvariant(w[0]) + w[1..].ToLowerInvariant()));
    }

    public static async Task Show(XamlRoot root, string scope, string id)
    {
        VaultRequest? request;
        try
        {
            request = (await Node.VaultRequests(scope, id)).FirstOrDefault();
        }
        catch (Exception failed)
        {
            await new ContentDialog { Title = "Can’t Find This Request", Content = failed.Message, CloseButtonText = "OK", XamlRoot = root }.ShowAsync();
            return;
        }
        if (request == null)
        {
            await new ContentDialog { Title = "Can’t Find This Request", Content = "It isn’t for you, or it’s from another install.", CloseButtonText = "OK", XamlRoot = root }.ShowAsync();
            return;
        }
        var app = Store.Shared.App(request.App);
        var dialog = new ContentDialog { Title = "Vault", XamlRoot = root, CloseButtonText = "Cancel", DefaultButton = ContentDialogButton.Primary };
        var body = new StackPanel { Spacing = 14, Width = 420 };
        var who = new Grid { ColumnSpacing = 12 };
        who.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        who.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        who.Children.Add(Look.Tile(app, 44, request.App));
        var asks = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        asks.Children.Add(Look.Text(app != null ? $"{app.Label} wants {request.What}" : request.What, 15, semibold: true, lines: 2));
        if (request.Why is { Length: > 0 } why) asks.Children.Add(Look.Text(why, 13, Look.Secondary, wrap: true));
        asks.Children.Add(Look.Text(string.IsNullOrEmpty(request.Agent) ? request.ScopeName : $"{request.Asker} · {request.ScopeName}", 12, Look.Tertiary));
        Grid.SetColumn(asks, 1);
        who.Children.Add(asks);
        body.Children.Add(who);
        dialog.Content = body;

        if (request.State != "waiting")
        {
            body.Children.Add(Look.Text(request.State == "set" ? "Allowed." : request.State == "declined" ? "Declined." : $"This request expired. Ask {request.Asker} to ask again.", 14, Look.Secondary, wrap: true));
            dialog.CloseButtonText = "OK";
            await dialog.ShowAsync();
            return;
        }

        // The items that fit, then a new one.
        var choices = new RadioButtons();
        foreach (var match in request.Matches ?? []) choices.Items.Add(new RadioButton { Content = match.Type is { Length: > 0 } t && t != match.Title ? $"{match.Title} · {t}" : match.Title, Tag = match.Id });
        choices.Items.Add(new RadioButton { Content = "New Item", Tag = "" });
        choices.SelectedIndex = 0;
        body.Children.Add(choices);

        var fields = new StackPanel { Spacing = 8 };
        var title = new TextBox { Header = "Title", Text = request.What };
        fields.Children.Add(title);
        var values = new Dictionary<string, Func<string>>();
        foreach (var f in request.Wanted)
        {
            if (f.Secret)
            {
                var box = new PasswordBox { Header = FieldLabel(f.Name) };
                box.PasswordChanged += (_, _) => Ready();
                values[f.Name] = () => box.Password;
                fields.Children.Add(box);
            }
            else
            {
                var box = new TextBox { Header = FieldLabel(f.Name), IsSpellCheckEnabled = false };
                box.TextChanged += (_, _) => Ready();
                values[f.Name] = () => box.Text;
                fields.Children.Add(box);
            }
        }
        if (request.Hosts.Count > 0) fields.Children.Add(Look.Text($"Sent only to {string.Join(", ", request.Hosts)}", 12, Look.Secondary, wrap: true));
        body.Children.Add(fields);
        var error = new TextBlock { Foreground = Look.Toned("bad"), TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
        body.Children.Add(error);

        string? Picked() => (choices.SelectedItem as RadioButton)?.Tag as string is { Length: > 0 } item ? item : null;
        void Ready()
        {
            fields.Visibility = Picked() == null ? Visibility.Visible : Visibility.Collapsed;
            dialog.IsPrimaryButtonEnabled = Picked() != null || values.Values.All(v => v().Length > 0);
        }
        choices.SelectionChanged += (_, _) => Ready();
        dialog.PrimaryButtonText = "Allow";
        dialog.SecondaryButtonText = "Decline";
        Ready();

        async Task<bool> Answer(bool declined)
        {
            var answer = new JsonObject { ["scope"] = request.Scope, ["id"] = request.Id };
            if (declined) answer["declined"] = true;
            else if (Picked() is { } item) answer["item"] = item;
            else
            {
                if (title.Text.Trim() is { Length: > 0 } t) answer["title"] = t;
                var given = new JsonObject();
                foreach (var (name, value) in values) given[name] = value();
                answer["fields"] = given;
            }
            try
            {
                await Node.Answer(answer);
                _ = Store.Shared.RefreshState();
                return true;
            }
            catch (Exception e)
            {
                error.Text = e.Message;
                error.Visibility = Visibility.Visible;
                return false;
            }
        }
        dialog.PrimaryButtonClick += async (_, e) =>
        {
            var deferral = e.GetDeferral();
            e.Cancel = !await Answer(false);
            deferral.Complete();
        };
        dialog.SecondaryButtonClick += async (_, e) =>
        {
            var deferral = e.GetDeferral();
            e.Cancel = !await Answer(true);
            deferral.Complete();
        };

        // Answered somewhere else, it goes.
        using var open = new CancellationTokenSource();
        _ = Task.Run(async () =>
        {
            while (!open.IsCancellationRequested)
            {
                await Task.Delay(2100);
                try
                {
                    var now = (await Node.VaultRequests(scope, id)).FirstOrDefault();
                    if (now != null && now.State != "waiting" && !open.IsCancellationRequested) dialog.DispatcherQueue.TryEnqueue(() => dialog.Hide());
                }
                catch { }
            }
        });
        await dialog.ShowAsync();
        open.Cancel();
    }

    /// The requests waiting for this person, each opening in the sheet.
    public static async Task Requests(XamlRoot root)
    {
        var list = (await Node.VaultRequests()).Where(r => r.State == "waiting").ToList();
        var dialog = new ContentDialog { Title = "Requests", CloseButtonText = "Done", XamlRoot = root };
        VaultRequest? chosen = null;
        if (list.Count == 0) dialog.Content = Look.Text("Nothing waits for you.", 14, Look.Secondary);
        else
        {
            var rows = list.Select(r =>
            {
                var app = Store.Shared.App(r.App);
                return Look.Row(Look.Tile(app, 30, r.App), $"{app?.Label ?? r.App} wants {r.What}", $"{r.Asker} · {r.ScopeName}", click: () => { chosen = r; dialog.Hide(); }, chevron: true);
            }).ToArray();
            dialog.Content = new ScrollViewer { Content = Look.Section(rows: rows), MaxHeight = 480, Width = 440 };
        }
        await dialog.ShowAsync();
        if (chosen != null) await Show(root, chosen.Scope, chosen.Id);
    }
}
