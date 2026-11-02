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
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.FileProvider
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.io.ByteArrayOutputStream
import java.io.File

// An app's screen, or a Settings screen: the page its node serves, in a web view of its own. Screens are untrusted: the
// page gets an origin of its own that's nowhere (ORIGIN), never this app's or the install's; it reaches nothing but the
// bridge, a message listener only that origin's main frame is given, answered here as the iPhone app's AppScreen.swift
// answers it, each call made as the screen's own app, and only a Settings screen's (a route this app chose, never the
// page) as the person; it never navigates, and its links open in the browser, the install's own in a Custom Tab.

private const val ORIGIN = "https://screen.invalid"

/** Before the page's scripts: the bridge's way out (`__oneShell.postMessage`, src/screen/bridge.js), one JSON message
 *  each way with an id, and the look that makes the page sit in the app as its own. */
private fun shim(accent: String, dark: Boolean) = """
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
  window.__oneShell = { scan: false, postMessage: (message) => new Promise((ok, fail) => {
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
})();
"""

private fun css(accent: String, dark: Boolean) = accentColor(accent, dark).let { "rgb(${(it.red * 255).toInt()}, ${(it.green * 255).toInt()}, ${(it.blue * 255).toInt()})" }

@Composable
fun AppScreen(store: Store, route: Route, setTitle: (String?) -> Unit) {
    val context = LocalContext.current
    val palette = LocalPalette.current
    // Drawn again in this app's light or dark, once it changes: a web view takes the scheme its theme says.
    val dark = palette.dark
    var loaded by remember(route, dark) { mutableStateOf(false) }
    var failure by remember(route, dark) { mutableStateOf<String?>(null) }
    val accent = store.config?.accent ?: BuildConfig.ACCENT
    val bridge = remember(route, dark) { Bridge(context, store, route, setTitle) }
    DisposableEffect(bridge) { onDispose { bridge.close() } }

    key(dark) { Box(Modifier.fillMaxSize()) {
        AndroidView(
            modifier = Modifier.fillMaxSize().alpha(if (loaded) 1f else 0f),
            factory = { ctx ->
                val themed = android.view.ContextThemeWrapper(ctx, if (dark) android.R.style.Theme_DeviceDefault else android.R.style.Theme_DeviceDefault_Light)
                bridge.make(themed, shim(accent, dark)) { loaded = true }.also { web ->
                    bridge.scope.launch {
                        try {
                            val html = if (route is Route.Setting) store.settingsScreen(route) else store.screen(route as Route.Screen)
                            web.loadDataWithBaseURL("$ORIGIN/", html, "text/html", "utf-8", null)
                        } catch (e: Exception) {
                            failure = e.message
                        }
                    }
                }
            },
        )
        failure?.let { Unavailable("Can’t Open This", it) }
        if (!loaded && failure == null) {
            var slow by remember { mutableStateOf(false) }
            LaunchedEffect(Unit) { delay(400); slow = true }
            if (slow) CircularProgressIndicator(Modifier.align(Alignment.Center), color = palette.secondary)
        }
    } }
}

