// Notifications. The install's push Worker (kernel/push/) carries each note, sealed to this computer's device, through
// WNS as a raw notification, its body the sealed bytes, which wakes this app (the Windows App SDK's
// PushNotificationManager, by the custom build's Microsoft Entra app: CustomBuild.WnsApp and WnsObject, empty for none,
// and then nothing registers). The app opens the note with the device's key (client/ffi's OpenNote), asking nothing of
// the network, and shows it as a toast; clicking one opens the app screen it names. A device asking its person to join
// shows Approve and Not Me, answered from here as Settings › Devices answers them, with no window.
// Started for one of these (a push, a toast's button), the program does it and ends (Program.cs).
using System;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Microsoft.Windows.AppLifecycle;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;
using Microsoft.Windows.PushNotifications;

namespace One;

public static class Notes
{
    static bool Pushes => CustomBuild.WnsApp.Length > 0 && CustomBuild.WnsObject.Length > 0 && PushNotificationManager.IsSupported();

    /// As the program starts, before it asks how it was started: toasts' clicks and pushes come here while it runs.
    public static void Start()
    {
        try
        {
            AppNotificationManager.Default.NotificationInvoked += (_, e) => Clicked(e);
            AppNotificationManager.Default.Register();
            if (!Pushes) return;
            PushNotificationManager.Default.PushReceived += (_, e) => Received(e.Payload);
            PushNotificationManager.Default.Register();
        }
        catch (Exception e)
        {
            App.Problem(e);
        }
    }

    /// Started for a push, or for a join's Approve or Not Me: done here, with no window. → whether it was.
    public static bool Background(AppActivationArguments activated)
    {
        switch (activated.Kind)
        {
            case ExtendedActivationKind.Push when activated.Data is PushNotificationReceivedEventArgs push:
                var deferral = push.GetDeferral();
                try { Received(push.Payload); }
                finally { deferral.Complete(); }
                return true;
            case ExtendedActivationKind.AppNotification when activated.Data is AppNotificationActivatedEventArgs clicked && clicked.Arguments.ContainsKey("join"):
                Answer(clicked).Wait();
                return true;
            default:
                return false;
        }
    }

    /// The link a toast opens, when it was clicked to start the app: what Program.Link takes it by.
    public static Uri? Link(AppNotificationActivatedEventArgs clicked) =>
        clicked.Arguments.TryGetValue("open", out var link) && Uri.TryCreate(link, UriKind.Absolute, out var url) ? url : null;

    /// This computer's WNS channel, registered with the push Worker as its device's: each time it starts let in, since
    /// a channel lasts 30 days.
    public static async Task Register()
    {
        if (!Pushes) return;
        try
        {
            var made = await PushNotificationManager.Default.CreateChannelAsync(Guid.Parse(CustomBuild.WnsObject));
            if (made.Status != PushNotificationChannelStatus.CompletedSuccess) throw made.ExtendedError ?? new Exception($"WNS gave no channel: {made.Status}");
            await Client.RegisterPush("wns", made.Channel.Uri.ToString());
        }
        catch (Exception e)
        {
            App.Problem(e);
        }
    }

    /// A push came: its note opened with whichever of this computer's device keys it was sealed to, and shown. One that
    /// opens with none (sealed to a device signed out of here) shows nothing.
    static void Received(byte[] payload)
    {
        // As every push service but WNS carries it: base64url.
        var pushed = Convert.ToBase64String(payload).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        var note = Client.Secrets().Select(secret =>
        {
            try { return uniffi.one_client_ffi.OneClientFfiMethods.OpenNote(secret, pushed); }
            catch { return null; }
        }).FirstOrDefault(n => n != null);
        if (note != null) Show(note.Id, note.Title, note.Body, Opens(note), Join(note));
    }

    /// The code of the device a note's action asks to let in (`{ "join": "<code>" }`), if it does.
    static string? Join(uniffi.one_client_ffi.Note note)
    {
        try { return note.Action is { } a && JsonNode.Parse(a)?["join"]?.GetValue<string>() is { Length: > 0 } code ? code : null; }
        catch { return null; }
    }

    /// Where clicking a note goes: a join, to Settings › Devices, where it waits too; anything else, the app screen it
    /// names, or the app.
    static string Opens(uniffi.one_client_ffi.Note note)
    {
        if (Join(note) != null) return $"{CustomBuild.Cli}://settings/devices";
        // Settings' own screens (Settings › Devices, where a device that joined is) are this app's, not an app's.
        if (note.App == "settings") return $"{CustomBuild.Cli}://settings/{Uri.EscapeDataString(note.Screen)}";
        if (note.App.Length == 0) return $"{CustomBuild.Cli}://open";
        var screen = note.Screen.Length > 0 ? note.Screen : "main";
        var link = $"{CustomBuild.Cli}://open?app={Uri.EscapeDataString(note.App)}&screen={Uri.EscapeDataString(screen)}";
        return note.Params == "{}" ? link : $"{link}&params={Uri.EscapeDataString(note.Params)}";
    }

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

    /// A toast clicked while the app runs: a join's answer, or what it opens, in the window.
    static void Clicked(AppNotificationActivatedEventArgs e)
    {
        if (e.Arguments.ContainsKey("join")) _ = Answer(e);
        else App.Opened(Link(e));
    }

    /// Approve or Not Me, as this device: if it can't be done, a toast says why, with the same two answers.
    static async Task Answer(AppNotificationActivatedEventArgs e)
    {
        var a = e.Arguments;
        var approve = a.TryGetValue("answer", out var answer) && answer == "approve";
        var code = a["join"];
        var id = a.TryGetValue("id", out var i) ? i : "";
        try
        {
            if (id.Length > 0) await AppNotificationManager.Default.RemoveByTagAsync(id);
            var body = approve ? new JsonObject { ["code"] = code, ["confirm"] = true } : new JsonObject { ["code"] = code };
            await Node.Ask(System.Net.Http.HttpMethod.Post, approve ? "/v1/devices/approve" : "/v1/devices/refuse", body: body);
        }
        catch (Exception ex)
        {
            Show(id, a.TryGetValue("title", out var t) ? t : "", "", a.TryGetValue("open", out var o) ? o : $"{CustomBuild.Cli}://settings/devices", code,
                (approve ? "Couldn’t approve it: " : "Couldn’t turn it down: ") + ex.Message);
        }
    }
}
