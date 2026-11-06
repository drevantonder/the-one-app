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
import app.client.Glyphs
import app.client.Native
import app.client.WidgetInfo
import app.client.WidgetNode
import app.client.json
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

// The home screen's widgets, as the iPhone's (client/native/iphone/Widget): the app's library leaves every page's widgets
// as it last saw them in files of its own (client/native/src/widgets.rs, in `widgets/` here: widgets.json, icons.json and
// the pictures they show), which it keeps again whenever something changes, and these draw from; between, they refresh
// themselves every 15 minutes (or as the system allows), through the same library, with the app's own sign-in.

/** Every page's widgets as the app last saw them: `widgets.json`. */
@Serializable
data class Kept(
    val widgets: List<KeptWidget> = emptyList(),
    /** When they were all fetched, in seconds from 2001 (as the iPhone's widget reads it). */
    val fetched: Double? = null,
    /** Set once the person signs out: nothing of theirs is left. */
    val signedOut: Boolean = false,
) {
    /** When they were fetched, in ms. */
    val at get() = ((fetched ?: 0.0) + 978_307_200.0).times(1000).toLong().takeIf { fetched != null } ?: 0L
}

/** A widget on one of the pages, its tree at each size as `GET /v1/widgets` gave it. */
@Serializable
data class KeptWidget(
    val scope: String,
    /** Its page's name. */
    val page: String,
    val title: String,
    val icon: String? = null,
    val color: String? = null,
    val image: String? = null,
    val imageKind: String? = null,
    /** Whether its app has a screen to open. */
    val opens: Boolean = false,
    val app: String,
    val widget: String,
    val error: String? = null,
    val sizes: Map<String, WidgetNode>? = null,
    val root: WidgetNode? = null,
) {
    /** `<scope>/<app>.<widget>`: what a home-screen widget's choice is found again by. */
    val id get() = "$scope/$app.$widget"
    fun decoded(): WidgetInfo? = root?.let { WidgetInfo(app, widget, title, sizes, error, it) }
}

object HomeWidgets {
    /** What every home-screen widget draws from. */
    data class Shown(
        val kept: Kept? = null,
        val choices: Map<Int, String> = emptyMap(),
        val signedIn: Boolean = false,
        /** Buttons and toggles tapped whose tools aren't done: each toggle's value, as it's drawn till then. */
        val doing: Map<String, Boolean> = emptyMap(),
    )

    val shown = MutableStateFlow(Shown())
    private lateinit var app: Application
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val dir get() = File(app.filesDir, "widgets")
    private val prefs get() = app.getSharedPreferences("home-widgets", Context.MODE_PRIVATE)

    /** Whether someone's signed in here: their refresh token is kept (Host.kt). */
    private fun signedIn(): Boolean = app.getSharedPreferences("sign-in", Context.MODE_PRIVATE).contains("refresh")

    fun start(context: Context) {
        if (::app.isInitialized) return
        app = context.applicationContext as Application
        shown.value = Shown(null, choices(), signedIn())
        // Read in the background, not as the app opens.
        scope.launch {
            Glyphs.load(app)
            load()
            read.complete(Unit)
        }
    }

    /** Done once what was kept is read, for the home screen, which draws once. */
    val read = CompletableDeferred<Unit>()

    private fun load() {
        val kept = runCatching { json.decodeFromString(Kept.serializer(), File(dir, "widgets.json").readText()) }.getOrNull()
        shown.update { it.copy(kept = kept?.takeIf { k -> !k.signedOut }, signedIn = signedIn() && kept?.signedOut != true) }
    }

    /** What the app's library kept changed (it says so after each refresh, or a tool a widget ran): read again, and drawn. */
    fun reload(context: Context) {
        start(context)
        scope.launch {
            read.await()
            Glyphs.load(app)
            load()
            redraw()
        }
    }

    private fun choices(): Map<Int, String> =
        prefs.all.mapNotNull { (k, v) -> k.removePrefix("choice.").toIntOrNull()?.let { id -> (v as? String)?.let { id to it } } }.toMap()

    /** Which widget a home-screen widget shows, chosen by long-pressing it, and its title, to say if it goes. */
    fun choose(widgetId: Int, id: String, title: String) {
        prefs.edit().putString("choice.$widgetId", id).putString("title.$widgetId", title).apply()
        shown.value = shown.value.copy(choices = choices())
        redraw()
    }

    fun title(widgetId: Int): String? = prefs.getString("title.$widgetId", null)

    /** A button or toggle tapped: drawn so at once. */
    suspend fun doing(which: String, on: Boolean) {
        shown.update { it.copy(doing = it.doing + (which to on)) }
        runCatching { HomeWidget().updateAll(app) }
    }

    /** Its tool done, or not: drawn from what the widgets say. */
    fun done(which: String) {
        shown.update { it.copy(doing = it.doing - which) }
        redraw()
    }

    fun forget(widgetIds: IntArray) {
        prefs.edit().apply { widgetIds.forEach { remove("choice.$it"); remove("title.$it") } }.apply()
        shown.value = shown.value.copy(choices = choices())
    }

    fun redraw() {
        scope.launch { runCatching { HomeWidget().updateAll(app) } }
    }

    /** Asks the app's library to fetch every page's widgets again and keep them, then draws them. */
    suspend fun refresh() {
        withContext(Dispatchers.IO) { runCatching { JSONObject(Native.call("widgets", "{}")) } }
        reload(app)
    }

    /** A button or toggle's tool, run by the app's library as the person → the error, if it didn't work. */
    suspend fun run(app: String, tool: String, input: JsonElement): String? = withContext(Dispatchers.IO) {
        val said = JSONObject(Native.call("call", JSONObject().put("app", app).put("tool", tool).put("input", JSONObject(input.toString())).toString()))
        said.optString("error").ifEmpty { null }
    }

    // MARK: Opening the app from a widget

    /** `<cli>://open/?scope=…&app=…&screen=…&params=…`: the widget's page, and its screen if it has one. */
    fun link(w: KeptWidget, screen: String?, params: JsonElement?): Uri = Uri.Builder().scheme(BuildConfig.CLI).authority("open")
        .appendQueryParameter("scope", w.scope).appendQueryParameter("app", w.app)
        .apply { if (screen != null) appendQueryParameter("screen", screen) }
        .apply { if (params != null) appendQueryParameter("params", params.toString()) }
        .build()

    /** Whether any is on the home screen, so the app keeps them up to date. */
    fun placed(context: Context): Boolean =
        AppWidgetManager.getInstance(context).getAppWidgetIds(ComponentName(context, HomeWidgetReceiver::class.java)).isNotEmpty()

    // MARK: Images

    /** Where an image a widget shows was saved by the app's library: the home screen can't download. */
    fun imageFile(url: String): File {
        val name = MessageDigest.getInstance("SHA-256").digest(url.toByteArray()).take(12).joinToString("") { "%02x".format(it) }
        return File(File(dir, "images"), name)
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
            if (signedIn()) refresh() else reload(applicationContext)
            return Result.success()
        }
    }
}
