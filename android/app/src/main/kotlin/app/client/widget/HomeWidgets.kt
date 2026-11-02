package app.client.widget

import android.app.Application
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.net.Uri
import android.util.Base64
import androidx.glance.appwidget.updateAll
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import app.client.BuildConfig
import app.client.ClientApp
import app.client.NodeError
import app.client.Route
import app.client.Store
import app.client.WidgetInfo
import app.client.WidgetNode
import app.client.http
import app.client.json
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

// The home screen's widgets, as the iPhone's (apple/Widget): the app leaves every page's widgets as it last saw them in
// a file of its own, and has the home-screen widgets draw again whenever they change; between, they refresh themselves
// every 15 minutes (or as the system allows) with the app's own sign-in.

/** Every page's widgets as the app last saw them. */
@Serializable
data class Kept(
    val accent: String? = null,
    val widgets: List<KeptWidget> = emptyList(),
)

/** A widget on one of the pages, its tree at each size as `GET /v1/widgets` gave it. */
@Serializable
data class KeptWidget(
    val scope: String,
    /** Its page's name. */
    val page: String,
    val title: String,
    val icon: String? = null,
    val color: String? = null,
    /** Whether its app has a screen to open. */
    val opens: Boolean = false,
    val app: String,
    val widget: String,
    val info: JsonElement,
) {
    /** `<scope>/<app>.<widget>`: what a home-screen widget's choice is found again by. */
    val id get() = "$scope/$app.$widget"
    fun decoded(): WidgetInfo? = runCatching { json.decodeFromJsonElement(WidgetInfo.serializer(), info) }.getOrNull()
}

object HomeWidgets {
    /** What every home-screen widget draws from. */
    data class Shown(val kept: Kept? = null, val choices: Map<Int, String> = emptyMap(), val signedIn: Boolean = false)

    val shown = MutableStateFlow(Shown())
    private lateinit var app: Application
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val file get() = File(app.filesDir, "home-widgets.json")
    private val prefs get() = app.getSharedPreferences("home-widgets", Context.MODE_PRIVATE)
    private val store get() = (app as ClientApp).store

    fun start(context: Application) {
        if (::app.isInitialized) return
        app = context
        val kept = runCatching { json.decodeFromString(Kept.serializer(), file.readText()) }.getOrNull()
        shown.value = Shown(kept, choices(), store.signIn.signedIn)
    }

    fun start(context: Context) = start(context.applicationContext as Application)

    private fun choices(): Map<Int, String> =
        prefs.all.mapNotNull { (k, v) -> k.removePrefix("choice.").toIntOrNull()?.let { id -> (v as? String)?.let { id to it } } }.toMap()

    /** Which widget a home-screen widget shows, chosen by long-pressing it, and its title, to say if it goes. */
    fun choose(widgetId: Int, id: String, title: String) {
        prefs.edit().putString("choice.$widgetId", id).putString("title.$widgetId", title).apply()
        shown.value = shown.value.copy(choices = choices())
        redraw()
    }

    fun title(widgetId: Int): String? = prefs.getString("title.$widgetId", null)

    fun forget(widgetIds: IntArray) {
        prefs.edit().apply { widgetIds.forEach { remove("choice.$it"); remove("title.$it") } }.apply()
        shown.value = shown.value.copy(choices = choices())
    }

    /** What the app's pages just got from `/v1/widgets`, by page: kept, and drawn on the home screen if it changed. */
    fun saw(store: Store, raw: Map<String, String>) {
        if (!::app.isInitialized) return
        val before = shown.value.kept?.widgets.orEmpty()
        val widgets = store.pages.flatMap { p ->
            val array = raw[p.scope]?.let { runCatching { json.parseToJsonElement(it) as JsonArray }.getOrNull() }
                ?: return@flatMap before.filter { it.scope == p.scope }
            array.mapNotNull { e ->
                val w = runCatching { json.decodeFromJsonElement(WidgetInfo.serializer(), e) }.getOrNull() ?: return@mapNotNull null
                val a = store.app(w.app)?.takeIf { it.scopes.contains(p.scope) } ?: return@mapNotNull null
                KeptWidget(p.scope, p.title, w.title ?: a.label, a.icon, a.color, a.hasScreens, w.app, w.widget, e)
            }
        }
        // A refresh that saw nothing keeps what was: the home screen goes blank only for a reason.
        if (widgets.isEmpty()) return
        val kept = Kept(store.config?.accent, widgets)
        val signedIn = store.signIn.signedIn
        if (kept == shown.value.kept && signedIn == shown.value.signedIn) return
        shown.value = shown.value.copy(kept = kept, signedIn = signedIn)
        scope.launch {
            runCatching { file.writeText(json.encodeToString(Kept.serializer(), kept)) }
            redraw()
            if (saveImages(store, widgets)) redraw()
        }
    }