/** A screen's web view, and the bridge it answers. */
private class Bridge(val context: Context, val store: Store, route: Route, val setTitle: (String?) -> Unit) {
    /** A Settings screen's name, for one; its app's screen, for the rest. */
    private val settings = (route as? Route.Setting)?.name
    private val screen = route as? Route.Screen ?: Route.Screen("")
    val scope: CoroutineScope = MainScope()
    private var web: WebView? = null

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
        }
        val rules = setOf(ORIGIN)
        check(WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) && WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
            "This phone’s web view is too old to show app screens: update Android System WebView."
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
                val url = request.url
                val address = store.signIn.address?.let(Uri::parse)
                if (address != null && url.scheme == address.scheme && url.authority == address.authority) SignIn.openInTab(context, url)
                else if (url.scheme in listOf("http", "https", "mailto", "tel")) runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, url).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
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
        // Node events reach the page as `one.__event(event)`.
        scope.launch { store.events.collect { e -> web.evaluateJavascript("window.one && one.__event($e)", null) } }
        return web
    }

    fun close() {
        scope.cancel()
        web?.destroy()
    }

    private fun str(m: JsonObject, k: String) = (m[k] as? JsonPrimitive)?.contentOrNull

    /** The bridge: `{ type: "call" | "open" | "title" | … }`, and for Settings screens alone, `"api" | "signOut" |
     *  "appearance" | "device" | "request"`. A page only ever calls its own app, but may open another's screens. */
    private suspend fun answer(m: JsonObject): JsonElement? {
        val type = str(m, "type")
        if (type in listOf("api", "signOut", "appearance", "device", "request") && settings == null) throw NodeError("Only Settings can do that.")
        when (type) {
            "api" -> {
                val path = str(m, "path") ?: ""
                val bare = Uri.parse(path).path ?: ""
                if (!bare.startsWith("/v1/") || ".." in bare) throw NodeError("That isn’t your server’s.")
                val method = (str(m, "method") ?: "GET").uppercase()
                val body = m["body"]?.takeIf { it !is JsonNull && method != "GET" } ?: if (method == "GET") null else JsonObject(emptyMap())
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
                val server = store.install?.servers?.firstOrNull { it.key == store.state?.host?.key }
                val build = "${store.config?.name ?: BuildConfig.NAME}, app ${BuildConfig.COMMIT}" + (server?.build?.let { "\n${server.name} runs $it" } ?: "")
                return obj("name" to SignIn.deviceName(context), "build" to build, "signOut" to true)
            }
            "request" -> return JsonPrimitive(ask(str(m, "scope") ?: "", str(m, "id") ?: ""))
            "call" -> {
                if (settings != null) throw NodeError("Settings has no app to call.")
                return store.call(screen.app, str(m, "op") ?: "", m["input"] ?: JsonObject(emptyMap()))
            }
            "open" -> {
                val params = (m["params"] ?: JsonObject(emptyMap())).toString()
                val app = str(m, "app")?.ifEmpty { null }
                store.path.add(if (settings != null && app == null) Route.Setting(str(m, "screen") ?: "", params) else Route.Screen(app ?: screen.app, str(m, "screen") ?: "main", params, screen.scope))
                return null
            }
            "title" -> {
                setTitle(str(m, "text"))
                return null
            }
            "save", "preview" -> {
                file(str(m, "type")!!, m["file"] as? JsonObject ?: throw NodeError("No file to show."))
                return null
            }
            "copy" -> {
                val clipboard = context.getSystemService(ClipboardManager::class.java)
                clipboard.setPrimaryClip(ClipData.newPlainText("", str(m, "text") ?: ""))
                return null
            }
            "vault" -> return JsonPrimitive(askVault(str(m, "need") ?: ""))
            "scan" -> throw NodeError("Scanning documents needs the iPhone or iPad app. Choose the files instead.")
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
            else -> throw NodeError("Unknown message")
        }
    }

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

    /** `one.vault(need)`: asks for what the app declares, in the Vault's sheet → whether it's allowed. */
    private suspend fun askVault(need: String): Boolean {
        if (settings != null) throw NodeError("Settings has no app to ask for.")
        val scope = screen.scope ?: "user"
        val out = store.post("/v1/vault/request", obj("scope" to scope, "app" to screen.app, "need" to need)) as? JsonObject
        val id = (out?.get("id") as? JsonPrimitive)?.contentOrNull ?: throw NodeError("The Vault didn’t take the request")
        return ask(scope, id)
    }

    /** A request for a Vault item, in the Vault's sheet → whether it's allowed once it's gone, or once the request is
     *  answered somewhere else (another device, an agent's CLI), which closes the sheet. */
    private suspend fun ask(scope: String, id: String): Boolean {
        val sheet = VaultSheet(scope, id)
        store.sheet = sheet
        var waited = 0
        while (store.sheet == sheet) {
            delay(300)
            if (++waited % 7 == 0 && store.sheet == sheet) {
                val state = runCatching { store.vaultRequests(scope, id).firstOrNull()?.state }.getOrNull()
                if (state != null && state != "waiting" && store.sheet == sheet) store.sheet = null
            }
        }
        return store.vaultRequests(scope, id).firstOrNull()?.state == "set"
    }
}
