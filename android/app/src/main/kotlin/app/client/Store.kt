package app.client

import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

/** Where the app is, over its pages: what's pushed on top, as the iPhone app's navigation path. */
sealed interface Route {
    data class Screen(val app: String, val screen: String = "main", val params: String = "{}", val scope: String? = null) : Route
    data class Services(val scope: String) : Route
    /** A Settings screen, by its item's name. */
    data class Setting(val name: String, val params: String = "{}") : Route
}

/** A page: a scope this person may read (theirs, a group's, everyone's), as a page of icons and widgets. */
data class Page(val scope: String, val title: String, val subtitle: String, val whose: String)

/** One thing on a page: an app's icon, or a copy of a widget (`kept` is how the page keeps it). */
sealed interface PageItem {
    val key: String
    data class Icon(val app: AppInfo) : PageItem { override val key get() = "app:${app.name}" }
    data class Widget(val widget: WidgetInfo, val kept: ArrangementItem, val size: String) : PageItem { override val key get() = "widget:${kept.id}" }
}

/** A request to the Vault shown as a sheet, and what to do once it's gone. */
data class VaultSheet(val scope: String, val id: String, val done: (() -> Unit)? = null)

/** The node as this person sees it, through the install's address with their sign-in. */
class Store(val signIn: SignIn, private val prefs: android.content.SharedPreferences) {
    enum class Phase { Starting, SignedOut, Unreachable, Ready }

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
    var widgets by mutableStateOf<Map<String, List<WidgetInfo>>>(emptyMap())
    var badges by mutableStateOf<Map<String, Map<String, Int>>>(emptyMap())
    val path = mutableStateListOf<Route>()
    /** The page shown: a page's scope, or [SETTINGS]. */
    var current by mutableStateOf("user")
    var sheet by mutableStateOf<VaultSheet?>(null)
    /** Node events, for the open screen. */
    val events = MutableSharedFlow<String>(extraBufferCapacity = 64)
    private var running: Job? = null

    // MARK: The node

    /** A request to the node, as this person: JSON in, the reply's bytes out. A refused token is renewed once. */
    suspend fun api(path: String, method: String = "GET", body: JsonElement? = null, query: Map<String, String?> = emptyMap()): ByteArray {
        val url = url(path, query)
        var token = signIn.token()
        for (tries in 0..1) {
            val r = http(method, url, body?.toString()?.toByteArray(), "application/json", mapOf("authorization" to "Bearer $token"))
            if (r.status == 401 && tries == 0) {
                token = signIn.token(stale = token)
                continue
            }
            val error = runCatching { (json.parseToJsonElement(r.text) as? JsonObject)?.get("error")?.let { (it as? JsonPrimitive)?.content } }.getOrNull()
            if (r.status == 401) throw NodeError(error ?: "Sign in again.", signedOut = true)
            if (r.status !in 200..299 || error != null) {
                throw NodeError(error ?: if (r.status >= 500) "Your server isn’t answering." else "It didn’t work (${r.status}).")
            }
            return r.bytes
        }
        error("unreachable")
    }

    private fun url(path: String, query: Map<String, String?>): String {
        val q = query.filterValues { it != null }.entries.joinToString("&") { (k, v) -> "$k=${URLEncoder.encode(v, "UTF-8")}" }
        return "${signIn.address}$path" + if (q.isEmpty()) "" else "?$q"
    }

    suspend inline fun <reified T> get(path: String, query: Map<String, String?> = emptyMap()): T = json.decodeFromString(String(api(path, query = query)))

    suspend fun post(path: String, body: JsonElement): JsonElement = json.parseToJsonElement(String(api(path, "POST", body)).ifEmpty { "null" })

    /** Runs an app's operation as this person → its result. */
    suspend fun call(app: String, op: String, input: JsonElement = JsonObject(emptyMap())): JsonElement? =
        (post("/v1/call", obj("app" to app, "op" to op, "input" to input, "client" to "android")) as? JsonObject)?.get("ok")

    /** An app screen's page, as its node made it. */
    suspend fun screen(r: Route.Screen): String = String(api("/v1/screens/${enc(r.app)}/${enc(r.screen)}", query = mapOf("params" to r.params, "scope" to r.scope)))