    /** Signed out: the widgets say so, and forget what they showed and its images. */
    fun signedOut() {
        if (!::app.isInitialized) return
        file.delete()
        images.deleteRecursively()
        shown.value = shown.value.copy(kept = null, signedIn = false)
        redraw()
    }

    fun redraw() {
        scope.launch { runCatching { HomeWidget().updateAll(app) } }
    }

    /** Fetches every page's widgets again, as the app does, which keeps and draws them. */
    suspend fun refresh() {
        val store = store
        if (!store.signIn.signedIn) {
            if (shown.value.signedIn || shown.value.kept != null) withContext(Dispatchers.Main) { store.signedOut() }
            return
        }
        try {
            val first = store.state == null
            store.refreshState()
            if (!first) store.refreshApps()
            store.refreshWidgets(always = true)
        } catch (e: NodeError) {
            // Signed out while the app was away: the app forgets what it kept too, not only the widgets.
            if (e.signedOut) withContext(Dispatchers.Main) { store.signedOut(e.message) }
        } catch (_: Exception) {
        }
    }

    // MARK: Opening the app from a widget

    /** `<cli>://open/?scope=…&app=…&screen=…&params=…`: the widget's page, and its screen if it has one. */
    fun link(w: KeptWidget, screen: String?, params: JsonElement?): Uri = Uri.Builder().scheme(BuildConfig.CLI).authority("open")
        .appendQueryParameter("scope", w.scope).appendQueryParameter("app", w.app)
        .apply { if (screen != null) appendQueryParameter("screen", screen) }
        .apply { if (params != null) appendQueryParameter("params", params.toString()) }
        .build()

    /** Opens what [link] names, over the pages. */
    fun open(store: Store, uri: Uri) {
        val scope = uri.getQueryParameter("scope") ?: return
        val app = uri.getQueryParameter("app") ?: return
        store.openPage(scope)
        val screen = uri.getQueryParameter("screen") ?: return
        store.push(Route.App(app, screen, uri.getQueryParameter("params") ?: "{}", scope))
    }

    /** Whether any is on the home screen, so the app keeps them up to date. */
    fun placed(): Boolean = ::app.isInitialized &&
        AppWidgetManager.getInstance(app).getAppWidgetIds(ComponentName(app, HomeWidgetReceiver::class.java)).isNotEmpty()

    // MARK: Images

    private val images get() = File(app.filesDir, "home-widget-images")

    /** Where an image a widget shows is saved: the home screen can't download. */
    fun imageFile(url: String): File {
        val name = MessageDigest.getInstance("SHA-256").digest(url.toByteArray()).take(12).joinToString("") { "%02x".format(it) }
        return File(images.apply { mkdirs() }, name)
    }

    /** Saves the images not saved yet → whether any were. */
    private suspend fun saveImages(store: Store, widgets: List<KeptWidget>): Boolean {
        val urls = mutableSetOf<String>()
        fun walk(n: WidgetNode) {
            if (n.t == "image") n.url?.takeIf { !it.startsWith("data:") }?.let(urls::add)
            n.c?.forEach(::walk)
        }
        widgets.mapNotNull { it.decoded() }.forEach { w -> (listOf(w.root) + w.sizes?.values.orEmpty()).forEach(::walk) }
        var saved = false
        for (url in urls) {
            val f = imageFile(url)
            if (f.exists()) continue
            val bytes = runCatching {
                when {
                    url.startsWith("https://") -> http("GET", url).takeIf { it.status == 200 }?.bytes
                    url.startsWith("/v1/blobs/") -> store.blob(url.removePrefix("/v1/blobs/"))
                    else -> null
                }
            }.getOrNull() ?: continue
            if (bytes.size > 2_000_000) continue
            f.writeBytes(bytes)
            saved = true
        }
        return saved
    }

    fun dataImage(url: String): ByteArray? = runCatching { Base64.decode(url.substringAfter(','), Base64.DEFAULT) }.getOrNull()

    // MARK: Refreshing on their own

    /** Refreshes now, and then while any widget is on the home screen. */
    fun schedule(context: Context) {
        val online = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
        val work = WorkManager.getInstance(context)
        work.enqueueUniqueWork("home-widgets-now", ExistingWorkPolicy.KEEP, OneTimeWorkRequestBuilder<Refresh>().setConstraints(online).build())
        work.enqueueUniquePeriodicWork("home-widgets", ExistingPeriodicWorkPolicy.KEEP, PeriodicWorkRequestBuilder<Refresh>(15, TimeUnit.MINUTES).setConstraints(online).build())
    }

    fun unschedule(context: Context) {
        WorkManager.getInstance(context).cancelUniqueWork("home-widgets")
    }

    class Refresh(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
        override suspend fun doWork(): Result {
            start(applicationContext)
            refresh()
            return Result.success()
        }
    }
}
