package app.client

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.google.android.gms.tasks.Tasks
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject

// Notifications. The network's push Worker (cloudflare/push/) carries each note, sealed to this phone's device, through
// Firebase Cloud Messaging as a data message alone (`n`, base64url), which wakes this app; the app's library opens it with
// the device's key (Native "open-note"), asking nothing of the network, and this shows it. Tapping one opens the app
// screen it names. A device asking its person to join shows Approve and Not Me, answered from here (PushActions) as
// Settings › Devices answers them. Firebase is the custom build's own project (config.toml's `firebase`), which a build
// without one leaves out: then this phone registers nothing and gets no notifications.

object Push {
    private const val CHANNEL = "notes"

    /** Whether this build has a Firebase project, so pushes. */
    fun available(context: Context) = FirebaseApp.getApps(context).isNotEmpty()

    /** This phone's FCM token, for the push Worker: blocks, so never on the main thread. */
    fun token(context: Context): String? = if (available(context)) runCatching { Tasks.await(FirebaseMessaging.getInstance().token) }.getOrNull() else null

    /** A push came: its note opened with this device's key, and shown. One that opens with none (sealed to a device signed
     *  out of here) shows nothing. Blocks, so never on the main thread. */
    fun received(context: Context, n: String) {
        val note = JSONObject(Native.call("open-note", JSONObject().put("n", n).toString()))
        if (!note.has("error")) show(context, note)
    }

    /** The code of the device a note's action asks to let in (`{ "join": "<code>" }`), if it does. */
    private fun join(note: JSONObject): String? = note.optJSONObject("action")?.optString("join")?.ifEmpty { null }

    private fun enc(text: String) = Uri.encode(text)

    fun show(context: Context, note: JSONObject, problem: String? = null) {
        val join = join(note)
        if (android.os.Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "Notifications", NotificationManager.IMPORTANCE_HIGH))
        }
        val (id, title, body) = Triple(note.optString("id"), note.optString("title"), note.optString("body"))
        val (app, screen) = note.optString("app") to note.optString("screen")
        val params = note.optJSONObject("params")
        // A join's taps open Settings › Devices, where it waits too; anything else the app screen it names, or the app.
        val link = when {
            join != null -> "${BuildConfig.CLI}://settings/devices"
            // Settings' own screens (Settings › Devices, where a device that joined is) are this app's, not an app's.
            app == "settings" -> "${BuildConfig.CLI}://settings/${enc(screen)}"
            app.isNotEmpty() -> "${BuildConfig.CLI}://open?app=${enc(app)}&screen=${enc(screen.ifEmpty { "main" })}" + (params?.let { "&params=${enc(it.toString())}" } ?: "")
            else -> null
        }
        val open = Intent(context, MainActivity::class.java).apply {
            action = Intent.ACTION_VIEW
            link?.let { data = Uri.parse(it) }
            flags = Intent.FLAG_ACTIVITY_NEW_TASK
        }
        val built = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.notification)
            .setContentTitle(title)
            .setContentText(problem ?: body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(problem ?: body))
            .setAutoCancel(true)
            .setContentIntent(PendingIntent.getActivity(context, id.hashCode(), open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
        join?.let { code ->
            built.setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
            built.addAction(0, "Not Me", PushActions.intent(context, note, code, approve = false))
            built.addAction(0, "Approve", PushActions.intent(context, note, code, approve = true))
        }
        runCatching { NotificationManagerCompat.from(context).notify(id, 0, built.build()) }
    }

    fun dismiss(context: Context, id: String) = NotificationManagerCompat.from(context).cancel(id, 0)
}

/** Where FCM hands this app its pushes, and its new tokens. */
class PushService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        message.data["n"]?.let { Push.received(this, it) }
    }

    override fun onNewToken(token: String) {
        Native.call("register-push", JSONObject().put("token", token).toString())
    }
}

/** A join's Approve or Not Me, tapped: answered as this device, from here, with nothing opened. If it can't be, the
 *  notification says why, with the same two answers. */
class PushActions : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val code = intent.getStringExtra("code") ?: return
        val approve = intent.action == APPROVE
        val note = JSONObject().put("id", intent.getStringExtra("id") ?: "").put("title", intent.getStringExtra("title") ?: "")
            .put("body", intent.getStringExtra("body") ?: "").put("action", JSONObject().put("join", code))
        val done = goAsync()
        Push.dismiss(context, note.getString("id"))
        Thread {
            try {
                val said = JSONObject(Native.call("join", JSONObject().put("code", code).put("approve", approve).toString()))
                if (said.has("error")) {
                    Push.show(context, note, problem = (if (approve) "Couldn’t approve it: " else "Couldn’t turn it down: ") + said.getString("error").ifEmpty { "try again." })
                }
            } finally {
                done.finish()
            }
        }.start()
    }

    companion object {
        private const val APPROVE = "app.client.push.APPROVE"
        private const val REFUSE = "app.client.push.REFUSE"

        fun intent(context: Context, note: JSONObject, code: String, approve: Boolean): PendingIntent {
            val i = Intent(context, PushActions::class.java).apply {
                action = if (approve) APPROVE else REFUSE
                putExtra("code", code)
                putExtra("id", note.optString("id"))
                putExtra("title", note.optString("title"))
                putExtra("body", note.optString("body"))
            }
            return PendingIntent.getBroadcast(context, (note.optString("id") + approve).hashCode(), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
    }
}
