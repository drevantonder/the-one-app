// Signing in, as the web client does (kernel/web/web.js): this app is an OAuth client of the install's auth service,
// registered the first time it signs in. Sign-in and consent happen in the person's own browser, where their passkey
// works, and come back to a port on this machine (RFC 8252's loopback redirect), any port, since auth matches loopback
// redirects without theirs. The refresh token is kept in Windows' Credential Locker; access tokens only in memory.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Windows.Security.Credentials;

namespace One;

public sealed class SignedOutException(string message) : Exception(message);

public sealed class Auth
{
    public static readonly Auth Shared = new();

    static readonly HttpClient http = new() { Timeout = TimeSpan.FromSeconds(30) };
    /// Where this app keeps what isn't secret: the install's address and its client id there.
    static string Kept => Path.Combine(WhiteLabel.Folder, "sign-in.json");
    /// The Credential Locker's name for this app's refresh token.
    static string Locker => $"{WhiteLabel.Name} sign-in";

    readonly SemaphoreSlim refreshing = new(1, 1);
    string? access;
    DateTime until;

    /// The install's address: the config's, or the one the person gave.
    public string? Address { get; private set; }
    /// The secret of the invite link this person opened here, for their first device's request to be let in.
    public string? Invite
    {
        get => invite;
        set
        {
            invite = value;
            Save();
        }
    }
    string? invite;
    string? client;
    string? clientFor;

    Auth()
    {
        try
        {
            var kept = JsonNode.Parse(File.ReadAllText(Kept));
            Address = kept?["address"]?.GetValue<string>();
            client = kept?["client"]?.GetValue<string>();
            clientFor = kept?["client_for"]?.GetValue<string>();
            invite = kept?["invite"]?.GetValue<string>();
        }
        catch { }
        if (!string.IsNullOrEmpty(WhiteLabel.Address)) Address = WhiteLabel.Address;
    }

    void Save()
    {
        Directory.CreateDirectory(WhiteLabel.Folder);
        File.WriteAllText(Kept, new JsonObject { ["address"] = Address, ["client"] = client, ["client_for"] = clientFor, ["invite"] = invite }.ToJsonString());
    }

    /// Takes what the person typed: an address, or an invite link (`<address>/invite#…`). → the invite link, if it was one.
    public Uri? Take(string typed)
    {
        typed = typed.Trim();
        if (!typed.Contains("://")) typed = "https://" + typed;
        if (!Uri.TryCreate(typed, UriKind.Absolute, out var url) || (url.Scheme != "https" && !(url.Scheme == "http" && url.IsLoopback)))
            throw new Exception("That isn’t an address. It looks like one.example.com.");
        var address = url.GetLeftPart(UriPartial.Authority);
        if (address != Address)
        {
            Address = address;
            client = null;
            clientFor = null;
            Forget();
        }
        Save();
        _ = Shell.Claim(Address);
        if (url.AbsolutePath != "/invite" || url.Fragment.Length <= 1) return null;
        Invite = url.Fragment[1..];
        return url;
    }

    /// Forgets the install, to sign in to another.
    public void Leave()
    {
        Forget();
        access = null;
        Address = string.IsNullOrEmpty(WhiteLabel.Address) ? null : WhiteLabel.Address;
        client = null;
        clientFor = null;
        Save();
        _ = Shell.Claim(Address);
    }

    public bool SignedIn => Address != null && Refresh() != null;

    static string? Refresh()
    {
        try
        {
            var vault = new PasswordVault();
            var credential = vault.Retrieve(Locker, "refresh");
            credential.RetrievePassword();
            return credential.Password;
        }
        catch { return null; }
    }

    static void Keep(string refresh)
    {
        var vault = new PasswordVault();
        Forget();
        vault.Add(new PasswordCredential(Locker, "refresh", refresh));
    }

    static void Forget()
    {
        try
        {
            var vault = new PasswordVault();
            foreach (var c in vault.FindAllByResource(Locker)) vault.Remove(c);
        }
        catch { }
    }