    /** A Settings screen's page. */
    suspend fun settingsScreen(r: Route.Setting): String = String(api("/v1/settings-screens/${enc(r.name)}", query = mapOf("params" to r.params)))

    suspend fun blob(hash: String): ByteArray = api("/v1/blobs/${enc(hash)}")

    suspend fun vaultRequests(scope: String, id: String): List<VaultRequest> = get("/v1/vault/requests", mapOf("scope" to scope, "id" to id))

    // MARK: Starting and keeping up

    /** Loads what the pages show, and keeps up with the node's events, until signed out. */
    fun run() {
        if (running?.isActive == true) return
        running = scope.launch {
            var wait = 1000L
            while (true) {
                try {
                    refreshState()
                    break
                } catch (e: NodeError) {
                    if (e.signedOut) return@launch signedOut(e.message)
                    unreachable(e.message)
                } catch (e: Exception) {
                    unreachable("Can’t reach ${signIn.address}.")
                }
                delay(wait)
                wait = minOf(wait * 2, 15_000)
            }
            phase = Phase.Ready
            launch { redrawing() }
            listen()
        }
    }

    private fun unreachable(message: String?) {
        failure = message
        if (state == null) phase = Phase.Unreachable
    }

    /** Back to the first screen, saying `why` when the install turned this phone away rather than the person signing out. */
    private fun signedOut(why: String? = null) {
        MainActivity.welcomeError = why
        running?.cancel()
        state = null
        path.clear()
        phase = Phase.SignedOut
    }

    suspend fun refreshState() {
        val next: NodeState = get("/v1/state")
        val regrouped = (next.user to next.install?.groups) != (state?.user to state?.install?.groups)
        val first = state == null
        state = next
        failure = null
        if (first || regrouped || apps.isEmpty()) refreshApps()
    }

    suspend fun refreshApps() {
        if (state?.install == null) return
        apps = get("/v1/apps")
        refreshWidgets()
    }

    suspend fun refreshWidgets() {
        val raw = java.util.concurrent.ConcurrentHashMap<String, String>()
        val results = pages.map { p ->
            scope.async {
                val w = runCatching { String(api("/v1/widgets", query = mapOf("scope" to p.scope))).also { raw[p.scope] = it }.let { json.decodeFromString<List<WidgetInfo>>(it) } }
                    .getOrElse { widgets[p.scope] ?: emptyList() }
                val b = runCatching { get<Map<String, Int>>("/v1/badges", mapOf("scope" to p.scope)) }.getOrElse { emptyMap() }
                Triple(p.scope, w, b)
            }
        }.awaitAll()
        widgets = results.associate { it.first to it.second }
        badges = results.associate { it.first to it.third }
        app.client.widget.HomeWidgets.saw(this, raw)
    }

    /** Draws the widgets again when the soonest asks to be (a tree's `next`), between a minute and fifteen from now. */
    private suspend fun redrawing() {
        while (true) {
            val nexts = mutableListOf<Double>()
            fun walk(n: WidgetNode) { n.next?.let(nexts::add); n.c?.forEach(::walk) }
            widgets.values.flatten().forEach { w -> (w.sizes?.values ?: listOf(w.root)).forEach(::walk) }
            val soonest = nexts.minOrNull()?.toLong()?.minus(System.currentTimeMillis()) ?: (15 * 60_000L)
            delay(soonest.coerceIn(60_000, 15 * 60_000))
            runCatching { refreshWidgets() }
        }
    }

    /** The node's events: what changed, so the pages and the open screen keep up. */
    private suspend fun listen() {
        while (true) {
            try {
                val token = signIn.token()
                withContext(Dispatchers.IO) {
                    val c = URL(url("/v1/events", emptyMap())).openConnection() as HttpURLConnection
                    c.setRequestProperty("authorization", "Bearer $token")
                    c.setRequestProperty("accept", "text/event-stream")
                    c.readTimeout = 0
                    try {
                        if (c.responseCode == 401) signIn.token(stale = token)
                        if (c.responseCode == 200) c.inputStream.bufferedReader().useLines { lines ->
                            for (line in lines) {
                                if (!line.startsWith("data:")) continue
                                val data = line.removePrefix("data:").trim()
                                events.tryEmit(data)
                                runCatching { json.decodeFromString<NodeEvent>(data) }.getOrNull()?.let { e -> scope.launch { runCatching { handle(e) } } }
                            }
                        }
                    } finally {
                        c.disconnect()
                    }
                }
            } catch (e: NodeError) {
                if (e.signedOut) return signedOut(e.message)
            } catch (_: Exception) {
            }
            delay(3000)
        }
    }

