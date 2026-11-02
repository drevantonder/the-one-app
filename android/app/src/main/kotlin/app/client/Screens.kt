package app.client

import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.util.Base64
import android.view.HapticFeedbackConstants
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.ui.graphics.Color
import androidx.compose.material3.HorizontalDivider
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.windowInsetsTopHeight
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.shadow
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.FileProvider
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.coroutines.resume

// Every screen the app shows is a page the node serves (src/screen/), the same on every client: the pages, Services, a
// Vault request and Settings' (the client's own), and apps'. Each is in a web view of its own, and this is the shell
// around them: the pages as the home, what they open pushed over them with Back or in a sheet, and the bridge each
// page talks through (src/screen/bridge.js), answered natively wherever a person touches: menus, alerts, sheets, files,
// scanning, links. Screens are untrusted: a page gets an origin of its own that's nowhere (ORIGIN), never this app's or
// the install's; it reaches nothing but the bridge, a message listener only that origin's main frame is given; an
// app's screen acts only as its app, and only the client's own (a route this app chose, never the page) as the person;
// it never navigates, and its links open in the browser, the install's own in a Custom Tab.

private const val ORIGIN = "https://screen.invalid"

/** Before the page's scripts: the bridge's way out (`__oneShell.postMessage`, src/screen/bridge.js), one JSON message
 *  each way with an id, and the look that makes the page sit in the app as its own. */
private fun shim(accent: String, dark: Boolean, scan: Boolean, zoom: Float) = """
(() => {
  const port = window.oneShell;
  if (!port) return;
  let asked = 0;
  const waiting = {};
  port.onmessage = (e) => {
    const r = JSON.parse(e.data), w = waiting[r.id];
    if (!w) return;
    delete waiting[r.id];
    "error" in r ? w.fail(new Error(r.error)) : w.ok(r.ok ?? null);
  };
  window.__oneShell = { scan: $scan, postMessage: (message) => new Promise((ok, fail) => {
    const id = ++asked;
    waiting[id] = { ok, fail };
    port.postMessage(JSON.stringify({ id, message }));
  }) };
  const style = document.createElement("style");
  style.textContent = `html, body { background: transparent !important; }
    html { -webkit-tap-highlight-color: transparent; -webkit-touch-callout: none; }
    :root:root { --accent: ${css(accent, dark)}; }`;
  document.documentElement.appendChild(style);
  addEventListener("DOMContentLoaded", () => document.querySelector('meta[name="viewport"]')?.setAttribute("content", "width=device-width,initial-scale=1,maximum-scale=1,viewport-fit=cover"));
  // The kit is sized for the phone's usual text: at a larger or smaller font size, the page is zoomed to it, as an
  // iPhone's follows Dynamic Type, though never to less than the 320 CSS pixels wide phone layouts are made for.
  const zoom = $zoom;
  if (zoom !== 1) {
    const fit = () => { document.documentElement.style.zoom = String(innerWidth ? Math.min(zoom, innerWidth / 320) : zoom); };
    addEventListener("resize", fit);
    addEventListener("DOMContentLoaded", fit);
    addEventListener("load", fit);
  }
})();
"""

private fun css(accent: String, dark: Boolean) = accentColor(accent, dark).let { "rgb(${(it.red * 255).toInt()}, ${(it.green * 255).toInt()}, ${(it.blue * 255).toInt()})" }

// MARK: Where the app is

