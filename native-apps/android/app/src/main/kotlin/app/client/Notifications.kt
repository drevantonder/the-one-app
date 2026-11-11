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

// Notifications. The network's notifications Worker (notifications/worker/) carries each note, sealed to this phone's device, through
// Firebase Cloud Messaging as a data message alone (`n`, base64url), which wakes this app; the app's library opens it with
// the device's key (Native "open-note"), asking nothing of the network, and this shows it. Tapping one opens the app
// screen it names. A device asking its user to join shows Approve and Not Me, answered from here (NotificationActions) as
// Settings › Devices answers them. Firebase is the custom build's own project (config.toml's `firebase`), which a build
// without one leaves out: then this phone registers nothing and gets no notifications.

object Notifications {
    private const val CHANNEL = "notes"

    /** Whether this build has a Firebase project, so notifications. */
    fun available(context: Context) = FirebaseApp.getApps(context).isNotEmpty()

    /** This phone's FCM token, for the notifications Worker: blocks, so never on the main thread. */
    fun token(context: Context): String? = if (available(context)) runCatching { Tasks.await(FirebaseMessaging.getInstance().token) }.getOrNull() else null

    /** A notification came: its note opened with this device's key, and shown. One that opens with none (sealed to a device signed
     *  out of here) shows nothing. Blocks, so never on the main thread. */
    fun received(context: Context, n: String) {
        val note = JSONObject(Native.call("open-note", JSONObject().put("n", n).toString()))
        if (!note.has("error")) show(context, note)
    }

    /** The code of the device a note asks to let in, if it does. */
    private fun join(note: JSONObject): String? = note.optString("join").ifEmpty { null }

    fun show(context: Context, note: JSONObject, problem: String? = null) {
        val join = join(note)
        if (android.os.Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "Notifications", NotificationManager.IMPORTANCE_HIGH))
        }
        val (id, title, body) = Triple(note.optString("id"), note.optString("title"), note.optString("body"))
        val link = note.optString("link").ifEmpty { null }
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
            built.addAction(0, "Not Me", NotificationActions.intent(context, note, code, approve = false))
            built.addAction(0, "Approve", NotificationActions.intent(context, note, code, approve = true))
        }
        runCatching { NotificationManagerCompat.from(context).notify(id, 0, built.build()) }
    }

    fun dismiss(context: Context, id: String) = NotificationManagerCompat.from(context).cancel(id, 0)
}

/** Where FCM hands this app its notifications, and its new tokens. */
class NotificationsService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        message.data["n"]?.let { Notifications.received(this, it) }
    }

    override fun onNewToken(token: String) {
        Native.call("register-notifications", JSONObject().put("via", "fcm").put("address", token).toString())
    }
}

/** A join's Approve or Not Me, tapped: answered as this device, from here, with nothing opened. If it can't be, the
 *  notification says why, with the same two answers. */
class NotificationActions : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val code = intent.getStringExtra("code") ?: return
        val approve = intent.action == APPROVE
        val note = JSONObject().put("id", intent.getStringExtra("id") ?: "").put("title", intent.getStringExtra("title") ?: "")
            .put("body", intent.getStringExtra("body") ?: "").put("join", code)
        val done = goAsync()
        Notifications.dismiss(context, note.getString("id"))
        Thread {
            try {
                val said = JSONObject(Native.call("join", JSONObject().put("code", code).put("approve", approve).toString()))
                if (said.has("error")) {
                    Notifications.show(context, note, problem = (if (approve) "Couldn’t approve it: " else "Couldn’t turn it down: ") + said.getString("error").ifEmpty { "try again." })
                }
            } finally {
                done.finish()
            }
        }.start()
    }

    companion object {
        private const val APPROVE = "app.client.notifications.APPROVE"
        private const val REFUSE = "app.client.notifications.REFUSE"

        fun intent(context: Context, note: JSONObject, code: String, approve: Boolean): PendingIntent {
            val i = Intent(context, NotificationActions::class.java).apply {
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
