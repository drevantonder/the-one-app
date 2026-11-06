// Requests to the person's nodes, over iroh as this computer's own device (Client), with the keys each needs.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using uniffi.one_client_ffi;

namespace One;

public sealed class NodeException(string message) : Exception(message);

/// This computer isn't one of the network's, yet or any more: not an app's problem, so the window shows its approval
/// screen, or its sign-in.
public sealed class NotLetInException() : Exception("This computer hasn’t been let in yet.");

public static class Node
{
    public static readonly JsonSerializerOptions Json = new() { DefaultIgnoreCondition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull };

    /// How long to wait before asking again the `misses`th time running (from 0) after the node didn't answer: at once,
    /// then a quarter second, doubling, up to five. A node restarting for an update is back within a moment.
    public static TimeSpan RetryWait(int misses) => TimeSpan.FromMilliseconds(misses == 0 ? 0 : Math.Min(5000, 125 << Math.Min(misses, 6)));

    /// Names a call wherever it's sent, so sent again it runs once.
    static string CallId() => Convert.ToHexStringLower(System.Security.Cryptography.RandomNumberGenerator.GetBytes(8));

    /// A request → its response, once it's ok; a call carries an id, so it runs once however often it's sent.
    static async Task<Response> Send(HttpMethod method, string path, Dictionary<string, string?>? query = null, JsonNode? body = null, string? accept = null)
    {
        var q = (query ?? []).Where(kv => kv.Value != null).Select(kv => $"{Uri.EscapeDataString(kv.Key)}={Uri.EscapeDataString(kv.Value!)}").ToList();
        if (q.Count > 0) path += "?" + string.Join("&", q);
        if (path.StartsWith("/v1/call") && body is JsonObject call && call["id"] == null) call["id"] = CallId();
        var r = await Client.Fetch(method.Method, path, body == null ? null : Encoding.UTF8.GetBytes(body.ToJsonString()), accept);
        if (r.Status() is >= 200 and < 300) return r;
        var text = Encoding.UTF8.GetString(await Client.Off(r.Bytes));
        string? error = null;
        try { error = JsonNode.Parse(text)?["error"]?.GetValue<string>(); } catch { }
        throw new NodeException(error ?? (r.Status() >= 500 ? "Your devices aren’t answering." : "That didn’t work. Try again."));
    }

    static async Task<string> Text(Response r) => Encoding.UTF8.GetString(await Client.Off(r.Bytes));

    /// JSON in and out: a failure the node says (`{ error }`) is thrown.
    public static async Task<JsonNode?> Ask(HttpMethod method, string path, Dictionary<string, string?>? query = null, JsonNode? body = null, TimeSpan? timeout = null)
    {
        var reply = JsonNode.Parse(await Text(await Send(method, path, query, body)));
        if (reply is JsonObject o && o["error"]?.GetValue<string>() is { } error) throw new NodeException(error);
        return reply;
    }

    public static async Task<T> Get<T>(string path, Dictionary<string, string?>? query = null)
    {
        return JsonSerializer.Deserialize<T>(await Text(await Send(HttpMethod.Get, path, query)), Json)!;
    }

    public static Task<JsonNode?> Post(string path, JsonNode body) => Ask(HttpMethod.Post, path, body: body);

    public static Task<NodeState> State() => Get<NodeState>("/v1/state");
    public static Task<List<AppInfo>> Apps() => Get<List<AppInfo>>("/v1/apps");
    public static Task<List<WidgetInfo>> Widgets(string scope) => Get<List<WidgetInfo>>("/v1/widgets", new() { ["scope"] = scope });
    public static Task<Dictionary<string, int>> Badges(string scope) => Get<Dictionary<string, int>>("/v1/badges", new() { ["scope"] = scope });

    /// Runs an app's tool as this person → its result.
    public static async Task<JsonNode?> Call(string app, string op, JsonNode? input = null)
    {
        var reply = await Ask(HttpMethod.Post, "/v1/call", body: new JsonObject { ["app"] = app, ["op"] = op, ["input"] = input ?? new JsonObject(), ["client"] = "windows" }, timeout: TimeSpan.FromMinutes(5));
        return reply?["ok"]?.DeepClone();
    }

