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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// Notifications. The install's push Worker (cloudflare/push/) carries each note, sealed to this phone's device, through
// Firebase Cloud Messaging as a data message alone (`n`, base64url), which wakes this app; the app opens it with the
// device's key (client/ffi's openNote), asking nothing of the network, and shows it. Tapping one opens the app screen
// it names. A device asking its person to join shows Approve and Not Me, answered from here (PushActions) as Settings ›
// Devices answers them. Firebase is the custom build's own project (config.toml's `firebase`), which a build without
// one leaves out: then this phone registers nothing and gets no notifications.

object Push {
    private const val CHANNEL = "notes"
    private val prefs = { context: Context -> context.getSharedPreferences("push", Context.MODE_PRIVATE) }

    /** This phone's FCM token, registered with the push Worker as its device's: as the app starts let in, and whenever
     *  FCM gives it a new one. Nothing without the custom build's Firebase. */
    suspend fun register(context: Context, client: Client) {
        if (FirebaseApp.getApps(context).isEmpty()) return
        val token = withContext(Dispatchers.IO) { Tasks.await(FirebaseMessaging.getInstance().token) }
        client.registerPush("fcm", token)
    }

    /** A push came: its note opened with whichever of this phone's device keys it was sealed to, and shown. One that
     *  opens with none (sealed to a device signed out of here) shows nothing. */
    fun received(context: Context, n: String) {
        val note = SignIn(context).deviceSecrets().firstNotNullOfOrNull { secret ->
            runCatching { uniffi.one_client_ffi.openNote(secret, n) }.getOrNull()
        } ?: return
        show(context, note)
    }

    /** The code of the device a note's action asks to let in (`{ "join": "<code>" }`), if it does. */
    private fun join(note: uniffi.one_client_ffi.Note): String? = note.action?.let { a ->
        runCatching { (json.parseToJsonElement(a) as? kotlinx.serialization.json.JsonObject)?.get("join")?.let { (it as? kotlinx.serialization.json.JsonPrimitive)?.content } }.getOrNull()
    }?.ifEmpty { null }

    fun show(context: Context, note: uniffi.one_client_ffi.Note, problem: String? = null) {
        val join = join(note)
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED &&
            android.os.Build.VERSION.SDK_INT >= 33) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "Notifications", NotificationManager.IMPORTANCE_HIGH))
        }
        // A join's taps open Settings › Devices, where it waits too; anything else the app screen it names, or the app.
        val link = when {
            join != null -> "${BuildConfig.CLI}://settings/devices"
            // Settings' own screens (Settings › Devices, where a device that joined is) are this app's, not an app's.
            note.app == "settings" -> "${BuildConfig.CLI}://settings/${enc(note.screen)}"
            note.app.isNotEmpty() -> "${BuildConfig.CLI}://app/${enc(note.app)}/${enc(note.screen.ifEmpty { "main" })}" +
                if (note.params == "{}") "" else "?params=${enc(note.params)}"
            else -> null
        }
        val open = Intent(context, MainActivity::class.java).apply {
            action = Intent.ACTION_VIEW
            link?.let { data = Uri.parse(it) }
            flags = Intent.FLAG_ACTIVITY_NEW_TASK
        }
        val built = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.notification)
            .setContentTitle(note.title)
            .setContentText(problem ?: note.body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(problem ?: note.body))
            .setAutoCancel(true)
            .setContentIntent(PendingIntent.getActivity(context, note.id.hashCode(), open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
        join?.let { code ->
            built.setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
            built.addAction(0, "Not Me", PushActions.intent(context, note, code, approve = false))
            built.addAction(0, "Approve", PushActions.intent(context, note, code, approve = true))
        }
        runCatching { NotificationManagerCompat.from(context).notify(note.id, 0, built.build()) }
    }

    fun dismiss(context: Context, id: String) = NotificationManagerCompat.from(context).cancel(id, 0)

    /** Once, as the app is first let in: whether this phone may show notifications (Android 13 on asks). */
    fun shouldAsk(context: Context): Boolean {
        if (android.os.Build.VERSION.SDK_INT < 33 || FirebaseApp.getApps(context).isEmpty()) return false
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return false
        val p = prefs(context)
        if (p.getBoolean("asked", false)) return false
        p.edit().putBoolean("asked", true).apply()
        return true
    }
}

/** Where FCM hands this app its pushes, and its new tokens. */
class PushService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        message.data["n"]?.let { Push.received(this, it) }
    }

    override fun onNewToken(token: String) {
        val store = (application as ClientApp).store
        if (store.signIn.signedIn) MainScope().launch { runCatching { store.client.registerPush("fcm", token) } }
    }
}

/** A join's Approve or Not Me, tapped: answered as this device, from here, with nothing opened. If it can't be, the
 *  notification says why, with the same two answers. */
class PushActions : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val code = intent.getStringExtra("code") ?: return
        val approve = intent.action == APPROVE
        val note = uniffi.one_client_ffi.Note(intent.getStringExtra("id") ?: "", intent.getStringExtra("title") ?: "", intent.getStringExtra("body") ?: "",
            "", "", "{}", null, obj("join" to code).toString(), null)
        val done = goAsync()
        Push.dismiss(context, note.id)
        MainScope().launch {
            try {
                val store = (context.applicationContext as ClientApp).store
                if (approve) store.post("/v1/devices/approve", obj("code" to code, "confirm" to true))
                else store.post("/v1/devices/refuse", obj("code" to code))
            } catch (e: Exception) {
                Push.show(context, note, problem = (if (approve) "Couldn’t approve it: " else "Couldn’t turn it down: ") + (e.message ?: "try again."))
            } finally {
                done.finish()
            }
        }
    }

    companion object {
        private const val APPROVE = "app.client.push.APPROVE"
        private const val REFUSE = "app.client.push.REFUSE"

        fun intent(context: Context, note: uniffi.one_client_ffi.Note, code: String, approve: Boolean): PendingIntent {
            val i = Intent(context, PushActions::class.java).apply {
                action = if (approve) APPROVE else REFUSE
                putExtra("code", code)
                putExtra("id", note.id)
                putExtra("title", note.title)
                putExtra("body", note.body)
            }
            return PendingIntent.getBroadcast(context, (note.id + approve).hashCode(), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
    }
}
