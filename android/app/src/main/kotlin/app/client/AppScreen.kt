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

// An app's screen: the page its node serves, in a web view of its own. Screens are untrusted: the page gets an origin
// of its own that's nowhere (ORIGIN), never this app's or the install's; it reaches nothing but the bridge, a message
// listener only that origin's main frame is given, answered here as the iPhone app's AppScreen.swift answers it, each
// call made as the screen's own app; it never navigates, and its links open in the browser.

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
fun AppScreen(store: Store, route: Route.Screen, setTitle: (String?) -> Unit) {
    val context = LocalContext.current
    val palette = LocalPalette.current
    var loaded by remember(route) { mutableStateOf(false) }
    var failure by remember(route) { mutableStateOf<String?>(null) }
    val accent = store.config?.accent ?: BuildConfig.ACCENT
    val bridge = remember(route) { Bridge(context, store, route, setTitle) }
    DisposableEffect(bridge) { onDispose { bridge.close() } }

    Box(Modifier.fillMaxSize()) {
        AndroidView(
            modifier = Modifier.fillMaxSize().alpha(if (loaded) 1f else 0f),
            factory = { ctx ->
                bridge.make(ctx, shim(accent, palette.dark)) { loaded = true }.also { web ->
                    bridge.scope.launch {
                        try {
                            web.loadDataWithBaseURL("$ORIGIN/", store.screen(route), "text/html", "utf-8", null)
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
    }
}

/** A screen's web view, and the bridge it answers. */
private class Bridge(val context: Context, val store: Store, val route: Route.Screen, val setTitle: (String?) -> Unit) {
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
                if (url.scheme in listOf("http", "https", "mailto", "tel")) runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, url).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
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

    /** The bridge: `{ type: "call" | "open" | "title" | … }`. A page only ever calls its own app, but may open
     *  another's screens. */
    private suspend fun answer(m: JsonObject): JsonElement? {
        when (str(m, "type")) {
            "call" -> return store.call(route.app, str(m, "op") ?: "", m["input"] ?: JsonObject(emptyMap()))
            "open" -> {
                val app = str(m, "app")?.ifEmpty { null } ?: route.app
                store.path.add(Route.Screen(app, str(m, "screen") ?: "main", (m["params"] ?: JsonObject(emptyMap())).toString(), route.scope))
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
            "icon" -> {
                val look = TileLook(str(m, "symbol")?.ifEmpty { null }, str(m, "seed") ?: "", str(m, "color")?.ifEmpty { null }, str(m, "image")?.ifEmpty { null }, str(m, "kind"))
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

    /** `one.vault(need)`: asks for what the app declares, in the Vault's sheet → whether it's allowed once it's gone,
     *  or once the request is answered somewhere else (another device, an agent's CLI), which closes the sheet. */
    private suspend fun askVault(need: String): Boolean {
        val scope = route.scope ?: "user"
        val out = store.post("/v1/vault/request", obj("scope" to scope, "app" to route.app, "need" to need)) as? JsonObject
        val id = (out?.get("id") as? JsonPrimitive)?.contentOrNull ?: throw NodeError("The Vault didn’t take the request")
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
