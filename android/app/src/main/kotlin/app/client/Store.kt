package app.client

import android.net.Uri
import android.webkit.CookieManager
import android.webkit.WebStorage
import android.webkit.WebView
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.net.URLEncoder

/** A screen the app shows: one of the client's own, by name (`/v1/client-screens/<name>`: the pages, Services, a Vault
 *  request, Settings'), or an app's. Only this app makes an `Own` route, never a page, so only a screen it chose acts as
 *  the person. */
sealed interface Route {
    data class Own(val name: String, val params: String = "{}") : Route
    data class App(val app: String, val screen: String = "main", val params: String = "{}", val scope: String? = null) : Route
}

/** A screen pushed over the pages, with Back: each one its own, though two may show the same route. */
class Pushed(val route: Route)

/** A screen in a sheet over everything, and what it gives whoever opened it once it's gone: what it gave `one.close`,
 *  or null if it was dismissed. */
class Sheet(val route: Route) {
    val done = CompletableDeferred<JsonElement?>()
    /** Set once it's to go, with what it gives. */
    var going by mutableStateOf(false)
        private set
    var value: JsonElement? = null
        private set

    fun close(value: JsonElement?) {
        if (going) return
        this.value = value
        going = true
    }
}

/** A page: a scope this person may read (theirs, a group's, everyone's), as the home screen's widgets name it. */
data class Page(val scope: String, val title: String)

/** The node as this person sees it, reached over iroh as this phone's own device (Client), and where the app is: the
 *  pages, what's pushed over them, and the sheets over everything. */
class Store(private val application: android.app.Application, val signIn: SignIn, private val prefs: android.content.SharedPreferences) {
    enum class Phase { Starting, SignedOut, Joining, Unreachable, Ready }

    val client = Client(application, signIn)
    /** The code this phone shows while it waits to be approved. */
    var code by mutableStateOf<String?>(null)
        private set

    /** Light, Dark or Automatic, as chosen in Settings on this phone. */
    var appearance by mutableStateOf(runCatching { Appearance.valueOf(prefs.getString("appearance", null) ?: "") }.getOrDefault(Appearance.Automatic))
        private set

