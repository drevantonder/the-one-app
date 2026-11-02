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
    public static Task<List<VaultItem>> Vault() => Get<List<VaultItem>>("/v1/vault");

    public static Task<List<VaultRequest>> VaultRequests(string? scope = null, string? id = null) =>
        Get<List<VaultRequest>>("/v1/vault/requests", new() { ["scope"] = scope, ["id"] = id });

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

    /// A file the node keeps, by its hash.
    public static async Task<byte[]> Blob(string hash)
    {
        using var r = await Send(HttpMethod.Get, $"/v1/blobs/{Uri.EscapeDataString(hash)}", timeout: TimeSpan.FromMinutes(5));
        return await r.Content.ReadAsByteArrayAsync();
    }

    public static Task Arrange(string page, IEnumerable<ArrangedItem> items, IEnumerable<string> hidden) =>
        Post("/v1/arrange", new JsonObject
        {
            ["page"] = page,
            ["items"] = JsonSerializer.SerializeToNode(items.ToList(), Json),
            ["hidden"] = new JsonArray(hidden.Select(h => (JsonNode)h).ToArray()),
        });

    public static Task Install(string app, string scope, bool installed) => Post("/v1/install", new JsonObject { ["app"] = app, ["scope"] = scope, ["installed"] = installed });
    public static Task Retire(string app, string how) => Post("/v1/retire", new JsonObject { ["app"] = app, ["how"] = how });

    /// Asks for an item an app declares → the request's id.
    public static async Task<string?> RequestItem(string scope, string app, string need) =>
        (await Post("/v1/vault/request", new JsonObject { ["scope"] = scope, ["app"] = app, ["need"] = need }))?["id"]?.GetValue<string>();

    public static Task Answer(JsonObject answer) => Post("/v1/vault/answer", answer);

    /// Lets an app use an item.
    public static Task Share(VaultItem item, IEnumerable<string> apps) =>
        Post("/v1/vault", new JsonObject { ["scope"] = item.Scope, ["id"] = item.Id, ["apps"] = new JsonArray(apps.Select(a => (JsonNode)a).ToArray()) });

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
