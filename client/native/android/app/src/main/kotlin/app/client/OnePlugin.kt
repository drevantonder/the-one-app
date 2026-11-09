package app.client

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.view.HapticFeedbackConstants
import android.webkit.MimeTypeMap
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.browser.customtabs.CustomTabsClient
import androidx.browser.customtabs.CustomTabsIntent
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

/** What the window's activity does for the app (client/native/src/platform/android.rs): signing in in a Chrome Custom Tab, which comes
 *  back to the app's own link; links, files and the document scanner; haptics; notifications' permission and this
 *  phone's FCM token; and the system bars, which follow light or dark. */
@TauriPlugin
class OnePlugin(private val activity: Activity) : Plugin(activity) {
    /** A sign-in waiting to come back, and the link it comes back to. */
    private var signing: Invoke? = null
    private var scheme = ""

    @Command
    fun signIn(invoke: Invoke) {
        val args = invoke.getArgs()
        signing?.reject("Sign in again.")
        signing = invoke
        scheme = args.getString("scheme")
        if (args.optBoolean("open", true)) openInTab(activity, Uri.parse(args.getString("url")))
        // Waiting for the user to sign in takes a while, but not forever.
        val waiting = invoke
        android.os.Handler(activity.mainLooper).postDelayed({
            if (signing === waiting) {
                signing = null
                waiting.reject("Signing in took too long. Try again.")
            }
        }, 300_000)
    }

    /** The link the sign-in came back to (`<cli>://signed-in/?code=…`): it ends the wait. */
    override fun onNewIntent(intent: Intent) {
        val link = intent.data ?: return
        if (link.scheme != scheme || link.host != "signed-in") return
        signing?.resolve(JSObject().put("url", link.toString()))
        signing = null
    }

    @Command
    fun open(invoke: Invoke) {
        val args = invoke.getArgs()
        val url = Uri.parse(args.getString("url"))
        // The network's own pages in a Custom Tab, as the browser is signed in there; anything else where it opens.
        if (args.optBoolean("own") && url.scheme?.startsWith("http") == true) openInTab(activity, url)
        else activity.startActivity(Intent(Intent.ACTION_VIEW, url).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        invoke.resolve()
    }

    /** A file a screen gives (`path`, in this app's folder): shared, or with `preview`, shown by whatever views its type. */
    @Command
    fun give(invoke: Invoke) {
        val args = invoke.getArgs()
        val file = File(args.getString("path"))
        val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream"
        val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.files", file)
        val intent = if (args.optBoolean("preview")) {
            Intent(Intent.ACTION_VIEW).setDataAndType(uri, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        } else {
            Intent.createChooser(Intent(Intent.ACTION_SEND).setType(type).putExtra(Intent.EXTRA_STREAM, uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION), file.name)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        try {
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("No file to show.")
        }
    }

    @Command
    fun scan(invoke: Invoke) {
        CoroutineScope(Dispatchers.Main).launch {
            try {
                invoke.resolve(JSObject().put("files", Scanner.scan(activity as ComponentActivity)))
            } catch (e: Exception) {
                invoke.reject(e.message ?: "Couldn’t open the scanner. Choose the files instead.")
            }
        }
    }

    /** "selection" (a press), "medium" (a long press), "success" or "error". */
    @Command
    fun haptic(invoke: Invoke) {
        val feedback = when (invoke.getArgs().optString("style")) {
            "medium" -> HapticFeedbackConstants.LONG_PRESS
            "success" -> if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.CONFIRM else HapticFeedbackConstants.CONTEXT_CLICK
            "error" -> if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.REJECT else HapticFeedbackConstants.LONG_PRESS
            else -> HapticFeedbackConstants.CLOCK_TICK
        }
        activity.runOnUiThread { activity.window.decorView.performHapticFeedback(feedback) }
        invoke.resolve()
    }

    /** Whether this phone shows notifications: "granted", "denied" or "default" (the user hasn't been asked); none in a
     *  build with no Firebase project, which gets no pushes. Asks if `ask`; with `once`, only if it never has. */
    @Command
    fun notify(invoke: Invoke) {
        val args = invoke.getArgs()
        val answer = { state: String? -> invoke.resolve(JSObject().put("state", state ?: JSONObject.NULL)) }
        if (!Push.available(activity)) return answer(null)
        val prefs = activity.getSharedPreferences("push", Context.MODE_PRIVATE)
        val granted = Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        if (granted) return answer("granted")
        val asked = prefs.getBoolean("asked", false)
        if (!args.optBoolean("ask") || (args.optBoolean("once") && asked)) return answer(if (asked) "denied" else "default")
        prefs.edit().putBoolean("asked", true).apply()
        val key = "notify-${System.nanoTime()}"
        val launcher = (activity as ComponentActivity).activityResultRegistry.register(key, ActivityResultContracts.RequestPermission()) { ok ->
            answer(if (ok) "granted" else "denied")
        }
        launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
    }

    /** This phone's FCM token, for the push Worker (none without the custom build's Firebase). */
    @Command
    fun push(invoke: Invoke) {
        Thread {
            val token = Push.token(activity)
            invoke.resolve(JSObject().put("token", token ?: JSONObject.NULL))
        }.start()
    }

    /** Light, dark or automatic, as the user chose in Settings: the system bars' icons follow it, and the app starts in it. */
    @Command
    fun appearance(invoke: Invoke) {
        val value = invoke.getArgs().optString("value")
        activity.getSharedPreferences("app", Context.MODE_PRIVATE).edit().putString("appearance", value).apply()
        activity.runOnUiThread { (activity as? MainActivity)?.appearance(value) }
        invoke.resolve()
    }

    companion object {
        /** A Custom Tab, in Chrome where it can, else whatever the phone has. */
        fun openInTab(context: Context, url: Uri) {
            val tab = CustomTabsIntent.Builder().setShowTitle(true).setShareState(CustomTabsIntent.SHARE_STATE_OFF).build()
            CustomTabsClient.getPackageName(context, listOf("com.android.chrome"), true)?.let { tab.intent.setPackage(it) }
            tab.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            tab.launchUrl(context, url)
        }
    }
}