/** The pages, as the home; what they open pushed over them; and the sheets over everything. */
@Composable
fun Shell(store: Store) {
    val p = LocalPalette.current
    Box(Modifier.fillMaxSize().background(p.background)) {
        // Every page side by side then Settings, to swipe between, opened at the page asked for. Its dots sit at its
        // bottom, above the gesture bar.
        Box(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.systemBars)) {
            if (store.phase == Store.Phase.Ready && store.install != null) {
                key(store.opened) { ScreenView(store, Route.Own("pages", obj("swipe" to true, "scope" to store.page).toString())) }
            } else Starting(store)
        }
        BackHandler(enabled = store.path.isNotEmpty()) { store.path.removeAt(store.path.lastIndex) }
        AnimatedContent(
            targetState = store.path.lastOrNull(),
            transitionSpec = {
                val forward = targetState != null && (initialState == null || store.path.size > 1)
                if (forward) (slideInHorizontally { it / 3 } + fadeIn()) togetherWith (slideOutHorizontally { -it / 6 } + fadeOut())
                else (slideInHorizontally { -it / 6 } + fadeIn()) togetherWith (slideOutHorizontally { it / 3 } + fadeOut())
            },
            label = "pushed",
        ) { pushed -> if (pushed != null) PushedView(store, pushed) }
        for (sheet in store.sheets) key(sheet) { SheetView(store, sheet) }
    }
}

/** Before the node has answered: a spinner, or that it can't be reached and keeps trying. */
@Composable
private fun Starting(store: Store) {
    val p = LocalPalette.current
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        if (store.phase == Store.Phase.Unreachable) Unavailable("Can’t Reach Your Devices", (store.failure ?: "") + "\nIt keeps trying.")
        else if (store.state != null && store.install == null) Unavailable("Nothing Here Yet", "It isn’t set up yet. Set it up from the ${BuildConfig.NAME} app on one of your devices.")
        else CircularProgressIndicator(color = p.secondary)
    }
}

/** A pushed screen: its bar, with Back and its title, over the screen. */
@Composable
private fun PushedView(store: Store, pushed: Pushed) {
    val p = LocalPalette.current
    var title by remember(pushed) { mutableStateOf<String?>(null) }
    Column(Modifier.fillMaxSize().background(p.background)) {
        Spacer(Modifier.windowInsetsTopHeight(WindowInsets.statusBars))
        Box(Modifier.fillMaxWidth().height(48.dp)) {
            IconButton({ store.path.remove(pushed) }, Modifier.align(Alignment.CenterStart)) {
                Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back", tint = p.accent)
            }
            Text(
                title ?: store.title(pushed.route), style = Type.headline, color = p.label, maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center,
                modifier = Modifier.align(Alignment.Center).padding(horizontal = 56.dp),
            )
        }
        // Above the gesture bar, and the keyboard while it's up.
        Box(Modifier.weight(1f).fillMaxWidth().windowInsetsPadding(WindowInsets.navigationBars.union(WindowInsets.ime))) {
            ScreenView(store, pushed.route, pushed = pushed) { title = it }
        }
    }
}

