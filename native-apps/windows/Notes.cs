// Notifications. The network's notifications Worker (notifications/worker/) carries each note, sealed to this computer's device,
// through WNS as a raw notification, its body the sealed bytes, which starts this helper (the Windows App SDK's
// PushNotificationManager, by the custom build's Microsoft Entra app: CustomBuild.WnsApp and WnsObject, empty for none,
// and then nothing registers). The app opens the note with the device's key (open-note), asking nothing of the
// network, and this shows it as a toast; clicking one opens the app screen it names. A device asking its user to
// join shows Approve and Not Me, answered by the app (join) as Settings › Devices answers them, with no window.
using System;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.Windows.AppLifecycle;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;
using Microsoft.Windows.PushNotifications;

namespace One;

public static class Notes
{
    static bool Available => CustomBuild.WnsApp.Length > 0 && CustomBuild.WnsObject.Length > 0 && PushNotificationManager.IsSupported();

    /// As it starts, before it asks how it was started: toasts' clicks and notifications come here while it runs.
    public static void Start()
    {
        try
        {
            AppNotificationManager.Default.NotificationInvoked += (_, e) => Clicked(e).Wait();
            AppNotificationManager.Default.Register();
            if (!Available) return;
            PushNotificationManager.Default.PushReceived += (_, e) => Received(e.Payload).Wait();
            PushNotificationManager.Default.Register();
        }
        catch (Exception e)
        {
            App.Problem(e);
        }
    }

    /// Started for a notification, or for a toast's click or button: done here.
    public static void Started(AppActivationArguments activated)
    {
        switch (activated.Kind)
        {
            case ExtendedActivationKind.Push when activated.Data is PushNotificationReceivedEventArgs push:
                var deferral = push.GetDeferral();
                try { Received(push.Payload).Wait(); }
                finally { deferral.Complete(); }
                break;
            case ExtendedActivationKind.AppNotification when activated.Data is AppNotificationActivatedEventArgs clicked:
                Clicked(clicked).Wait();
                break;
        }
    }

    /// This computer's WNS channel, registered with the notifications Worker as its device's by the app: each time the app
    /// starts let in, since a channel lasts 30 days.
    public static async Task Register()
    {
        if (!Available) return;
        try
        {
            var made = await PushNotificationManager.Default.CreateChannelAsync(Guid.Parse(CustomBuild.WnsObject));
            if (made.Status != PushNotificationChannelStatus.CompletedSuccess) throw made.ExtendedError ?? new Exception($"WNS gave no channel: {made.Status}");
            await App.Ask("register-notifications", new JsonObject { ["via"] = "wns", ["address"] = made.Channel.Uri.ToString() });
        }
        catch (Exception e)
        {
            App.Problem(e);
        }
    }

    /// A notification came: its note opened by the app with this device's key, and shown. One it can't open (sealed to a
    /// device signed out of here) shows nothing.
    static async Task Received(byte[] payload)
    {
        // As every push service but WNS carries it: base64url.
        var carried = Convert.ToBase64String(payload).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        JsonNode? note;
        try { note = await App.Ask("open-note", new JsonObject { ["n"] = carried }); }
        catch (Exception e)
        {
            App.Problem(e);
            return;
        }
        if (note == null) return;
        Show(Str(note, "id"), Str(note, "title"), Str(note, "body"), Opens(note), Join(note));
    }

    static string Str(JsonNode note, string key) => note[key] is JsonValue v && v.TryGetValue<string>(out var s) ? s : "";

    /// The code of the device a note asks to let in, if it does.
    static string? Join(JsonNode note) => Str(note, "join") is { Length: > 0 } code ? code : null;

    /// Where clicking a note goes, as the app worked it out; with none, the app.
    static string Opens(JsonNode note) => Str(note, "link") is { Length: > 0 } link ? link : $"{CustomBuild.Cli}://open";

    static void Show(string id, string title, string body, string open, string? join, string? problem = null)
    {
        var toast = new AppNotificationBuilder().AddArgument("open", open).AddText(title);
        if ((problem ?? body).Length > 0) toast.AddText(problem ?? body);
        if (join != null)
        {
            // What it is is kept on its buttons, so a problem answering it can say which.
            AppNotificationButton Answer(string label, string answer) => new AppNotificationButton(label)
                .AddArgument("join", join).AddArgument("answer", answer).AddArgument("id", id).AddArgument("title", title).AddArgument("open", open);
            toast.AddButton(Answer("Not Me", "refuse")).AddButton(Answer("Approve", "approve"));
        }
        var shown = toast.BuildNotification();
        if (id.Length > 0) shown.Tag = id;
        AppNotificationManager.Default.Show(shown);
    }

    /// A toast clicked: a join's answer, with no window, or what it opens, in the app's window.
    static async Task Clicked(AppNotificationActivatedEventArgs e)
    {
        var a = e.Arguments;
        if (!a.TryGetValue("join", out var code))
        {
            if (a.TryGetValue("open", out var link)) App.Open(link);
            return;
        }
        // Approve or Not Me, as this device: if it can't be done, a toast says why, with the same two answers.
        var approve = a.TryGetValue("answer", out var answer) && answer == "approve";
        var id = a.TryGetValue("id", out var i) ? i : "";
        try
        {
            if (id.Length > 0) await AppNotificationManager.Default.RemoveByTagAsync(id);
            await App.Ask("join", new JsonObject { ["code"] = code, ["approve"] = approve });
        }
        catch (Exception ex)
        {
            Show(id, a.TryGetValue("title", out var t) ? t : "", "", a.TryGetValue("open", out var o) ? o : $"{CustomBuild.Cli}://settings/devices", code,
                (approve ? "Couldn’t approve it: " : "Couldn’t turn it down: ") + ex.Message);
        }
    }
}