    /// The auth service's endpoints, as it says them.
    async Task<JsonNode> Meta()
    {
        try
        {
            using var r = await http.GetAsync($"{Address}/.well-known/oauth-authorization-server");
            if (!r.IsSuccessStatusCode) throw new Exception();
            return JsonNode.Parse(await r.Content.ReadAsStringAsync())!;
        }
        catch
        {
            throw new Exception("Can’t reach that address. Check it, and that you’re online.");
        }
    }

    const string Redirect = "http://127.0.0.1/callback";

    /// This app, as a client of the auth service: registered the first time it signs in.
    async Task<string> Client(JsonNode meta)
    {
        if (client != null && clientFor == Address) return client;
        var body = new JsonObject
        {
            ["client_name"] = $"{WhiteLabel.Name} on {Environment.MachineName}",
            ["redirect_uris"] = new JsonArray(Redirect),
            ["token_endpoint_auth_method"] = "none",
            ["grant_types"] = new JsonArray("authorization_code", "refresh_token"),
            ["response_types"] = new JsonArray("code"),
            ["application_type"] = "native",
        };
        using var r = await http.PostAsync(meta["registration_endpoint"]!.GetValue<string>(), new StringContent(body.ToJsonString(), Encoding.UTF8, "application/json"));
        var out_ = JsonNode.Parse(await r.Content.ReadAsStringAsync());
        var id = out_?["client_id"]?.GetValue<string>();
        if (!r.IsSuccessStatusCode || id == null)
            throw new Exception(out_?["error_description"]?.GetValue<string>() ?? out_?["message"]?.GetValue<string>() ?? "This computer couldn’t sign up to sign in.");
        client = id;
        clientFor = Address;
        Save();
        return id;
    }

    static string B64u(byte[] bytes) => Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');

