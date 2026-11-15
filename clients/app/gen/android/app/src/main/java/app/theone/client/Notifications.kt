package app.theone.client

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import app.tauri.notification.NotificationPlugin
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject

/**
 * What a notification reaches this phone through: a data message from FCM holding the notification encrypted to this
 * phone (`n`), which Google carries without reading. It's opened here, with the key in the app's own folder, by the
 * app's Rust (`open`, `client::notification`), and shown as the system's own notification. It runs whether or not
 * the app does. It takes the place of the push plugin's own service, which shows only what arrives in the clear, and
 * hands that plugin each new token, which the app registers.
 */
class Notifications : FirebaseMessagingService() {
  override fun onNewToken(token: String) {
    NotificationPlugin.instance?.handleNewToken(token)
  }

  override fun onMessageReceived(message: RemoteMessage) {
    val sealed = message.data["n"] ?: return
    // The app's folder, as Tauri's `app_data_dir` is on Android, which is where the client keeps its key.
    val said = open(dataDir.absolutePath, sealed)?.let(::JSONObject) ?: return
    val manager = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      manager.createNotificationChannel(NotificationChannel(CHANNEL, "Notifications", NotificationManager.IMPORTANCE_HIGH))
    }
    // A tap opens the app, on the app the notification is from.
    val tap = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("app", said.optString("app"))
    val shown = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle(said.optString("title"))
      .setContentText(said.optString("text").ifEmpty { null })
      .setAutoCancel(true)
      .setContentIntent(PendingIntent.getActivity(this, message.sentTime.toInt(), tap, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
      .build()
    manager.notify(message.sentTime.toInt(), shown)
  }

  companion object {
    private const val CHANNEL = "notifications"

    init {
      System.loadLibrary("app")
    }

    /** The notification `sealed` (base64) opened with the key in `home`, as JSON: its title, text and app. Null if it doesn't open. */
    @JvmStatic external fun open(home: String, sealed: String): String?
  }
}