    private suspend fun handle(e: NodeEvent) {
        if (e.type == "servers" || e.type == "install") refreshState()
        if (e.type == "changed") {
            if (e.apps.isNullOrEmpty()) refreshState()
            refreshApps()
            changed.tryEmit(e.apps ?: emptyList())
        }
    }

    /** The apps a change was to (none: any), so the Services view lists their accounts again. */
    val changed = MutableSharedFlow<List<String>>(extraBufferCapacity = 16)

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

    fun signOut() {
        scope.launch {
            signIn.signOut()
            signedOut()
            app.client.widget.HomeWidgets.signedOut()
            apps = emptyList()
            widgets = emptyMap()
        }
    }

    /** Runs a change, then takes in what it changed. → what went wrong, if it did. */
    suspend fun change(work: suspend () -> Unit): String? = try {
        work()
        refreshState()
        refreshApps()
        null
    } catch (e: Exception) {
        e.message
    }

    // MARK: What the pages show (as kernel/web/web.js works it out)

    val me get() = state?.user
    val install get() = state?.install
    val isAdmin get() = me?.admin == true
    val config get() = state?.config
    fun app(name: String) = apps.firstOrNull { it.name == name }
    fun userName(id: String) = if (id == me?.id) "You" else install?.user(id)?.name ?: id.take(6)

    /** Every page this person can read: theirs, their groups', then everyone's. */
    val pages: List<Page>
        get() {
            val i = install ?: return emptyList()
            val u = me ?: return emptyList()
            val devices = i.servers.count { it.user == u.id } + i.clients.count { it.user == u.id }
            val out = mutableListOf(Page("user", "You", if (devices == 1) "Just you · 1 device" else "Just you · $devices devices", "For you only"))
            for (g in i.groups.filter { g -> g.members.any { it.user == u.id } }.sortedBy { it.name }) {
                out += Page("group:${g.id}", g.name, "${people(g.members.size)} · ${chosenBy(g.admins)}", "For ${g.name}")
            }
            out += Page("global", "Everyone", "${people(i.users.size)} · ${chosenBy(i.users.filter { it.admin }.map { it.id })}", "For everyone")
            return out
        }

    fun page(scope: String) = pages.firstOrNull { it.scope == scope }

    private fun people(n: Int) = if (n == 1) "1 person" else "$n people"

    private fun chosenBy(admins: List<String>): String {
        val names = admins.sortedWith(compareBy({ it != me?.id }, { userName(it) })).map { if (it == me?.id) "you" else userName(it) }
        return when (names.size) {
            0 -> "apps chosen by admins"
            1 -> "apps chosen by ${names[0]}"
            2 -> "apps chosen by ${names[0]} and ${names[1]}"
            else -> "apps chosen by ${names.size} admins"
        }
    }

    /** Whether this person may install into a scope: their own, a group they admin, or everyone's as an admin. */
    fun canArrange(scope: String): Boolean = when {
        scope == "user" -> true
        scope == "global" -> isAdmin
        else -> install?.groups?.firstOrNull { "group:${it.id}" == scope }?.role(me?.id ?: "") == "admin"
    }

    fun appsOn(p: Page) = apps.filter { it.hasScreens && it.scopes.contains(p.scope) }
    fun servicesOn(p: Page) = apps.filter { it.isService && it.scopes.contains(p.scope) }.sortedBy { it.label }
    fun badge(app: String, p: Page) = maxOf(0, badges[p.scope]?.get(app) ?: 0)

    fun owns(a: AppInfo): Boolean {
        val owner = a.owner ?: return false
        val u = me ?: return false
        if (owner == "admins") return isAdmin
        if (owner == "user:${u.id}") return true
        return owner.startsWith("group:") && install?.group(owner.removePrefix("group:"))?.role(u.id) == "admin"
    }