/** A screen in a sheet over everything: a bar with its title and Done (Cancel for a request, answered in it or not). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SheetView(store: Store, sheet: Sheet) {
    val p = LocalPalette.current
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var title by remember(sheet) { mutableStateOf<String?>(null) }
    fun gone() {
        store.sheets.remove(sheet)
        sheet.done.complete(sheet.value)
    }
    LaunchedEffect(sheet.going) {
        if (!sheet.going) return@LaunchedEffect
        runCatching { state.hide() }
        gone()
    }
    // Dragging would fight the page's own scrolling: it goes by its button, Back, or a tap above it.
    // Below the status bar, the page under it showing above.
    ModalBottomSheet(
        onDismissRequest = ::gone, modifier = Modifier.windowInsetsPadding(WindowInsets.statusBars).padding(top = 12.dp), sheetState = state,
        sheetGesturesEnabled = false, dragHandle = null, containerColor = p.background,
    ) {
        Box(Modifier.fillMaxWidth().height(52.dp)) {
            Text(
                title?.ifEmpty { null } ?: store.title(sheet.route), style = Type.headline, color = p.label, maxLines = 1, overflow = TextOverflow.Ellipsis,
                textAlign = TextAlign.Center, modifier = Modifier.align(Alignment.Center).padding(horizontal = 90.dp),
            )
            TextButton({ sheet.close(null) }, Modifier.align(Alignment.CenterEnd).padding(end = 8.dp)) {
                Text(if ((sheet.route as? Route.Own)?.name == "request") "Cancel" else "Done", style = Type.body, fontWeight = FontWeight.SemiBold, color = p.accent)
            }
        }
        Box(Modifier.fillMaxSize().imePadding()) { ScreenView(store, sheet.route, sheet = sheet) { title = it } }
    }
}

// MARK: A screen

/** A screen's page in its web view, with the menus and alerts it asks for. */
@Composable
private fun ScreenView(store: Store, route: Route, sheet: Sheet? = null, pushed: Pushed? = null, setTitle: (String?) -> Unit = {}) {
    val context = LocalContext.current
    val palette = LocalPalette.current
    // Drawn again in this app's light or dark, once it changes: a web view takes the scheme its theme says.
    val dark = palette.dark
    var loaded by remember(route, dark) { mutableStateOf(false) }
    val accent = store.config?.accent ?: BuildConfig.ACCENT
    val bridge = remember(route, dark) { Bridge(context, store, route, sheet, pushed, setTitle) }
    DisposableEffect(bridge) { onDispose { bridge.close() } }

    key(dark) { Box(Modifier.fillMaxSize()) {
        AndroidView(
            // Below the bar saying it's offline, while it is.
            modifier = Modifier.fillMaxSize().padding(top = if (bridge.offline != null) 64.dp else 0.dp).alpha(if (loaded) 1f else 0f),
            factory = { ctx ->
                val themed = android.view.ContextThemeWrapper(ctx, if (dark) android.R.style.Theme_DeviceDefault else android.R.style.Theme_DeviceDefault_Light)
                bridge.make(themed, shim(accent, dark, Scanner.available(ctx), ctx.resources.configuration.fontScale)) { loaded = true }.also { bridge.load() }
            },
        )
        bridge.failure?.let { Unavailable("Can’t Open This", it) }
        bridge.offline?.let { Offline(it, bridge::tryNow) }
        if (!loaded && bridge.failure == null) {
            var slow by remember { mutableStateOf(false) }
            LaunchedEffect(Unit) { delay(400); slow = true }
            if (slow) CircularProgressIndicator(Modifier.align(Alignment.Center), color = palette.secondary)
        }
        bridge.menu?.let { Menu(it) }
        bridge.alert?.let { Alert(it) }
    } }
}

