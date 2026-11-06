package app.client

import android.content.Context
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import app.client.widget.HomeWidgets
import org.json.JSONObject
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** What the app's library asks of Android that needs no window (src/android.rs's `host`): the Keystore this device's keys
 *  are sealed by, its folder, its name, and the widgets drawn again. `call` answers `{ "ok": … }` or `{ "error": … }`. */
object Host {
    private val context get() = ClientApp.context
    private val prefs get() = context.getSharedPreferences("sign-in", Context.MODE_PRIVATE)

    /** What only the page's own frame is given, for each of the app's commands to carry (Rust refuses one without it): a
     *  screen's sandboxed frame has Tauri's script and `ipc` too, but is not at the app's origin, so is not answered. */
    val token: String by lazy { ByteArray(24).also { java.security.SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) } }

    @JvmStatic
    fun call(method: String, json: String): String = try {
        JSONObject().put("ok", run(method, JSONObject(json)) ?: JSONObject.NULL).toString()
    } catch (e: Throwable) {
        JSONObject().put("error", e.message ?: e.toString()).toString()
    }

    private fun run(method: String, a: JSONObject): Any? = when (method) {
        "get" -> prefs.getString(a.getString("name"), null)?.let { runCatching { Keystore.open(it) }.getOrNull() }
        "set" -> {
            // Kept before anything else can ask: each refresh token works once.
            prefs.edit().putString(a.getString("name"), Keystore.seal(a.getString("value"))).commit()
            null
        }
        "delete" -> {
            prefs.edit().remove(a.getString("name")).commit()
            null
        }
        "token" -> token
        "files" -> context.filesDir.path
        "name" -> deviceName(context)
        "device" -> hex(MessageDigest.getInstance("SHA-256").digest("this device ${Settings.Secure.getString(context.contentResolver, Settings.Secure.ANDROID_ID)}".toByteArray()))
        "canScan" -> Scanner.available(context)
        "textScale" -> context.resources.configuration.fontScale.toDouble()
        "reload" -> {
            HomeWidgets.reload(context)
            null
        }
        "takeOver" -> {
            takeOver(a.getString("scheme"))
            null
        }
        else -> throw IllegalArgumentException("Unknown")
    }

    /** What the consent page and Devices call this phone: its name in Settings, else its model. */
    fun deviceName(context: Context): String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.ifEmpty { null } ?: android.os.Build.MODEL

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

    /** What the Kotlin app kept, taken over once, so no one signs in again: its address, an invite's secret, its sign-in's
     *  client, and who's signed in, as files in this app's folder; its refresh token as this app keeps it (`refresh`);
     *  its device keys stay where they were (`device:…`, the same names). The old widgets' files are gone: the app keeps
     *  them anew, and which widget each home-screen widget shows (`home-widgets`) is kept as it was. */
    private fun takeOver(scheme: String) {
        val files = context.filesDir
        val p = prefs
        val address = p.getString("address", null)
        if (address != null && !File(files, "address").exists()) {
            File(files, "address").writeText(address)
            p.getString("invite", null)?.let { File(files, "invite").writeText(it) }
            val client = p.getString("client", null)
            if (client != null && p.getString("client.at", null) == address) File(files, "sign-in").writeText("$address\n$client\n$scheme://signed-in/")
        }
        val sealed = p.getString("tokens", null)
        if (sealed != null) {
            val tokens = runCatching { JSONObject(Keystore.open(sealed)) }.getOrNull()
            val refresh = tokens?.optString("refresh").orEmpty()
            if (refresh.isNotEmpty() && p.getString("refresh", null) == null) {
                p.edit().putString("refresh", Keystore.seal(Base64.encodeToString(refresh.toByteArray(), Base64.NO_WRAP))).commit()
                val sub = runCatching { JSONObject(String(Base64.decode(tokens!!.getString("access").split(".")[1], Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP))).getString("sub") }.getOrNull()
                if (sub != null && !File(files, "user").exists()) File(files, "user").writeText(sub)
            }
            p.edit().remove("tokens").commit()
        }
        File(files, "home-widgets.json").delete()
        File(files, "home-widget-images").deleteRecursively()
    }
}

/** Seals text with an AES key that lives in this device's keystore and can't be taken out of it (the Kotlin app's key, the
 *  alias it made, so what it sealed still opens). */
object Keystore {
    private const val ALIAS = "sign-in"

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).build())
        return generator.generateKey()
    }

    fun seal(plain: String): String {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        return Base64.encodeToString(cipher.iv + cipher.doFinal(plain.toByteArray()), Base64.NO_WRAP)
    }

    fun open(sealed: String): String {
        val bytes = Base64.decode(sealed, Base64.NO_WRAP)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12)) }
        return String(cipher.doFinal(bytes, 12, bytes.size - 12))
    }
}
