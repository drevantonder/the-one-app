package app.theone.client

import android.content.res.Configuration
import android.os.Bundle
import android.webkit.WebView
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature

/** The app's window: the web view, kept clear of the system's bars and of the keyboard, with the window showing behind them. */
class MainActivity : TauriActivity() {
  /** Whether back has anything to go back from: an app over the launcher, or something open over the app that's showing, as the page says (`androidBack` in client.js). */
  private var behind = false
  private var gate: Gate? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content)) { view, insets ->
      val kept = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      val clear = insets.getInsets(kept)
      view.setPadding(clear.left, clear.top, clear.right, clear.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }

  /**
   * The system's bars follow light and dark as the page does. The window isn't made again when that changes, so
   * what shows behind the bars, and whether their icons are dark or light, is set again.
   */
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    window.setBackgroundDrawableResource(R.color.background)
    enableEdgeToEdge()
  }

  /**
   * Back closes what's open over an app's page before it goes anywhere: a sheet, an alert, a menu. The page knows
   * whether anything is (`back` in home.js). If nothing was, back is what it was before this was asked: back
   * through the web view's history, and from its start out of the app.
   */
  override fun onWebViewCreate(webView: WebView) {
    if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
      WebViewCompat.addWebMessageListener(webView, "androidBack", setOf("http://tauri.localhost")) { _, message, _, _, _ ->
        behind = message.data == "true"
        gate?.pass()
      }
    }
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
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
   * The window's back, which reaches the app's back callbacks only while back has something to go back from.
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
      if (passed == behind) return
      passed = behind
      for ((callback, priority) in asked) {
        if (passed) window.registerOnBackInvokedCallback(priority, callback) else window.unregisterOnBackInvokedCallback(callback)
      }
    }
  }
}
