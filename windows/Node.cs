// Requests to a node, through the install's address, as this person: each with the access token, as the web client
// makes them (kernel/web/web.js). A refused token is refreshed once; refused again, the person is signed out.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;

namespace One;

public sealed class NodeException(string message) : Exception(message);

public static class Node
{
    static readonly HttpClient http = new() { Timeout = Timeout.InfiniteTimeSpan };

    public static readonly JsonSerializerOptions Json = new() { DefaultIgnoreCondition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull };

    /// A request → its response, once it's ok; with the person's token, refreshed once if it's refused.
    static async Task<HttpResponseMessage> Send(HttpMethod method, string path, Dictionary<string, string?>? query = null, JsonNode? body = null,
        bool stream = false, TimeSpan? timeout = null, CancellationToken cancel = default)
    {
        var url = $"{Auth.Shared.Address}{path}";
        var q = (query ?? []).Where(kv => kv.Value != null).Select(kv => $"{Uri.EscapeDataString(kv.Key)}={Uri.EscapeDataString(kv.Value!)}").ToList();
        if (q.Count > 0) url += "?" + string.Join("&", q);
        var token = await Auth.Shared.Token();
        for (var tries = 0; ; tries++)
        {
            using var request = new HttpRequestMessage(method, url);
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
            if (body != null) request.Content = new StringContent(body.ToJsonString(), Encoding.UTF8, "application/json");
            using var limit = CancellationTokenSource.CreateLinkedTokenSource(cancel);
            if (!stream) limit.CancelAfter(timeout ?? TimeSpan.FromSeconds(60));
            HttpResponseMessage r;
            try
            {
                r = await http.SendAsync(request, stream ? HttpCompletionOption.ResponseHeadersRead : HttpCompletionOption.ResponseContentRead, limit.Token);
            }
            catch (Exception) when (!cancel.IsCancellationRequested)
            {
                throw new NodeException("Your server isn’t answering.");
            }
            if (r.StatusCode == HttpStatusCode.Unauthorized && tries == 0)
            {
                r.Dispose();
                token = await Auth.Shared.Token(token);
                continue;
            }
            if (r.StatusCode == HttpStatusCode.Unauthorized) throw new SignedOutException("Signed out.");
            if (r.IsSuccessStatusCode) return r;
            var text = await r.Content.ReadAsStringAsync(cancel);
            r.Dispose();
            string? error = null;
            try { error = JsonNode.Parse(text)?["error"]?.GetValue<string>(); } catch { }
            throw new NodeException(error ?? ((int)r.StatusCode >= 500 ? "Your server isn’t answering." : $"It didn’t work ({(int)r.StatusCode})."));
        }
    }

    /// JSON in and out: a failure the node says (`{ error }`) is thrown.
    public static async Task<JsonNode?> Ask(HttpMethod method, string path, Dictionary<string, string?>? query = null, JsonNode? body = null, TimeSpan? timeout = null)
    {
        using var r = await Send(method, path, query, body, timeout: timeout);
        var reply = JsonNode.Parse(await r.Content.ReadAsStringAsync());
        if (reply is JsonObject o && o["error"]?.GetValue<string>() is { } error) throw new NodeException(error);
        return reply;
    }

    public static async Task<T> Get<T>(string path, Dictionary<string, string?>? query = null)
    {
        using var r = await Send(HttpMethod.Get, path, query);
        return JsonSerializer.Deserialize<T>(await r.Content.ReadAsStringAsync(), Json)!;
    }

    public static Task<JsonNode?> Post(string path, JsonNode body) => Ask(HttpMethod.Post, path, body: body);

    public static Task<NodeState> State() => Get<NodeState>("/v1/state");
    public static Task<List<AppInfo>> Apps() => Get<List<AppInfo>>("/v1/apps");
    public static Task<List<WidgetInfo>> Widgets(string scope) => Get<List<WidgetInfo>>("/v1/widgets", new() { ["scope"] = scope });
    public static Task<Dictionary<string, int>> Badges(string scope) => Get<Dictionary<string, int>>("/v1/badges", new() { ["scope"] = scope });

    /// Runs an app's operation as this person → its result.
    public static async Task<JsonNode?> Call(string app, string op, JsonNode? input = null)
    {
        var reply = await Ask(HttpMethod.Post, "/v1/call", body: new JsonObject { ["app"] = app, ["op"] = op, ["input"] = input ?? new JsonObject(), ["client"] = "windows" }, timeout: TimeSpan.FromMinutes(5));
        return reply?["ok"]?.DeepClone();
    }

    /// An app's screen, as its page.
    public static async Task<string> Screen(string app, string screen, string parameters, string? scope)
    {
        using var r = await Send(HttpMethod.Get, $"/v1/screens/{Uri.EscapeDataString(app)}/{Uri.EscapeDataString(screen)}", new() { ["params"] = parameters, ["scope"] = scope }, timeout: TimeSpan.FromSeconds(20));
        return await r.Content.ReadAsStringAsync();
    }

    /// One of this app's own screens (a page, Services, a Vault request, Settings'), as its page: the same on every client.
    public static async Task<string> ClientScreen(string name, string parameters)
    {
        using var r = await Send(HttpMethod.Get, $"/v1/client-screens/{Uri.EscapeDataString(name)}", new() { ["params"] = parameters }, timeout: TimeSpan.FromSeconds(20));
        return await r.Content.ReadAsStringAsync();
    }

    /// An own screen's `one.api`: the node's /v1 at your One App's address, as this person; nothing else there. A call
    /// says it's this app's, as Call does.
    public static Task<JsonNode?> Api(string method, string path, JsonNode? body)
    {
        var address = new Uri(Auth.Shared.Address!);
        var url = new Uri(address, path);
        if (url.Authority != address.Authority || !url.AbsolutePath.StartsWith("/v1/")) throw new NodeException("That isn’t your server’s.");
        var get = method.Equals("GET", StringComparison.OrdinalIgnoreCase);
        body = get ? null : body ?? new JsonObject();
        var call = url.AbsolutePath == "/v1/call";
        if (call && body is JsonObject o) o["client"] = "windows";
        return Ask(new HttpMethod(method.ToUpperInvariant()), url.PathAndQuery, body: body, timeout: call ? TimeSpan.FromMinutes(5) : null);
    }

    /// A file the node keeps, by its hash.
    public static async Task<byte[]> Blob(string hash)
    {
        using var r = await Send(HttpMethod.Get, $"/v1/blobs/{Uri.EscapeDataString(hash)}", timeout: TimeSpan.FromMinutes(5));
        return await r.Content.ReadAsByteArrayAsync();
    }

    /// Asks for an item an app declares → the request's id.
    public static async Task<string?> RequestItem(string scope, string app, string need) =>
        (await Post("/v1/vault/request", new JsonObject { ["scope"] = scope, ["app"] = app, ["need"] = need }))?["id"]?.GetValue<string>();

    /// The node's events, as they come, until `cancel`: what changed, so the window and the open screen keep up.
    public static async Task Events(Action<JsonNode> each, CancellationToken cancel)
    {
        using var r = await Send(HttpMethod.Get, "/v1/events", stream: true, cancel: cancel);
        using var reader = new StreamReader(await r.Content.ReadAsStreamAsync(cancel));
        var data = new StringBuilder();
        while (await reader.ReadLineAsync(cancel) is { } line)
        {
            if (line.StartsWith("data:")) data.Append(line[5..].Trim());
            else if (line.Length == 0 && data.Length > 0)
            {
                try { if (JsonNode.Parse(data.ToString()) is { } e) each(e); } catch { }
                data.Clear();
            }
        }
    }
}
