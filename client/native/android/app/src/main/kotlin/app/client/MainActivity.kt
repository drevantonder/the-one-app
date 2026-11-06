package app.client

import android.content.Context
import android.content.res.Configuration
import android.graphics.drawable.ColorDrawable
import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature

/** The window: Tauri's web view (the web client's page), drawn edge to edge under the system bars, which follow light and
 *  dark; Back goes to the page's Back, or leaves the app at its home. */
class MainActivity : TauriActivity() {
    private var page: WebView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        // A test's settings (`<CLI>_ADDRESS`, …) come as extras named `env.<name>` of the intent that starts it, and are kept
        // for the starts after it (ClientApp): a debug build's alone.
        if (BuildConfig.DEBUG) ClientApp.settings(intent?.extras)
        enableEdgeToEdge()
        // As the person last chose, from the first frame.
        appearance(getSharedPreferences("app", Context.MODE_PRIVATE).getString("appearance", "automatic"), recreate = false)
        super.onCreate(savedInstanceState)
        // Edge to edge, but its content kept clear of the system bars and the keyboard, which the window behind it shows.
        ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content)) { view, insets ->
            val bars: Insets = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
            val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime())
            view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, keyboard.bottom))
            WindowInsetsCompat.CONSUMED
        }
        bars()
    }

    /** Back goes to the page's Back, or leaves the app. Tauri's own (AppPlugin's, which goes back in the web view's history)
     *  is added as the app starts, and the latest added is asked first: so this is added again whenever the window is
     *  shown, ahead of it. */
    private var back: OnBackPressedCallback? = null

    private fun backFirst() {
        back?.remove()
        back = object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                val web = page
                if (web == null) {
                    moveTaskToBack(true)
                    return
                }
                web.evaluateJavascript("window.__oneNative ? window.__oneNative.hardwareBack() : false") { handled ->
                    if (handled != "true") moveTaskToBack(true)
                }
            }
        }.also { onBackPressedDispatcher.addCallback(this, it) }
    }

    override fun onResume() {
        super.onResume()
        backFirst()
    }

    override fun onWebViewCreate(webView: WebView) {
        page = webView
        backFirst()
        // The token the app's commands need goes to the page's own frame alone: a screen's sandboxed frame, whose origin is
        // none, is not answered (src/platform/android.rs `ours`).
        if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            WebViewCompat.addWebMessageListener(webView, "oneBridge", setOf("https://tauri.localhost")) { _, _, _, isMainFrame, reply ->
                if (isMainFrame) reply.postMessage(Host.token)
            }
        }
        // The page's text is sized by the page (Settings' text size reaches it as a zoom, `scale`).
        webView.settings.textZoom = 100
        // Its colour until the page draws, as the window's, so a start in dark has no white frame.
        webView.setBackgroundColor(colour())
    }

    /** Light, dark or automatic ("automatic", "light", "dark"), as the person chose. */
    fun appearance(value: String?, recreate: Boolean = true) {
        AppCompatDelegate.setDefaultNightMode(when (value) {
            "light" -> AppCompatDelegate.MODE_NIGHT_NO
            "dark" -> AppCompatDelegate.MODE_NIGHT_YES
            else -> AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM
        })
        if (recreate) bars()
    }

    /** Whether the page draws dark: as the person chose, or the phone's own for automatic. Read from what's kept, not this
     *  activity's configuration, which isn't dark yet as a start in dark begins. */
    private fun dark(): Boolean = when (getSharedPreferences("app", Context.MODE_PRIVATE).getString("appearance", "automatic")) {
        "dark" -> true
        "light" -> false
        else -> (applicationContext.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
    }

    /** The page's colour (@one/ui's --background), for what's drawn before it is. */
    private fun colour(): Int = if (dark()) 0xFF0A0A0A.toInt() else 0xFFFFFFFF.toInt()

    /** The system bars' icons dark on light and light on dark, over the window's colour, which is the page's. */
    private fun bars() {
        val dark = dark()
        window.setBackgroundDrawable(ColorDrawable(colour()))
        page?.setBackgroundColor(colour())
        WindowCompat.getInsetsController(window, window.decorView).apply {
            isAppearanceLightStatusBars = !dark
            isAppearanceLightNavigationBars = !dark
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        bars()
        // Settings' text size changed while the app runs: the page zooms screens by it.
        page?.evaluateJavascript("window.__oneNative && window.__oneNative.scale(${newConfig.fontScale})", null)
    }
}
