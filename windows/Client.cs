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
    static readonly SemaphoreSlim gate = new(1, 1);

    /// The Credential Locker's name for this app's device keys.
    static string Locker => $"{WhiteLabel.Name} device";

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
            var made = await Off(() => new ThisDevice(Secret(name), address, Path.Combine(WhiteLabel.Folder, "client", name)));
            var old = device;
            if (old != null) await Off(() => { old.Disconnect(); return 0; });
            (device, deviceFor) = (made, id);
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

    /// A request to the person's nodes' API (`/v1/…`), its body JSON, or none.
    internal static async Task<Response> Fetch(string method, string path, byte[]? body = null, string? accept = null)
    {
        var d = await Current();
        var headers = new[] { body != null ? new Header("content-type", "application/json") : null, accept != null ? new Header("accept", accept) : null }.OfType<Header>().ToArray();
        return await Off(() => d.Fetch(method, path, headers, body ?? []));
    }

    /// Signed out: its connections close. Its key stays, so signing in again as the same person needs no approval.
    public static async Task Close()
    {
        var d = device;
        (device, deviceFor) = (null, null);
        if (d != null) await Off(() => { d.Disconnect(); return 0; });
    }

    /// Runs a call into the library, which waits for its answer, off the caller's thread; its failure in a person's words.
    internal static Task<T> Off<T>(Func<T> work) => Task.Run(() =>
    {
        try { return work(); }
        catch (OneException.Failed e) { throw new NodeException(e.reason); }
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