    /// Signs in: the person's browser goes to the auth service's sign-in and consent, and comes back here.
    public async Task SignIn(CancellationToken cancel)
    {
        var meta = await Meta();
        var id = await Client(meta);
        var verifier = B64u(RandomNumberGenerator.GetBytes(32));
        var state = B64u(RandomNumberGenerator.GetBytes(16));
        var challenge = B64u(SHA256.HashData(Encoding.ASCII.GetBytes(verifier)));
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        try
        {
            var redirect = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}/callback";
            var query = new Dictionary<string, string>
            {
                ["response_type"] = "code", ["client_id"] = id, ["redirect_uri"] = redirect, ["scope"] = "openid profile offline_access",
                ["code_challenge"] = challenge, ["code_challenge_method"] = "S256", ["state"] = state, ["prompt"] = "consent",
            };
            Shell.Open($"{meta["authorization_endpoint"]!.GetValue<string>()}?{Form(query)}");
            var back = await Callback(listener, cancel);
            if (back.TryGetValue("error", out var error))
                throw new Exception(error == "access_denied" ? "You didn’t let this computer in." : back.GetValueOrDefault("error_description", error));
            if (back.GetValueOrDefault("state") != state || !back.TryGetValue("code", out var code))
                throw new Exception("That sign-in didn’t start here. Sign in again.");
            await Grant(meta, new() { ["grant_type"] = "authorization_code", ["code"] = code, ["redirect_uri"] = redirect, ["client_id"] = id, ["code_verifier"] = verifier });
        }
        finally
        {
            listener.Stop();
        }
    }

    /// Waits for the browser to come back to /callback → its query. The page it shows opens `<cli>://signed-in`, which
    /// brings the window to the front: Windows lets only the browser, in front, put another app there.
    async Task<Dictionary<string, string>> Callback(TcpListener listener, CancellationToken cancel)
    {
        for (;;)
        {
            using var socket = await listener.AcceptTcpClientAsync(cancel);
            var stream = socket.GetStream();
            var reader = new StreamReader(stream, Encoding.ASCII);
            var line = await reader.ReadLineAsync(cancel) ?? "";
            var target = line.Split(' ').Length > 1 ? line.Split(' ')[1] : "";
            var callback = target.StartsWith("/callback?");
            var page = callback
                ? $"<!doctype html><meta charset=utf-8><title>{WebUtility.HtmlEncode(WhiteLabel.Name)}</title><meta name=color-scheme content=\"light dark\"><body style=\"font:15px system-ui;display:grid;place-items:center;height:90vh;color:GrayText\"><p>You’re signed in. Go back to {WebUtility.HtmlEncode(WhiteLabel.Name)}.</p><script>location.href = \"{WhiteLabel.Cli}://signed-in\"</script>"
                : "";
            var bytes = Encoding.UTF8.GetBytes(page);
            var head = Encoding.ASCII.GetBytes($"HTTP/1.1 {(callback ? "200 OK" : "404 Not Found")}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {bytes.Length}\r\nconnection: close\r\n\r\n");
            await stream.WriteAsync(head, cancel);
            await stream.WriteAsync(bytes, cancel);
            if (!callback) continue;
            var out_ = new Dictionary<string, string>();
            foreach (var pair in target["/callback?".Length..].Split('&', StringSplitOptions.RemoveEmptyEntries))
            {
                var kv = pair.Split('=', 2);
                out_[Uri.UnescapeDataString(kv[0].Replace('+', ' '))] = kv.Length > 1 ? Uri.UnescapeDataString(kv[1].Replace('+', ' ')) : "";
            }
            return out_;
        }
    }

    static FormUrlEncodedContent Fields(Dictionary<string, string> fields) => new(fields);
    static string Form(Dictionary<string, string> fields) => string.Join("&", fields.Select(kv => $"{Uri.EscapeDataString(kv.Key)}={Uri.EscapeDataString(kv.Value)}"));

    async Task Grant(JsonNode meta, Dictionary<string, string> fields)
    {
        using var r = await http.PostAsync(meta["token_endpoint"]!.GetValue<string>(), Fields(fields));
        var out_ = JsonNode.Parse(await r.Content.ReadAsStringAsync().ContinueWith(t => t.IsCompletedSuccessfully ? t.Result : "{}"));
        var token = out_?["access_token"]?.GetValue<string>();
        if (!r.IsSuccessStatusCode || token == null)
        {
            var why = out_?["error_description"]?.GetValue<string>() ?? out_?["error"]?.GetValue<string>() ?? "Signing in didn’t work.";
            if ((int)r.StatusCode is 400 or 401) throw new SignedOutException(why);
            throw new Exception(why);
        }
        access = token;
        until = DateTime.UtcNow.AddSeconds(out_?["expires_in"]?.GetValue<double>() ?? 3600);
        // Each refresh token works once: the new one is kept before anything else can ask.
        if (out_?["refresh_token"]?.GetValue<string>() is { } refresh) Keep(refresh);
    }

    /// An access token good for a minute more, new from the refresh token if it's older, or if `stale` was refused.
    public async Task<string> Token(string? stale = null)
    {
        bool Fresh() => access != null && access != stale && until > DateTime.UtcNow.AddMinutes(1);
        if (Fresh()) return access!;
        await refreshing.WaitAsync();
        // The Widgets board's widgets refresh it in a process of their own: one at a time, each with the token the
        // other kept last.
        try
        {
            using var held = await Held();
            if (Fresh()) return access!;
            var refresh = Refresh() ?? throw new SignedOutException("Signed out.");
            var meta = await Meta();
            try
            {
                await Grant(meta, new() { ["grant_type"] = "refresh_token", ["refresh_token"] = refresh, ["client_id"] = client ?? "" });
            }
            catch (SignedOutException)
            {
                Forget();
                access = null;
                throw;
            }
            return access!;
        }
        finally
        {
            refreshing.Release();
        }
    }

    /// This app's processes' lock on refreshing: a file only one may have open.
    static async Task<IDisposable> Held()
    {
        Directory.CreateDirectory(WhiteLabel.Folder);
        for (var tries = 0; ; tries++)
        {
            try { return new FileStream(Path.Combine(WhiteLabel.Folder, "sign-in.lock"), FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None); }
            catch (IOException) when (tries < 300) { await Task.Delay(100); }
        }
    }

    public async Task SignOut()
    {
        var refresh = Refresh();
        Forget();
        access = null;
        try
        {
            var meta = await Meta();
            if (refresh != null && meta["revocation_endpoint"]?.GetValue<string>() is { } revoke)
                await http.PostAsync(revoke, Fields(new() { ["token"] = refresh, ["token_type_hint"] = "refresh_token", ["client_id"] = client ?? "" }));
        }
        catch { }
    }
}
