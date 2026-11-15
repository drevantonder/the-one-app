package app.theone.client

import android.content.res.Configuration
import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

/** The app's window: the web view, kept clear of the system's bars and of the keyboard, with the window showing behind them. */
class MainActivity : TauriActivity() {
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
}