    /** Every widget a page could show: those of the apps installed in its scope. */
    fun widgetsOn(p: Page): List<WidgetInfo> {
        val here = apps.filter { it.scopes.contains(p.scope) }.map { it.name }.toSet()
        return widgets[p.scope].orEmpty().filter { here.contains(it.app) }
    }

    /** What a page shows, in its person's order: the widgets they put there and its apps' icons, those installed since
     *  they last arranged it at the end, without those they took off it. A page never arranged shows its apps. */
    fun items(p: Page): List<PageItem> {
        val appsHere = appsOn(p)
        val widgetsHere = widgetsOn(p)
        val arrangement = state?.arrangements?.get(p.scope) ?: return appsHere.map { PageItem.Icon(it) }
        val out = mutableListOf<PageItem>()
        val seen = mutableSetOf<String>()
        for (item in arrangement.items) {
            if (item.widget != null) {
                val w = widgetsHere.firstOrNull { it.app == item.app && it.widget == item.widget }
                if (item.id == null || w == null || !seen.add("widget:${item.id}")) continue
                out += PageItem.Widget(placed(w, item), item, item.size?.takeIf { w.supported.contains(it) } ?: w.size ?: "medium")
            } else {
                val a = appsHere.firstOrNull { it.name == item.app } ?: continue
                if (seen.add("app:${a.name}")) out += PageItem.Icon(a)
            }
        }
        val hidden = arrangement.hidden.orEmpty().toSet()
        return out + appsHere.filter { !seen.contains("app:${it.name}") && !hidden.contains(it.name) }.map { PageItem.Icon(it) }
    }

    /** A widget as one copy of it on a page draws: with the trees and error drawn with that copy's options. */
    private fun placed(w: WidgetInfo, item: ArrangementItem): WidgetInfo {
        val mine = item.id?.let { w.placed?.get(it) } ?: return w
        return w.copy(sizes = (w.sizes ?: emptyMap()) + (mine.sizes ?: emptyMap()), error = mine.error ?: w.error)
    }

    /** Keeps a page in its person's order with `edit` applied to each item (null takes it off). */
    fun arrange(p: Page, edit: (ArrangementItem) -> ArrangementItem?, failed: (String) -> Unit) {
        val kept = items(p).map { if (it is PageItem.Widget) it.kept else ArrangementItem((it as PageItem.Icon).app.name) }.mapNotNull(edit)
        val hidden = state?.arrangements?.get(p.scope)?.hidden.orEmpty()
        state = state?.copy(arrangements = (state?.arrangements ?: emptyMap()) + (p.scope to Arrangement(kept, hidden)))
        scope.launch {
            runCatching {
                post("/v1/arrange", obj("page" to p.scope, "items" to json.encodeToJsonElement(kotlinx.serialization.builtins.ListSerializer(ArrangementItem.serializer()), kept), "hidden" to JsonArray(hidden.map(::JsonPrimitive))))
            }.onFailure { failed(it.message ?: "") }
        }
    }

    fun launch(work: suspend CoroutineScope.() -> Unit) = scope.launch(block = work)

    /** The install's App Store, if it's on one of this person's pages. */
    val storeApp: AppInfo?
        get() {
            val s = config?.store?.let(::app) ?: return null
            return s.takeIf { it.hasScreens && pages.any { p -> it.scopes.contains(p.scope) } }
        }

    fun settingsRoute(a: AppInfo, p: Page, add: Boolean = false) =
        if (a.has("settings")) Route.Screen(a.name, "settings", if (add) "{\"add\":true}" else "{}", p.scope) else null

    fun storeRoute(a: AppInfo): Route.Screen? {
        val s = storeApp ?: return null
        return if (s.has("app") && (a.deploys ?: 0) > 0) Route.Screen(s.name, "app", obj("app" to a.name).toString()) else null
    }

    /** What Settings shows this person, in order, as the node says. */
    val settingsItems: List<SettingsItem> get() = state?.settingsItems.orEmpty()

    /** Whether Settings has something to see: requests waiting, or backups' problem. */
    val attention: Boolean get() = settingsItems.any { (it.badge ?: 0) > 0 }

    companion object {
        const val SETTINGS = "settings"
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