    fun choose(a: Appearance) {
        appearance = a
        prefs.edit().putString("appearance", a.name).apply()
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    var phase by mutableStateOf(if (signIn.signedIn) Phase.Starting else Phase.SignedOut)
    var failure by mutableStateOf<String?>(null)
    var state by mutableStateOf<NodeState?>(null)
    var apps by mutableStateOf<List<AppInfo>>(emptyList())
    /** Node events, for every open screen. */
    val events = MutableSharedFlow<String>(extraBufferCapacity = 64)
    private var running: Job? = null

    // MARK: Where the app is

    /** The page the pages screen opens at: a page's scope, or "settings". [opened] counts each time it's told to, so
     *  it opens there again even when it's the same page. */
    var page by mutableStateOf("user")
        private set
    var opened by mutableIntStateOf(0)
        private set
    val path = mutableStateListOf<Pushed>()
    val sheets = mutableStateListOf<Sheet>()

    /** The pages again, at `scope`, with nothing over them. */
    fun openPage(scope: String) {
        page = scope
        opened++
        path.clear()
        sheets.forEach { it.close(null) }
    }

    fun push(route: Route) = path.add(Pushed(route))

    /** A screen in a sheet → what it gave `one.close` once it's gone, or null. */
    suspend fun present(route: Route): JsonElement? = Sheet(route).also { sheets.add(it) }.done.await()

    /** What a screen is called until it says (`one.title`). */
    fun title(route: Route): String = when (route) {
        is Route.App -> app(route.app).let { a -> a?.screens?.firstOrNull { it.name == route.screen }?.title ?: a?.label ?: route.app }
        is Route.Own -> when (route.name) {
            "services" -> "Services"
            "request" -> "Vault"
            else -> ""
        }
    }

    // MARK: The node

    /** A request to the node, as this person: JSON in, the reply's bytes out. */
    suspend fun api(path: String, method: String = "GET", body: JsonElement? = null, query: Map<String, String?> = emptyMap()): ByteArray {
        // Its id makes a call run once, however often it's sent.
        val body = if (path == "/v1/call" && body is JsonObject && "id" !in body) JsonObject(body + ("id" to JsonPrimitive(callId()))) else body
        val r = client.fetch(method, url(path, query), body?.toString()?.toByteArray())
        val bytes = r.read()
        val status = r.status().toInt()
        val error = runCatching { (json.parseToJsonElement(String(bytes)) as? JsonObject)?.get("error")?.let { (it as? JsonPrimitive)?.content } }.getOrNull()
        if (status !in 200..299 || error != null) {
            throw NodeError(error ?: if (status >= 500) "Your devices aren’t answering." else "That didn’t work. Try again.")
        }
        return bytes
    }

    private fun url(path: String, query: Map<String, String?>): String {
        val q = query.filterValues { it != null }.entries.joinToString("&") { (k, v) -> "$k=${URLEncoder.encode(v, "UTF-8")}" }
        return path + if (q.isEmpty()) "" else "?$q"
    }

    /** This phone let in as its person's device: at once if it is, else it asks, shows its code, and waits for one of
     *  their devices (or, for someone new, their inviter's) to approve it. */
    private suspend fun join() {
        if (client.connect()) return
        code = client.ask()
        phase = Phase.Joining
        while (!client.waited()) delay(2000)
        signIn.invite = null
        code = null
        phase = Phase.Starting
    }

    suspend inline fun <reified T> get(path: String, query: Map<String, String?> = emptyMap()): T = json.decodeFromString(String(api(path, query = query)))

    suspend fun post(path: String, body: JsonElement): JsonElement = json.parseToJsonElement(String(api(path, "POST", body)).ifEmpty { "null" })

    /** Runs an app's operation as this person → its result. */
    suspend fun call(app: String, op: String, input: JsonElement = JsonObject(emptyMap())): JsonElement? =
        (post("/v1/call", obj("app" to app, "op" to op, "input" to input, "client" to "android")) as? JsonObject)?.get("ok")

    /** A screen's page, as the node makes it. */
    suspend fun screen(r: Route): String = when (r) {
        is Route.Own -> String(api("/v1/client-screens/${enc(r.name)}", query = mapOf("params" to r.params)))
        is Route.App -> String(api("/v1/screens/${enc(r.app)}/${enc(r.screen)}", query = mapOf("params" to r.params, "scope" to r.scope)))
    }

    suspend fun blob(hash: String): ByteArray = api("/v1/blobs/${enc(hash)}")

    // MARK: Starting and keeping up

    /** Loads who this is and the apps, and keeps up with the node's events, until signed out. */
    fun run() {
        if (running?.isActive == true) return
        drawKept()
        running = scope.launch {
            var misses = 1
            val started = System.currentTimeMillis()
            while (true) {
                try {
                    join()
                    refreshState()
                    break
                } catch (e: NodeError) {
                    if (e.signedOut) return@launch signedOut(e.message)
                    unreachable(e.message, misses, started)
                } catch (e: Exception) {
                    unreachable("Your devices aren’t answering.", misses, started)
                }
                delay(retryWait(misses++))
            }
            phase = Phase.Ready
            launch { runCatching { refreshWidgets() } }
            listen()
        }
    }

    /** Can't Reach Your Devices, once it's more than a blip: a second miss, or 3 seconds of them. */
    private fun unreachable(message: String?, misses: Int, started: Long) {
        failure = message
        if (state == null && phase != Phase.Joining && (misses >= 2 || System.currentTimeMillis() - started >= 3000)) phase = Phase.Unreachable
    }

    /** Back to the first screen, saying `why` when the install turned this phone away rather than the person signing out. */
    fun signedOut(why: String? = null) {
        MainActivity.welcomeError = why
        running?.cancel()
        state = null
        path.clear()
        sheets.forEach { it.close(null) }
        phase = Phase.SignedOut
        apps = emptyList()
        code = null
        scope.launch { client.close() }
        answered = false
        forget()
    }

    /** Nothing of the person's left on this phone: what screens kept (their copies, `one.cache`, the files they showed
     *  or shared, scans, their web storage and cookies, the web view's cache) and what the home-screen widgets showed,
     *  which say to sign in. */
    private fun forget() {
        prefs.edit().apply { prefs.all.keys.filter { it.startsWith("screen:") }.forEach(::remove) }.apply()
        // All the app's cache but the web view's, cleared below, and what Android and WorkManager keep there.
        application.cacheDir.listFiles()?.filter { it.name !in setOf("WebView", "data", "oat_primary") }?.forEach { it.deleteRecursively() }
        scope.launch {
            WebStorage.getInstance().deleteAllData()
            CookieManager.getInstance().removeAllCookies(null)
            WebView(application).apply { clearCache(true) }.destroy()
        }
        app.client.widget.HomeWidgets.signedOut()
    }

    // MARK: The window as it was last

    /** The node's state and apps as it last gave them, kept in the app's cache (which signing out clears), and drawn
     *  as the app opens, while it asks the node again. */
    private fun keptFile(name: String) = java.io.File(application.cacheDir, "window").apply { mkdirs() }.resolve(name)

    private suspend inline fun <reified T> kept(name: String, path: String): T {
        val text = String(api(path))
        val value: T = json.decodeFromString(text)
        withContext(Dispatchers.IO) { runCatching { keptFile(name).writeText(text) } }
        return value
    }

    private fun drawKept() {
        if (state != null || !signIn.signedIn) return
        val kept = runCatching { json.decodeFromString<NodeState>(keptFile("state").readText()) }.getOrNull()
        if (kept?.install == null) return
        state = kept
        apps = runCatching { json.decodeFromString<List<AppInfo>>(keptFile("apps").readText()) }.getOrDefault(emptyList())
        phase = Phase.Ready
    }

    /** Whether the node has answered since the app opened. */
    private var answered = false

    suspend fun refreshState(): Unit = coroutineScope {
        // The first time, the apps are asked for beside the state, not after it.
        val early = if (!answered) async { runCatching { kept<List<AppInfo>>("apps", "/v1/apps") }.getOrNull() } else null
        val next: NodeState = kept("state", "/v1/state")
        val regrouped = (next.user to next.install?.groups) != (state?.user to state?.install?.groups)
        val first = !answered
        answered = true
        state = next
        failure = null
        if (first || regrouped || apps.isEmpty()) refreshApps(early?.await())
    }

    suspend fun refreshApps(early: List<AppInfo>? = null) {
        if (state?.install == null) return
        apps = early ?: kept("apps", "/v1/apps")
    }

    /** Every page's widgets, for the home screen's, while any is there (or `always`, as one is chosen). */
    suspend fun refreshWidgets(always: Boolean = false) {
        if (!always && !app.client.widget.HomeWidgets.placed()) return
        val raw = pages.map { p ->
            scope.async { runCatching { p.scope to String(api("/v1/widgets", query = mapOf("scope" to p.scope))) }.getOrNull() }
        }.awaitAll().filterNotNull().toMap()
        app.client.widget.HomeWidgets.saw(this, raw)
    }

    /** The node's events: what changed, for every open screen, and for the home screen's widgets. */
    private suspend fun listen() {
        var misses = 0
        while (true) {
            val started = System.currentTimeMillis()
            try {
                val r = client.fetch("GET", "/v1/events", accept = "text/event-stream")
                try {
                    var rest = ""
                    while (r.status().toInt() == 200) {
                        val part = io { r.chunk() } ?: break
                        rest += String(part)
                        val lines = rest.split("\n")
                        rest = lines.last()
                        for (line in lines.dropLast(1)) {
                            if (!line.startsWith("data:")) continue
                            val data = line.removePrefix("data:").trim()
                            events.tryEmit(data)
                            runCatching { json.decodeFromString<NodeEvent>(data) }.getOrNull()?.let { e -> scope.launch { runCatching { handle(e) } } }
                        }
                    }
                } finally {
                    r.cancel()
                }
            } catch (e: NodeError) {
                if (e.signedOut) return signedOut(e.message)
            } catch (_: Exception) {
            }
            // At once after a stream that ran a while and ended (its node handing over to a new build), then waiting
            // longer each miss.
            if (System.currentTimeMillis() - started > 10_000) misses = 0
            delay(retryWait(misses++))
        }
    }

    /** Back in front: what changed while it wasn't, which events may have missed, for every open screen and the
     *  home screen's widgets, as the iPhone reads it all again once it's active. */
    fun resumed() {
        if (phase != Phase.Ready) return
        scope.launch {
            runCatching { refreshState() }
            events.tryEmit("""{"type":"install"}""")
            runCatching { refreshWidgets() }
        }
    }

    private suspend fun handle(e: NodeEvent) {
        if (e.type == "devices" || e.type == "install") refreshState()
        if (e.type == "changed") {
            if (e.apps.isNullOrEmpty()) refreshState()
            refreshApps()
            refreshWidgets()
        }
    }

    // MARK: Signing in and out

    fun signInFinished(back: Uri, done: (String?) -> Unit) {
        scope.launch {
            try {
                signIn.finish(back)
                phase = Phase.Starting
                run()
                done(null)
            } catch (e: Exception) {
                done(e.message)
            }
        }
    }

    /** What the app's own screens keep between launches (`one.cache`), by key, forgotten when it signs out. */
    fun kept(key: String): String? = prefs.getString("screen:$key", null)

    fun keep(key: String, value: String) {
        // A screen still open as the person signs out keeps nothing after.
        if (key.isNotEmpty() && key.length <= 40 && signIn.signedIn) prefs.edit().putString("screen:$key", value).apply()
    }

    fun signOut() {
        scope.launch {
            signIn.signOut()
            signedOut()
        }
    }

    // MARK: What the home screen's widgets name

    val install get() = state?.install
    val config get() = state?.config
    fun app(name: String) = apps.firstOrNull { it.name == name }

    /** Every page this person can read: theirs, their groups', then everyone's. */
    val pages: List<Page>
        get() {
            val i = install ?: return emptyList()
            val u = state?.user ?: return emptyList()
            return listOf(Page("user", "You")) + i.groups.filter { g -> g.members.any { it.user == u.id } }.sortedBy { it.name }.map { Page("group:${it.id}", it.name) } +
                Page("global", "Everyone")
        }
}

fun enc(s: String): String = URLEncoder.encode(s, "UTF-8").replace("+", "%20")

/** A JSON object from pairs: strings, numbers, booleans, JSON, or null. */
fun obj(vararg pairs: Pair<String, Any?>): JsonObject = JsonObject(pairs.associate { (k, v) ->
    k to when (v) {
        null -> kotlinx.serialization.json.JsonNull
        is JsonElement -> v
        is String -> JsonPrimitive(v)
        is Number -> JsonPrimitive(v)
        is Boolean -> JsonPrimitive(v)
        is List<*> -> JsonArray(v.map { JsonPrimitive(it.toString()) })
        else -> JsonPrimitive(v.toString())
    }
})