/** Over a screen shown as it last looked: since when, and when it's tried again, or Try Now. */
@Composable
private fun BoxScope.Offline(offline: Bridge.Offline, tryNow: () -> Unit) {
    val p = LocalPalette.current
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(offline) {
        while (true) {
            now = System.currentTimeMillis()
            delay(1000)
        }
    }
    val ago = if (now - offline.kept < 60_000) "just now"
    else android.text.format.DateUtils.getRelativeTimeSpanString(offline.kept, now, android.text.format.DateUtils.MINUTE_IN_MILLIS).toString()
    Row(
        Modifier.align(Alignment.TopCenter).padding(8.dp).shadow(6.dp, RoundedCornerShape(50)).background(p.card, RoundedCornerShape(50)).padding(start = 14.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Symbol("wifi-slash", p.secondary, 16.dp)
        Text("Offline: this is how it looked ${ago.replaceFirstChar { it.lowercase() }}.", style = Type.footnote, fontWeight = FontWeight.Medium, color = p.secondary, modifier = Modifier.weight(1f, fill = false))
        val next = offline.next
        if (next != null) {
            Text("Reconnecting in ${maxOf(1, ((next - now + 999) / 1000).toInt())}s", style = Type.footnote, color = p.tertiary)
            TextButton(tryNow) { Text("Try Now", style = Type.footnote, fontWeight = FontWeight.SemiBold, color = p.accent) }
        } else {
            CircularProgressIndicator(Modifier.size(14.dp), color = p.secondary, strokeWidth = 2.dp)
            Text("Reconnecting…", style = Type.footnote, color = p.tertiary, modifier = Modifier.padding(end = 12.dp, top = 14.dp, bottom = 14.dp))
        }
    }
}

/** `one.menu`: the items at the point the page asked for, or from the element it's for (`rect`: x, y, width,
 *  height), opening away from the edge it's near (CSS pixels are dp, at the scale screens are drawn). */
private class MenuAsk(val items: List<MenuItem>, val x: Int, val y: Int, val rect: List<Int>?, val chose: CompletableDeferred<Int?> = CompletableDeferred())
private data class MenuItem(val title: String, val danger: Boolean, val checked: Boolean, val icon: String?, val row: Boolean, val section: Boolean)

@Composable
private fun Menu(ask: MenuAsk) {
    val p = LocalPalette.current
    val marks = ask.items.any { it.checked && !it.row }
    val row = ask.items.withIndex().filter { it.value.row }
    val anchor = ask.rect?.let { Modifier.offset(it[0].dp, it[1].dp).size(it[2].coerceAtLeast(1).dp, it[3].coerceAtLeast(1).dp) } ?: Modifier.offset(ask.x.dp, ask.y.dp).size(1.dp)
    Box(anchor) {
        DropdownMenu(expanded = true, onDismissRequest = { ask.chose.complete(null) }) {
            // A widget's sizes, side by side as their shapes, the one it is filled in.
            if (row.isNotEmpty()) {
                Row(Modifier.padding(horizontal = 8.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    row.forEach { (i, item) ->
                        IconButton({ ask.chose.complete(i) }, Modifier.size(52.dp, 40.dp).background(if (item.checked) p.accent else p.fill, RoundedCornerShape(10.dp))) {
                            if (item.icon != null) Symbol(item.icon, if (item.checked) Color.White else p.label, 20.dp)
                            else Text(item.title.take(1), color = if (item.checked) Color.White else p.label)
                        }
                    }
                }
                HorizontalDivider(color = p.separator)
            }
            ask.items.forEachIndexed { i, item ->
                if (item.row) return@forEachIndexed
                if (item.section && i > row.size) HorizontalDivider(color = p.separator)
                val tint = if (item.danger) p.red else p.label
                DropdownMenuItem(
                    text = { Text(item.title, color = tint) },
                    leadingIcon = if (marks) ({ if (item.checked) Icon(Icons.Rounded.Check, null, tint = p.label) else Spacer(Modifier.size(24.dp)) }) else null,
                    trailingIcon = item.icon?.let { { Symbol(it, tint, 20.dp) } },
                    onClick = { ask.chose.complete(i) },
                )
            }
        }
    }
}

/** `one.alert`: its title and text, then OK; or Cancel and its action → whether that was chosen. */
private class AlertAsk(val title: String, val text: String, val action: String?, val danger: Boolean, val chose: CompletableDeferred<Boolean> = CompletableDeferred())

@Composable
private fun Alert(ask: AlertAsk) {
    val p = LocalPalette.current
    AlertDialog(
        onDismissRequest = { ask.chose.complete(false) },
        title = ask.title.takeIf { it.isNotEmpty() }?.let { { Text(it) } },
        text = ask.text.takeIf { it.isNotEmpty() }?.let { { Text(it) } },
        confirmButton = {
            TextButton({ ask.chose.complete(ask.action != null) }) { Text(ask.action ?: "OK", color = if (ask.danger) p.red else p.accent, fontWeight = FontWeight.SemiBold) }
        },
        dismissButton = ask.action?.let { { TextButton({ ask.chose.complete(false) }) { Text("Cancel", color = p.accent) } } },
    )
}

// MARK: The bridge

/** A screen's web view, and the bridge it answers. */
private class Bridge(val context: Context, val store: Store, val route: Route, val sheet: Sheet?, val pushed: Pushed?, val setTitle: (String?) -> Unit) {
    /** One of the client's own screens, which act as the person; else an app's, which act only as that app. */
    private val own = route is Route.Own
    private val app = route as? Route.App
    val scope: CoroutineScope = MainScope()
    private var web: WebView? = null
    var menu by mutableStateOf<MenuAsk?>(null)
        private set
    var alert by mutableStateOf<AlertAsk?>(null)
        private set

    /** Why the screen can't be shown, with no copy of it to show instead. */
    var failure by mutableStateOf<String?>(null)
        private set

    /** An app's screen shown as it last looked, since `kept`, while the node can't be reached: asked again at `next`. */
    data class Offline(val kept: Long, val next: Long?)
    var offline by mutableStateOf<Offline?>(null)
        private set
    /** The node's tries since it last answered, for the wait before the next: 2, 4, 8, 16, then every 30 seconds. */
    private var misses = 0
    private var loading: Job? = null

    /** Loads the screen's page; offline, an app's screen shows its last copy, saying so, and asks again and again,
     *  waiting longer each time, as the iPhone's does. */
    fun load() {
        val web = web ?: return
        loading?.cancel()
        loading = scope.launch {
            val page = try {
                store.screen(route)
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                val app = app
                val kept = offline?.kept ?: app?.let { ScreenCache.last(it) }?.let { (html, time) ->
                    web.loadDataWithBaseURL("$ORIGIN/", html, "text/html", "utf-8", null)
                    time
                }
                if (kept == null || (e as? NodeError)?.signedOut == true) {
                    failure = e.message ?: "Can’t reach ${store.signIn.address}."
                    return@launch
                }
                misses++
                val wait = minOf(30, 1 shl minOf(misses, 5)) * 1000L
                offline = Offline(kept, System.currentTimeMillis() + wait)
                delay(wait)
                offline = Offline(kept, null)
                load()
                return@launch
            }
            web.loadDataWithBaseURL("$ORIGIN/", page, "text/html", "utf-8", null)
            app?.let { ScreenCache.keep(it, page) }
            misses = 0
            offline = null
        }
    }

    fun tryNow() {
        val o = offline ?: return
        offline = Offline(o.kept, null)
        load()
    }

    @SuppressLint("SetJavaScriptEnabled")
    fun make(ctx: Context, shim: String, loaded: () -> Unit): WebView {
        val web = WebView(ctx)
        // Its own size, not its content's: a web view laid out to wrap its content takes 0 as the height `vh` is of.
        web.layoutParams = android.view.ViewGroup.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT, android.view.ViewGroup.LayoutParams.MATCH_PARENT)
        this.web = web
        web.setBackgroundColor(android.graphics.Color.TRANSPARENT)
        web.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            setSupportMultipleWindows(false)
            javaScriptCanOpenWindowsAutomatically = false
            setGeolocationEnabled(false)
            mediaPlaybackRequiresUserGesture = true
            // The page zooms as a whole to the phone's font size (shim), not its text alone, which overflows its boxes.
            textZoom = 100
        }
        val rules = setOf(ORIGIN)
        check(WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) && WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
            "This phone’s web view is too old to show screens: update Android System WebView."
        }
        WebViewCompat.addDocumentStartJavaScript(web, shim, rules)
        WebViewCompat.addWebMessageListener(web, "oneShell", rules) { _, message, origin, isMainFrame, reply ->
            if (!isMainFrame || origin.toString().trimEnd('/') != ORIGIN) return@addWebMessageListener
            val body = runCatching { json.parseToJsonElement(message.data ?: "").jsonObject }.getOrNull() ?: return@addWebMessageListener
            val id = body["id"] ?: return@addWebMessageListener
            val m = body["message"] as? JsonObject ?: return@addWebMessageListener
            scope.launch {
                val out = try {
                    obj("id" to id, "ok" to (answer(m) ?: JsonNull))
                } catch (e: Exception) {
                    obj("id" to id, "error" to (e.message ?: "It didn’t work"))
                }
                runCatching { reply.postMessage(out.toString()) }
            }
        }
        web.webViewClient = object : WebViewClient() {
            // The page never goes anywhere: links open in the browser.
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                follow(request.url)
                return true
            }

            // Its own origin is nowhere; what it asks for there is only the node's files, by their hash.
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
                val url = request.url
                if (url.scheme != "https" || url.host != "screen.invalid") return null
                val hash = url.path?.removePrefix("/v1/blobs/")?.takeIf { url.path!!.startsWith("/v1/blobs/") && request.method == "GET" }
                    ?: return WebResourceResponse("text/plain", null, 404, "Not Found", emptyMap(), null)
                val bytes = runCatching { runBlocking { store.blob(hash) } }.getOrNull()
                    ?: return WebResourceResponse("text/plain", null, 404, "Not Found", emptyMap(), null)
                return WebResourceResponse("application/octet-stream", null, bytes.inputStream())
            }

            override fun onPageFinished(view: WebView, url: String?) = loaded()
        }
        // `<input type=file>`: the phone's own picker.
        web.webChromeClient = object : WebChromeClient() {
            override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<Uri>>, params: FileChooserParams): Boolean {
                val activity = context as? ComponentActivity ?: return false
                scope.launch { callback.onReceiveValue(runCatching { choose(activity, params) }.getOrNull()) }
                return true
            }
        }
        // Node events reach the page as `one.__event(event)`.
        scope.launch { store.events.collect { e -> web.evaluateJavascript("window.one && one.__event($e)", null) } }
        return web
    }

    fun close() {
        menu?.chose?.complete(null)
        alert?.chose?.complete(false)
        scope.cancel()
        web?.destroy()
    }

    private fun str(m: JsonObject, k: String) = (m[k] as? JsonPrimitive)?.contentOrNull

    /** A web address: the install's own in a Custom Tab, signed in as the browser is; anything else where the phone
     *  opens it. */
    private fun follow(url: Uri) {
        val address = store.signIn.address?.let(Uri::parse)
        if (address != null && url.scheme == address.scheme && url.authority == address.authority) SignIn.openInTab(context, url)
        else if (url.scheme in listOf("http", "https", "mailto", "tel")) runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, url).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    }

    /** Each of the bridge's messages (src/screen/bridge.js's list; tests/contracts.rs holds this to it). */
    private suspend fun answer(m: JsonObject): JsonElement? {
        val type = str(m, "type")
        if (type in listOf("api", "signOut", "appearance", "device", "request", "cache") && !own) throw NodeError("Only the app’s own screens can do that.")
        when (type) {
            "call" -> {
                val app = app ?: throw NodeError("This screen has no app to call.")
                return store.call(app.app, str(m, "op") ?: "", m["input"] ?: JsonObject(emptyMap()))
            }
            "open" -> {
                val params = (m["params"] ?: JsonObject(emptyMap())).toString()
                val screen = str(m, "screen")?.ifEmpty { null }
                val other = str(m, "app")?.ifEmpty { null }
                val scope = str(m, "scope")?.ifEmpty { null } ?: app?.scope
                val to = when {
                    other != null -> Route.App(other, screen ?: "main", params, scope)
                    app != null -> Route.App(app.app, screen ?: "main", params, scope)
                    else -> Route.Own(screen ?: "", params)
                }
                if ((m["sheet"] as? JsonPrimitive)?.booleanOrNull == true) return store.present(to)
                // From a sheet, the sheet goes and it opens over the pages.
                sheet?.close(null)
                store.push(to)
                return null
            }
            "close" -> {
                if (sheet != null) sheet.close(m["value"]?.takeIf { it !is JsonNull })
                else if (pushed != null) store.path.remove(pushed)
                return null
            }
            "title" -> {
                setTitle(str(m, "text"))
                return null
            }
            "menu" -> {
                val flag = { o: JsonObject, k: String -> (o[k] as? JsonPrimitive)?.booleanOrNull == true }
                val items = (m["items"] as? JsonArray)?.mapNotNull { it as? JsonObject }?.map {
                    MenuItem(str(it, "title") ?: "", flag(it, "danger"), flag(it, "checked"), str(it, "icon")?.ifEmpty { null }, flag(it, "row"), flag(it, "section"))
                }.orEmpty()
                if (items.isEmpty()) return JsonNull
                val number = { k: String -> (m[k] as? JsonPrimitive)?.doubleOrNull?.toInt() ?: 0 }
                val rect = (m["rect"] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.doubleOrNull?.toInt() }?.takeIf { it.size == 4 }
                val ask = MenuAsk(items, number("x"), number("y"), rect)
                menu?.chose?.complete(null)
                menu = ask
                val chose = try { ask.chose.await() } finally { if (menu == ask) menu = null }
                return chose?.let(::JsonPrimitive) ?: JsonNull
            }
            "alert" -> {
                val ask = AlertAsk(str(m, "title") ?: "", str(m, "text") ?: "", str(m, "action")?.ifEmpty { null }, (m["danger"] as? JsonPrimitive)?.booleanOrNull == true)
                alert?.chose?.complete(false)
                alert = ask
                return JsonPrimitive(try { ask.chose.await() } finally { if (alert == ask) alert = null })
            }
            "save", "preview" -> {
                file(type, m["file"] as? JsonObject ?: throw NodeError("No file to show."))
                return null
            }
            "copy" -> {
                val clipboard = context.getSystemService(ClipboardManager::class.java)
                clipboard.setPrimaryClip(ClipData.newPlainText("", str(m, "text") ?: ""))
                return null
            }
            "link" -> {
                follow(Uri.parse(str(m, "url") ?: ""))
                return null
            }
            "scan" -> return Scanner.scan(context as? ComponentActivity ?: throw NodeError("This phone can’t scan documents here."))
            "vault" -> {
                val app = app ?: throw NodeError("This screen has no app to ask for.")
                val scope = app.scope ?: "user"
                val out = store.post("/v1/vault/request", obj("scope" to scope, "app" to app.app, "need" to (str(m, "need") ?: ""))) as? JsonObject
                val id = (out?.get("id") as? JsonPrimitive)?.contentOrNull ?: throw NodeError("The Vault didn’t take the request")
                return JsonPrimitive(request(scope, id))
            }
            "request" -> return JsonPrimitive(request(str(m, "scope") ?: "", str(m, "id") ?: ""))
            "icons" -> {
                val names = (m["names"] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull } ?: emptyList()
                return JsonObject(names.mapNotNull { n -> Glyphs.data(n)?.let { n to JsonPrimitive(it) } }.toMap())
            }
            "icon" -> {
                val look = TileLook(str(m, "icon")?.ifEmpty { null }, str(m, "seed") ?: "", str(m, "color")?.ifEmpty { null }, str(m, "image")?.ifEmpty { null }, str(m, "kind"))
                val size = ((m["size"] as? JsonPrimitive)?.doubleOrNull ?: 36.0) * context.resources.displayMetrics.density
                return JsonPrimitive(png(tileBitmap(look, size.toInt().coerceIn(8, 512))))
            }
            "haptic" -> {
                web?.performHapticFeedback(
                    when (str(m, "style")) {
                        "medium" -> HapticFeedbackConstants.LONG_PRESS
                        "success" -> HapticFeedbackConstants.CONFIRM
                        "error" -> HapticFeedbackConstants.REJECT
                        else -> HapticFeedbackConstants.CLOCK_TICK
                    },
                )
                return null
            }
            "cache" -> {
                val key = str(m, "key") ?: ""
                if (!m.containsKey("value")) return store.kept(key)?.let { runCatching { json.parseToJsonElement(it) }.getOrNull() } ?: JsonNull
                store.keep(key, m["value"].toString())
                return null
            }
            "api" -> {
                val path = str(m, "path") ?: ""
                val bare = Uri.parse(path).path ?: ""
                if (!bare.startsWith("/v1/") || ".." in bare) throw NodeError("That isn’t on your devices.")
                val method = (str(m, "method") ?: "GET").uppercase()
                var body = m["body"]?.takeIf { it !is JsonNull && method != "GET" } ?: if (method == "GET") null else JsonObject(emptyMap())
                if (bare == "/v1/call" && body is JsonObject) body = JsonObject(body + ("client" to JsonPrimitive("android")))
                return json.parseToJsonElement(String(store.api(path, method, body)).ifEmpty { "null" })
            }
            "signOut" -> {
                store.signOut()
                return null
            }
            "appearance" -> {
                str(m, "value")?.let { v -> Appearance.entries.firstOrNull { it.name.equals(v, ignoreCase = true) }?.let(store::choose) }
                return JsonPrimitive(store.appearance.name.lowercase())
            }
            "device" -> {
                val device = store.install?.devices?.firstOrNull { it.key == store.state?.host?.key }
                val build = "${store.config?.name ?: BuildConfig.NAME}, app ${BuildConfig.COMMIT}" + (device?.build?.let { "\n${device.name} runs $it" } ?: "")
                return obj("name" to SignIn.deviceName(context), "build" to build, "signOut" to true)
            }
            else -> throw NodeError("Unknown message")
        }
    }

    /** A request for a Vault item, in the client's own sheet → whether it was allowed, there or somewhere else. */
    private suspend fun request(scope: String, id: String): Boolean =
        store.present(Route.Own("request", obj("scope" to scope, "id" to id).toString())) == JsonPrimitive(true)

    private fun png(bitmap: Bitmap): String {
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
        return "data:image/png;base64," + Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)
    }

    /** `one.save` shares a file, as the iPhone's share sheet does; `one.preview` shows it in what views its kind. */
    private suspend fun file(how: String, f: JsonObject) {
        val bytes = when {
            f["base64"] != null -> Base64.decode(f["base64"]!!.jsonPrimitive.content, Base64.DEFAULT)
            f["blob"] != null || f["file"] != null -> store.blob((f["blob"] ?: f["file"])!!.jsonPrimitive.content)
            else -> throw NodeError("No file to show.")
        }
        val name = (f["name"] as? JsonPrimitive)?.contentOrNull?.replace('/', '_')?.ifEmpty { null } ?: "file"
        val type = (f["type"] as? JsonPrimitive)?.contentOrNull?.ifEmpty { null } ?: "application/octet-stream"
        val dir = File(context.cacheDir, "files").apply { mkdirs() }
        val file = File(dir, name)
        withContext(Dispatchers.IO) { file.writeBytes(bytes) }
        val uri: Uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        val intent = if (how == "preview") Intent(Intent.ACTION_VIEW).setDataAndType(uri, type)
        else Intent.createChooser(Intent(Intent.ACTION_SEND).setType(type).putExtra(Intent.EXTRA_STREAM, uri), name)
        context.startActivity(intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    /** The files a person picks for `<input type=file>` → none if they cancel. */
    private suspend fun choose(activity: ComponentActivity, params: WebChromeClient.FileChooserParams): Array<Uri>? {
        val intent = params.createIntent().apply { if (params.mode == WebChromeClient.FileChooserParams.MODE_OPEN_MULTIPLE) putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true) }
        val result = suspendCancellableCoroutine { done ->
            lateinit var launcher: androidx.activity.result.ActivityResultLauncher<Intent>
            launcher = activity.activityResultRegistry.register("files-${++asked}", ActivityResultContracts.StartActivityForResult()) {
                launcher.unregister()
                done.resume(it)
            }
            done.invokeOnCancellation { launcher.unregister() }
            launcher.launch(intent)
        }
        val data = result.data ?: return null
        if (result.resultCode != android.app.Activity.RESULT_OK) return null
        val clip = data.clipData
        return if (clip != null) Array(clip.itemCount) { clip.getItemAt(it).uri } else data.data?.let { arrayOf(it) }
    }

    companion object {
        private var asked = 0
    }
}

/** The last copy of each app screen this phone showed, which stands in while the node can't be reached; gone once
 *  the person signs out. */
object ScreenCache {
    lateinit var dir: File

    private fun file(r: Route.App) = File(dir, "${steadyHash(listOf(r.app, r.screen, r.params, r.scope ?: "").joinToString("\u0001"))}-${r.app.length + r.screen.length + r.params.length}.html")

    fun keep(r: Route.App, html: String) {
        runCatching {
            dir.mkdirs()
            file(r).writeText(html)
        }
    }

    /** Its page and when it was kept. */
    fun last(r: Route.App): Pair<String, Long>? = file(r).takeIf { it.isFile }?.let { runCatching { it.readText() to it.lastModified() }.getOrNull() }
}