    /// What an app screen's call answered last, if its tool only reads: `{ answer, at }`, for bridge.js's `kept`.
    public static async Task<JsonNode?> Kept(string app, string op, JsonNode? input)
    {
        if (await Client.Kept(app, op, System.Text.Encoding.UTF8.GetBytes((input ?? new JsonObject()).ToJsonString())) is not { } kept) return null;
        try { return new JsonObject { ["answer"] = JsonNode.Parse(kept.Answer), ["at"] = kept.At }; }
        catch { return null; }
    }

    /// An app's screen, as its page.
    public static async Task<string> Screen(string app, string screen, string parameters, string? scope)
    {
        return await Text(await Send(HttpMethod.Get, $"/v1/screens/{Uri.EscapeDataString(app)}/{Uri.EscapeDataString(screen)}", new() { ["params"] = parameters, ["scope"] = scope }));
    }

    /// One of this app's own screens (a page, Accounts, a Vault request, Settings'), as its page, the same on every
    /// client: from those built into the app (client/ffi), never a node's, since they act as its person.
    public static Task<string> ClientScreen(string name, string parameters) => Task.FromResult(OneClientFfiMethods.ClientScreen(name, parameters));

    /// An own screen's `one.api`: the node's /v1 at your One App network's address, as this person; nothing else there. A call
    /// says it's this app's, as Call does.
    public static Task<JsonNode?> Api(string method, string path, JsonNode? body)
    {
        var address = new Uri(Auth.Shared.Address!);
        var url = new Uri(address, path);
        if (url.Authority != address.Authority || !url.AbsolutePath.StartsWith("/v1/")) throw new NodeException("That isn’t on your devices.");
        var get = method.Equals("GET", StringComparison.OrdinalIgnoreCase);
        body = get ? null : body ?? new JsonObject();
        var call = url.AbsolutePath == "/v1/call";
        if (call && body is JsonObject o) o["client"] = "windows";
        return Ask(new HttpMethod(method.ToUpperInvariant()), url.PathAndQuery, body: body, timeout: call ? TimeSpan.FromMinutes(5) : null);
    }

    /// A file the node keeps, by its hash.
    public static async Task<byte[]> Blob(string hash)
    {
        var r = await Send(HttpMethod.Get, $"/v1/blobs/{Uri.EscapeDataString(hash)}");
        return await Client.Off(r.Bytes);
    }

    /// Asks for an item an app declares → the request's id, and the scope it's in (their own, where they can't change
    /// the page's).
    public static async Task<(string Id, string Scope)?> RequestItem(string scope, string app, string need)
    {
        var asked = await Post("/v1/vault/request", new JsonObject { ["scope"] = scope, ["app"] = app, ["need"] = need });
        var id = asked?["id"]?.GetValue<string>();
        return id == null ? null : (id, asked?["scope"]?.GetValue<string>() ?? scope);
    }

    /// The node's events, as they come, until `cancel`: what changed, so the window and the open screen keep up.
    public static async Task Events(Action<JsonNode> each, CancellationToken cancel)
    {
        var r = await Send(HttpMethod.Get, "/v1/events", accept: "text/event-stream");
        using var stop = cancel.Register(r.Cancel);
        var rest = new List<byte>();
        var data = new StringBuilder();
        while (!cancel.IsCancellationRequested && await Client.Off(r.Chunk) is { } part)
        {
            rest.AddRange(part);
            for (int end; (end = rest.IndexOf((byte)'\n')) >= 0; )
            {
                var line = Encoding.UTF8.GetString(rest.GetRange(0, end).ToArray()).TrimEnd('\r');
                rest.RemoveRange(0, end + 1);
                if (line.StartsWith("data:")) data.Append(line[5..].Trim());
                else if (line.Length == 0 && data.Length > 0)
                {
                    try { if (JsonNode.Parse(data.ToString()) is { } e) each(e); } catch { }
                    data.Clear();
                }
            }
        }
    }
}
