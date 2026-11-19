package app.theone.client

import android.content.Intent
import android.content.res.Configuration
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.Rect
import android.graphics.drawable.ColorDrawable
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.util.Base64
import android.webkit.WebView
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import androidx.activity.BackEventCompat
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import androidx.core.net.toUri
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.children
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.theone.renderer.NativeHost
import org.json.JSONObject

/** The app's window: the web view, kept clear of the system's bars and of the keyboard, with the window showing behind them. */
class MainActivity : TauriActivity() {
  /** Whether back has anything to go back from: an app over the launcher, or something open over the app that's showing, as the page says (`androidBack` in client.js). */
  private var behind = false
  private var gate: Gate? = null
  /** Light or dark, as Colour Mode has it on this device, or "" as the system is (`setTheme` in host.js). */
  private var theme = ""

  override fun onCreate(savedInstanceState: Bundle?) {
    dress()
    super.onCreate(savedInstanceState)
    // The web view keeps clear of the bars and the keyboard; a native window draws under them, told where they are
    // (`NativeView`).
    ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content)) { view, insets ->
      val kept = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      val clear = insets.getInsets(kept)
      for (child in (view as android.view.ViewGroup).children) {
        if (child !is app.theone.renderer.NativeView) child.setPadding(clear.left, clear.top, clear.right, clear.bottom)
      }
      insets
    }
    // An app's window is a view over the web view, drawn natively, which Back closes (`NativeHost`).
    NativeHost.attach(this) { gate?.pass() }
  }

  /** Whether a key pressed now comes here, as a remote's does: Computer Use presses keys here as one (`ComputerUse.kt`). */
  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) focused = this else if (focused === this) focused = null
  }

  companion object {
    /** The app's window while keys come to it. */
    @Volatile var focused: MainActivity? = null
      private set
  }

  /** The window isn't made again when the system's light or dark changes, so its bars follow it here. */
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    dress()
    NativeHost.configurationChanged(this)
  }

  /**
   * The system's bars follow light and dark as the page does, Colour Mode's or the system's: what shows behind them,
   * the page's background (`colors.xml`), and whether their icons are dark or light.
   */
  private fun dress() {
    val night = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
    val dark = if (theme.isEmpty()) night else theme == "dark"
    window.setBackgroundDrawable(ColorDrawable(getColor(if (dark) R.color.dark else R.color.light)))
    val style = SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT) { dark }
    enableEdgeToEdge(style, style)
  }

  /**
   * Back closes what's open over an app's page before it goes anywhere: a sheet, an alert, a menu. The page knows
   * whether anything is (`back` in client.js). If nothing was, back is what it was before this was asked: back
   * through the web view's history, and from its start out of the app. As the back gesture is made (predictive back),
   * the page draws an app going home with it, where that's where back goes (`backStarted` in client.js).
   */
  override fun onWebViewCreate(webView: WebView) {
    // The network's sign-in page asks for a passkey in the web view, which Credential Manager gives for a site that
    // says this app is its own (`/.well-known/assetlinks.json`).
    if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_AUTHENTICATION)) {
      WebSettingsCompat.setWebAuthenticationSupport(webView.settings, WebSettingsCompat.WEB_AUTHENTICATION_SUPPORT_FOR_APP)
    }
    if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
      WebViewCompat.addWebMessageListener(webView, "androidTheme", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        theme = message.data ?: ""
        dress()
      }
      WebViewCompat.addWebMessageListener(webView, "androidBack", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        behind = message.data == "true"
        gate?.pass()
      }
      // Where the rail's line is on the window's right edge (`rail` in client.js), in the page's pixels, which
      // Android's back gesture leaves to it, so a finger there pulls the line; the rest of the edge is still back.
      // Android leaves an app at most 200 dp of each edge, which the line is shorter than.
      WebViewCompat.addWebMessageListener(webView, "androidEdge", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        val scale = resources.displayMetrics.density
        val at = message.data.orEmpty().split(",").mapNotNull { it.toFloatOrNull()?.times(scale)?.toInt() }
        ViewCompat.setSystemGestureExclusionRects(webView, if (at.size == 4) listOf(Rect(at[0], at[1], at[2], at[3])) else emptyList())
      }
      // Android's own Settings page an app asked to open (`one/client` in client.js): Accessibility, where its person
      // turns on Computer Use, or the app's App Info, where Android 13 and later has them allow restricted settings
      // first for an app that didn't come from a store.
      WebViewCompat.addWebMessageListener(webView, "androidSettings", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        val opened = when (message.data) {
          "accessibility" -> Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
          "app" -> Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", packageName, null))
          else -> return@addWebMessageListener
        }
        startActivity(opened)
      }
      // An app put on the device's own home screen, as its person asked from its menu on the Home Screen (`one/pin-shortcut`
      // in client.js): Android asks them where, and opens it here by its `one:` link.
      WebViewCompat.addWebMessageListener(webView, "androidPin", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        ShortcutManagerCompat.requestPinShortcut(this, shortcut(JSONObject(message.data ?: return@addWebMessageListener)), null)
      }
      // The apps used lately on this device, newest first, which a long press on the app's icon lists, as many as
      // Android shows; and every app installed, so a shortcut put on the home screen to one that's gone is greyed
      // out, and comes back if it's installed again (`shortcuts` in client.js).
      WebViewCompat.addWebMessageListener(webView, "androidShortcuts", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        val told = JSONObject(message.data ?: return@addWebMessageListener)
        val recent = told.getJSONArray("recent")
        val most = minOf(recent.length(), ShortcutManagerCompat.getMaxShortcutCountPerActivity(this))
        ShortcutManagerCompat.setDynamicShortcuts(this, (0 until most).map { shortcut(recent.getJSONObject(it), it) })
        val installed = told.getJSONArray("installed").let { all -> (0 until all.length()).map(all::getString).toSet() }
        val pinned = ShortcutManagerCompat.getShortcuts(this, ShortcutManagerCompat.FLAG_MATCH_PINNED).map { it.id }
        ShortcutManagerCompat.disableShortcuts(this, pinned.filter { it !in installed }, "This app isn't installed")
        ShortcutManagerCompat.enableShortcuts(this, ShortcutManagerCompat.getShortcuts(this, ShortcutManagerCompat.FLAG_MATCH_PINNED).filter { it.id in installed && !it.isEnabled })
      }
      // A haptic an app asked for, by iOS's name (`one/haptic` in client.js), played as Android's own, as a native
      // window's is (`NativeHost.haptic`).
      WebViewCompat.addWebMessageListener(webView, "androidHaptic", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        NativeHost.haptic(webView, message.data ?: return@addWebMessageListener)
      }
    }
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackStarted(backEvent: BackEventCompat) = webView.evaluateJavascript("backStarted()", null)

      override fun handleOnBackProgressed(backEvent: BackEventCompat) =
        webView.evaluateJavascript("backProgressed(${backEvent.progress})", null)

      override fun handleOnBackCancelled() = webView.evaluateJavascript("backCancelled()", null)

      override fun handleOnBackPressed() {
        webView.evaluateJavascript("back()") { closed ->
          if (closed == "true") return@evaluateJavascript
          isEnabled = false
          onBackPressedDispatcher.onBackPressed()
          isEnabled = true
        }
      }
    })
  }

  /**
   * An app's shortcut, as the page tells it: its entrypoint (`<app>/<page>`), name and icon, a PNG of its square, which
   * Android cuts to the home screen's shape as it does an app's own. It opens the app here by its `one:` link.
   */
  private fun shortcut(app: JSONObject, rank: Int = 0): ShortcutInfoCompat {
    val id = app.getString("id")
    val icon = Base64.decode(app.getString("icon"), Base64.DEFAULT)
    return ShortcutInfoCompat.Builder(this, id)
      .setShortLabel(app.getString("name"))
      .setIcon(IconCompat.createWithAdaptiveBitmap(BitmapFactory.decodeByteArray(icon, 0, icon.size)))
      .setIntent(Intent(Intent.ACTION_VIEW, "one:$id".toUri(), this, MainActivity::class.java))
      .setRank(rank)
      .build()
  }

  /**
   * The window's back, which reaches the app's back callbacks only while back has something to go back from: an app
   * over the launcher, something open over it, or an app's native window.
   * Otherwise the system has it, and shows going home as the gesture is made (predictive back). AndroidX asks the
   * window for back while any of its callbacks is on, and Tauri's app plugin keeps one on always (`AppPlugin`), so
   * without this the system never would.
   */
  override fun getOnBackInvokedDispatcher(): OnBackInvokedDispatcher =
    gate ?: Gate(super.getOnBackInvokedDispatcher()).also { gate = it }

  private inner class Gate(private val window: OnBackInvokedDispatcher) : OnBackInvokedDispatcher {
    private val asked = mutableMapOf<OnBackInvokedCallback, Int>()
    private var passed = false

    override fun registerOnBackInvokedCallback(priority: Int, callback: OnBackInvokedCallback) {
      asked[callback] = priority
      if (passed) window.registerOnBackInvokedCallback(priority, callback)
    }

    override fun unregisterOnBackInvokedCallback(callback: OnBackInvokedCallback) {
      if (asked.remove(callback) != null && passed) window.unregisterOnBackInvokedCallback(callback)
    }

    fun pass() {
      val wanted = behind || NativeHost.isOpen()
      if (passed == wanted) return
      passed = wanted
      for ((callback, priority) in asked) {
        if (passed) window.registerOnBackInvokedCallback(priority, callback) else window.unregisterOnBackInvokedCallback(callback)
      }
    }
  }
}
