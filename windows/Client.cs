// This computer as a device of its person (client/ffi, the library every native app shares): its own key, made here
// and kept in Windows' Credential Locker, one for each person at each address. It gets in when one of their devices
// approves the code it shows; then every request reaches their install's nodes over iroh, as this device, with the
// keys each needs. Signing in only says who's asking.
using System;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using uniffi.one_client_ffi;
using Windows.Security.Credentials;

namespace One;

public static class Client
{
    static ThisDevice? device;
    static string? deviceFor;
    static string? deviceName;
    static readonly SemaphoreSlim gate = new(1, 1);

    /// The Credential Locker's name for this app's device keys.
    static string Locker => $"{CustomBuild.Name} device";

    /// The computer this app is on, which signing in here again replaces its sign-in under: Windows' own id for it,
    /// which outlives reinstalling, hashed.
    static string? OnDevice()
    {
        using var key = Microsoft.Win32.Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Cryptography");
        return key?.GetValue("MachineGuid") is string id ? Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes($"this device {id}"))).ToLowerInvariant() : null;
    }

    /// This computer's device for whoever's signed in, at their address.
    static async Task<ThisDevice> Current()
    {
        var token = await Auth.Shared.Token();
        var address = Auth.Shared.Address ?? throw new SignedOutException("Sign in again.");
        var id = $"{address} {Subject(token)}";
        await gate.WaitAsync();
        try
        {
            if (device != null && deviceFor == id) return device;
            var name = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(id)))[..16].ToLowerInvariant();
            var made = await Off(() => new ThisDevice(Secret(name), address, Path.Combine(CustomBuild.Folder, "client", name)));
            if (OnDevice() is { } on) made.On(on);
            var old = device;
            if (old != null) await Off(() => { old.Disconnect(); return 0; });
            (device, deviceFor, deviceName) = (made, id, name);
            return made;
        }
        finally
        {
            gate.Release();
        }
    }

    /// Whether it's let in yet: it reaches a node as itself.
    public static async Task<bool> Connect()
    {
        var d = await Current();
        return await Off(d.Connect);
    }

    /// Asks to be let in, as the person signed in → the code it shows.
    public static async Task<string> Ask()
    {
        var d = await Current();
        var token = await Auth.Shared.Token();
        await Off(() => { d.Ask(token, Environment.MachineName, Auth.Shared.Invite); return 0; });
        return d.Code();
    }

    /// Whether it's been let in, asking again if its request waited too long.
    public static async Task<bool> Waited()
    {
        var d = await Current();
        var token = await Auth.Shared.Token();
        return await Off(() => d.Wait(token));
    }

    /// Has pushes reach this computer at `address` (its WNS channel), with the push service `via`.
    public static async Task RegisterPush(string via, string address)
    {
        var d = await Current();
        await Off(() => { d.RegisterPush(via, address, "", ""); return 0; });
    }

    /// Every device key kept here, for a push's note, which opens with the one it was sealed to.
    public static byte[][] Secrets()
    {
        try
        {
            return new PasswordVault().FindAllByResource(Locker).Select(c =>
            {
                c.RetrievePassword();
                return Convert.FromBase64String(c.Password);
            }).Where(s => s.Length == 32).ToArray();
        }
        catch
        {
            return [];
        }
    }

    /// Developer mode, on this computer alone (Settings › Developer): whether it's on, first turning it on or off.
    public static async Task<bool> Developer(bool? on)
    {
        var d = await Current();
        return await Off(() =>
        {
            if (on is bool value) d.SetDeveloper(value);
            return d.Developer();
        });
    }

    /// In developer mode, its latest requests' traces, as JSON, for a screen's `traces`; null out of it.
    public static async Task<string?> Traces()
    {
        var d = await Current();
        return await Off(d.Traces);
    }

    /// A request to the person's nodes' API (`/v1/…`), its body JSON, or none.
    internal static async Task<Response> Fetch(string method, string path, byte[]? body = null, string? accept = null)
    {
        var d = await Current();
        var headers = new[] { body != null ? new Header("content-type", "application/json") : null, accept != null ? new Header("accept", accept) : null }.OfType<Header>().ToArray();
        try
        {
            return await Off(() => d.Fetch(method, path, headers, body ?? []));
        }
        catch (NotLetInException)
        {
            var removed = await Off(d.Removed);
            // Taken out, its key and sign-in go; never in, it asks again: either way the window says so, not a screen.
            if (removed) await TakenOut();
            Lost?.Invoke(removed);
            throw removed ? new SignedOutException(Removed) : new NotLetInException();
        }
    }

    public const string Removed = "This computer was removed. Sign in again.";

    /// This computer isn't one of the install's: taken out (true), or not yet in (false). The window starts again at its
    /// sign-in, or its approval screen.
    public static event Action<bool>? Lost;

    /// Its key goes with its sign-in: a key the install removed can't be let in again, so the next sign-in makes a new one.
    static async Task TakenOut()
    {
        if (deviceName is { } name)
        {
            try { var vault = new PasswordVault(); vault.Remove(vault.Retrieve(Locker, name)); } catch { }
        }
        await Close();
    }

    /// Whether it was in, and the install took it out, which ends its sign-in too.
    public static async Task<bool> RemovedNow()
    {
        var d = await Current();
        var removed = await Off(d.Removed);
        if (removed) await TakenOut();
        return removed;
    }

    /// Signs out: this computer takes itself out of its person's devices (it asks their node to), then its key goes, so
    /// signing in again makes a new device that replaces it, never a second. Asked before the sign-in is revoked.
    public static async Task SignOut()
    {
        var d = device;
        if (d != null) await Off(() => { d.SignOut(); return 0; });
        await TakenOut();
    }

    /// Its connections close. Its key stays.
    public static async Task Close()
    {
        var d = device;
        (device, deviceFor) = (null, null);
        if (d != null) await Off(() => { d.Disconnect(); return 0; });
    }

    /// What an app's call answered last for `input` (JSON), if its tool only reads (client/ffi's `kept`), from this
    /// computer alone: its answer's JSON, and when (ms); null before anyone's signed in.
    internal static async Task<(byte[] Answer, long At)?> Kept(string app, string op, byte[] input)
    {
        try
        {
            var d = device ?? await Current();
            var kept = await Off(() => d.Kept(app, op, input));
            return kept == null ? null : (kept.Answer, kept.At);
        }
        catch
        {
            return null;
        }
    }

    /// Runs a call into the library, which waits for its answer, off the caller's thread; its failure in a person's words.
    internal static Task<T> Off<T>(Func<T> work) => Task.Run(() =>
    {
        try { return work(); }
        catch (OneException.Failed e) { throw new NodeException(e.reason); }
        catch (OneException.NotLetIn) { throw new NotLetInException(); }
    });

    /// The key kept for `name`, made the first time.
    static byte[] Secret(string name)
    {
        var vault = new PasswordVault();
        try
        {
            var kept = vault.Retrieve(Locker, name);
            kept.RetrievePassword();
            var secret = Convert.FromBase64String(kept.Password);
            if (secret.Length == 32) return secret;
        }
        catch { }
        var made = OneClientFfiMethods.NewDeviceSecret();
        vault.Add(new PasswordCredential(Locker, name, Convert.ToBase64String(made)));
        return made;
    }

    /// Who an access token is for: its `sub`.
    static string Subject(string token)
    {
        try
        {
            var part = token.Split('.')[1].Replace('-', '+').Replace('_', '/');
            part += new string('=', (4 - part.Length % 4) % 4);
            return JsonNode.Parse(Convert.FromBase64String(part))?["sub"]?.GetValue<string>() ?? "";
        }
        catch { return ""; }
    }
}
