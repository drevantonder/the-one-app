package app.client

import android.app.Application
import android.content.Context
import android.os.Bundle
import android.system.Os

/** The app's process: what the system starts with no window (a widget's refresh, a notification) finds the app's folder and
 *  keys through this, in the same process the window runs in. */
class ClientApp : Application() {
    override fun onCreate() {
        super.onCreate()
        context = this
        if (BuildConfig.DEBUG) settings(null)
    }

    companion object {
        /** In a debug build, a test's settings: those the intent that starts the app gives (`env.<name>` extras) kept, and
         *  every one kept set in the process's environment, which the app's library reads: for a start by the system with
         *  no window too (a widget's refresh). */
        fun settings(extras: Bundle?) {
            val kept = context.getSharedPreferences("debug-env", Context.MODE_PRIVATE)
            extras?.keySet()?.filter { it.startsWith("env.") }?.forEach { kept.edit().putString(it, extras.getString(it) ?: "").apply() }
            kept.all.forEach { (key, value) -> Os.setenv(key.removePrefix("env."), value as? String ?: "", true) }
        }

        lateinit var context: Context
            private set
    }
}
