package app.client

import android.content.Context
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import app.client.widget.HomeWidgets
import org.json.JSONObject
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** What the app's library asks of Android that needs no window (src/platform/android.rs's `host`): the Keystore this device's keys
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
        else -> throw IllegalArgumentException("Unknown")
    }

    /** What the consent page and Devices call this phone: its name in Settings, else its model. */
    fun deviceName(context: Context): String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.ifEmpty { null } ?: android.os.Build.MODEL

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }
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
