package app.client

import android.content.Context
import android.net.Uri
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.browser.customtabs.CustomTabsClient
import androidx.browser.customtabs.CustomTabsIntent
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

// Signing in, as every client of the network's sign-in does (cloudflare/web/web.js is the same in a browser): this
// app registers itself as an OAuth client, sends the person to signing in and consent at sign-in in a Custom Tab
// (Chrome, where a passkey's PRF works), and comes back at <cli>://signed-in/ with a code that PKCE turns into tokens.
// The refresh token is kept sealed by a key in this device's keystore, which never leaves it.

/** The tokens a sign-in gave. */
@Serializable
data class Tokens(val access: String, val refresh: String? = null, val until: Long)

class SignIn(private val context: Context) {
    private val prefs = context.getSharedPreferences("sign-in", Context.MODE_PRIVATE)
    /** Where the refresh token is: kept and unused, being swapped for a new one (each works once, so one swap at a
     *  time, under [swapping]), or gone, signed out, until the person signs in again. */
    private enum class State { Idle, Refreshing, SignedOut }
    @Volatile private var state = State.Idle
    private val swapping = Mutex()

    /** Where the network is: the build's, if the custom build has one, or the one the person gave. */
    var address: String?
        get() = BuildConfig.ADDRESS.ifEmpty { null } ?: prefs.getString("address", null)
        set(value) = prefs.edit().putString("address", value).apply()

    val redirect = "${BuildConfig.CLI}://signed-in/"

    var tokens: Tokens?
        get() = prefs.getString("tokens", null)?.let { sealed -> runCatching { json.decodeFromString<Tokens>(Keystore.open(sealed)) }.getOrNull() }
        private set(value) = prefs.edit().apply { if (value == null) remove("tokens") else putString("tokens", Keystore.seal(json.encodeToString(Tokens.serializer(), value))) }.apply()

    val signedIn get() = tokens != null && address != null

    /** The secret of an invite link this person opened here, for their first device's request to be let in. */
    var invite: String?
        get() = prefs.getString("invite", null)
        set(value) = prefs.edit().putString("invite", value).apply()

    /** This phone's device key for `id` (an address and a person), made the first time and kept sealed by the
     *  keystore: it's what the person's other devices approve, and what reaches their nodes. */
    fun deviceSecret(id: String): ByteArray {
        val name = "device:" + b64u(MessageDigest.getInstance("SHA-256").digest(id.toByteArray()))
        kept(name)?.let { return it }
        // The key its sign-in was for, made before it knew whose.
        val secret = kept(SIGNING_IN_FOR) ?: uniffi.one_client_ffi.newDeviceSecret()
        prefs.edit().putString(name, Keystore.seal(Base64.encodeToString(secret, Base64.NO_WRAP))).remove(SIGNING_IN_FOR).commit()
        return secret
    }

    /** Every device key kept here, for a push's note, which opens with the one it was sealed to. */
    fun deviceSecrets(): List<ByteArray> = prefs.all.keys.filter { it.startsWith("device:") && it != SIGNING_IN_FOR }.mapNotNull(::kept)

    private fun kept(name: String): ByteArray? =
        prefs.getString(name, null)?.let { sealed -> runCatching { Base64.decode(Keystore.open(sealed), Base64.NO_WRAP) }.getOrNull()?.takeIf { it.size == 32 } }

    /** The device a sign-in here is for, which its sign-in names (its key ‖ seal key, hex): the key the first person
     *  it signs in as takes ([deviceSecret]), made before it knows whose. */
    private fun signingInFor(): String {
        val secret = kept(SIGNING_IN_FOR) ?: uniffi.one_client_ffi.newDeviceSecret().also {
            prefs.edit().putString(SIGNING_IN_FOR, Keystore.seal(Base64.encodeToString(it, Base64.NO_WRAP))).commit()
        }
        return uniffi.one_client_ffi.deviceJoin(secret)
    }

    /** `id`'s device key goes: a key the network removed can't be let in again, so the next sign-in makes a new one. */
    fun forgetDeviceSecret(id: String) {
        prefs.edit().remove("device:" + b64u(MessageDigest.getInstance("SHA-256").digest(id.toByteArray()))).commit()
    }

    @Serializable
    private data class Meta(
        val authorization_endpoint: String,
        val token_endpoint: String,
        val registration_endpoint: String,
        val revocation_endpoint: String? = null,
    )

    /** Sign-in's endpoints, as it says them. */
    private suspend fun meta(): Meta {
        val r = http("GET", "${address ?: throw NodeError("Enter the address first.")}/.well-known/oauth-authorization-server")
        if (r.status != 200) throw NodeError("Can’t reach that address. Check it, and that you’re online.")
        return json.decodeFromString(r.text)
    }

    /** This app, as a client of sign-in: registered the first time it signs in at an address. */
    private suspend fun client(meta: Meta): String {
        prefs.getString("client", null)?.takeIf { prefs.getString("client.at", null) == address }?.let { return it }
        val body = JsonObject(mapOf(
            "client_name" to JsonPrimitive("${BuildConfig.NAME} on ${deviceName(context)}"),
            "redirect_uris" to kotlinx.serialization.json.JsonArray(listOf(JsonPrimitive(redirect))),
            "token_endpoint_auth_method" to JsonPrimitive("none"),
            "grant_types" to kotlinx.serialization.json.JsonArray(listOf(JsonPrimitive("authorization_code"), JsonPrimitive("refresh_token"))),
            "response_types" to kotlinx.serialization.json.JsonArray(listOf(JsonPrimitive("code"))),
            "application_type" to JsonPrimitive("native"),
        ))
        val r = http("POST", meta.registration_endpoint, body.toString().toByteArray(), "application/json")
        val out = runCatching { json.parseToJsonElement(r.text).jsonObject }.getOrNull()
        val id = out?.get("client_id")?.jsonPrimitive?.content
        if (r.status !in 200..299 || id == null) {
            throw NodeError(out?.get("error_description")?.jsonPrimitive?.content ?: out?.get("message")?.jsonPrimitive?.content ?: "This phone couldn’t sign up to sign in.")
        }
        prefs.edit().putString("client", id).putString("client.at", address).apply()
        return id
    }

    /** Off to signing in and consent at sign-in, in a Custom Tab; it comes back to [finish]. */
    suspend fun start(activity: android.app.Activity) {
        val meta = meta()
        val client = client(meta)
        val verifier = random(32)
        val state = random(16)
        prefs.edit().putString("pkce.verifier", verifier).putString("pkce.state", state).apply()
        val challenge = b64u(MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray()))
        val url = Uri.parse(meta.authorization_endpoint).buildUpon()
            .appendQueryParameter("response_type", "code").appendQueryParameter("client_id", client)
            .appendQueryParameter("redirect_uri", redirect).appendQueryParameter("scope", "openid profile offline_access")
            .appendQueryParameter("code_challenge", challenge).appendQueryParameter("code_challenge_method", "S256")
            .appendQueryParameter("state", state).appendQueryParameter("join", signingInFor()).build()
        withContext(Dispatchers.Main) { openInTab(activity, url) }
    }

    /** Back from consent: the code becomes tokens. */
    suspend fun finish(back: Uri) {
        val verifier = prefs.getString("pkce.verifier", null)
        val state = prefs.getString("pkce.state", null)
        prefs.edit().remove("pkce.verifier").remove("pkce.state").apply()
        back.getQueryParameter("error")?.let {
            throw NodeError(if (it == "access_denied") "You didn’t let this phone in." else back.getQueryParameter("error_description") ?: it)
        }
        val code = back.getQueryParameter("code")
        if (verifier == null || code == null || back.getQueryParameter("state") != state) throw NodeError("That sign-in didn’t start here. Sign in again.")
        val meta = meta()
        tokens = grant(meta, mapOf("grant_type" to "authorization_code", "code" to code, "redirect_uri" to redirect,
            "client_id" to (prefs.getString("client", null) ?: ""), "code_verifier" to verifier))
        this.state = State.Idle
    }

    private suspend fun grant(meta: Meta, fields: Map<String, String>): Tokens {
        val r = http("POST", meta.token_endpoint, form(fields).toByteArray(), "application/x-www-form-urlencoded")
        val out = runCatching { json.parseToJsonElement(r.text).jsonObject }.getOrNull()
        val access = out?.get("access_token")?.jsonPrimitive?.content
        if (r.status != 200 || access == null) {
            val why = out?.get("error_description")?.jsonPrimitive?.content ?: out?.get("error")?.jsonPrimitive?.content ?: "Signing in didn’t work."
            throw NodeError(why, signedOut = r.status == 400 || r.status == 401)
        }
        val seconds = out["expires_in"]?.jsonPrimitive?.content?.toLongOrNull() ?: 3600
        return Tokens(access, out["refresh_token"]?.jsonPrimitive?.content, System.currentTimeMillis() + seconds * 1000)
    }

    /** An access token good for a minute more, new from the refresh token if it's older (or is `stale`, which the
     *  node refused): one at a time, since each refresh token works once. */
    suspend fun token(stale: String? = null): String {
        fun fresh(t: Tokens?) = t != null && t.access != stale && t.until > System.currentTimeMillis() + 60_000
        tokens.takeIf(::fresh)?.let { return it.access }
        return swapping.withLock {
            if (state == State.SignedOut) throw NodeError("Sign in again.", signedOut = true)
            val kept = tokens
            if (fresh(kept)) return@withLock kept!!.access
            val refresh = kept?.refresh ?: throw NodeError("Sign in again.", signedOut = true)
            state = State.Refreshing
            try {
                val meta = meta()
                val next = refreshed(meta, mapOf("grant_type" to "refresh_token", "refresh_token" to refresh, "client_id" to (prefs.getString("client", null) ?: "")))
                tokens = next.copy(refresh = next.refresh ?: refresh)
                next.access
            } catch (e: NodeError) {
                if (e.signedOut) {
                    tokens = null
                    state = State.SignedOut
                }
                throw e
            } finally {
                if (state == State.Refreshing) state = State.Idle
            }
        }
    }

    /** A refresh token's grant. An answer lost on the way back leaves the token it sent, which sign-in still
     *  takes for a moment: so it's sent again at once, and only a refusal signs this phone out. */
    private suspend fun refreshed(meta: Meta, fields: Map<String, String>): Tokens {
        for (attempt in 0 until 3) {
            try {
                return grant(meta, fields)
            } catch (_: java.io.IOException) {
                delay(500L shl attempt)
            }
        }
        return grant(meta, fields)
    }

    /** Signs out: the refresh token is revoked, so it opens nothing again, and forgotten here. A refresh on its way
     *  finishes first, so what it kept is what's revoked and forgotten, not kept again after. */
    suspend fun signOut() {
        val kept = swapping.withLock {
            state = State.SignedOut
            tokens.also { tokens = null }
        }
        runCatching {
            val meta = meta()
            val endpoint = meta.revocation_endpoint
            if (kept?.refresh != null && endpoint != null) {
                http("POST", endpoint, form(mapOf("token" to kept.refresh, "token_type_hint" to "refresh_token", "client_id" to (prefs.getString("client", null) ?: ""))).toByteArray(), "application/x-www-form-urlencoded")
            }
        }
    }

    companion object {
        /** Where the key a sign-in here is for is kept until it knows whose ([signingInFor]). */
        private const val SIGNING_IN_FOR = "device:"

        /** Opens a page of the network's in a Custom Tab: Chrome's if it's here, since that's where passkeys work. */
        fun openInTab(context: Context, url: Uri) {
            val tab = CustomTabsIntent.Builder().setShowTitle(true).setShareState(CustomTabsIntent.SHARE_STATE_OFF).build()
            CustomTabsClient.getPackageName(context, listOf("com.android.chrome"), true)?.let { tab.intent.setPackage(it) }
            tab.launchUrl(context, url)
        }

        /** Who an access token is for: its `sub`. */
        fun subject(token: String): String? = runCatching {
            val payload = String(Base64.decode(token.split(".")[1], Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP))
            json.parseToJsonElement(payload).jsonObject["sub"]?.jsonPrimitive?.content
        }.getOrNull()

        /** What the consent page and Devices call this phone: its name in Settings, else its model. */
        fun deviceName(context: Context): String =
            Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.ifEmpty { null } ?: android.os.Build.MODEL
    }
}

/** A response's status and body. */
class Reply(val status: Int, val bytes: ByteArray, val type: String?) {
    val text get() = String(bytes)
}

/** How long to wait before asking again the `misses`th time running (from 0) after the node didn't answer, in ms: at
 *  once, then a quarter second, doubling, up to five. A node restarting for an update is back within a moment. */
fun retryWait(misses: Int): Long = if (misses == 0) 0 else minOf(5000L, 125L shl minOf(misses, 6))

/** `send`, again for a few seconds while nothing answers or the gateway reaches no node, so nothing ran; never one
 *  that timed out, which may have. */
suspend fun retried(send: suspend () -> Reply): Reply {
    for (misses in 0..4) {
        try {
            val r = send()
            if (r.status !in setOf(502, 503, 530)) return r
        } catch (e: java.net.ConnectException) {
        } catch (e: java.net.UnknownHostException) {
        } catch (e: java.net.NoRouteToHostException) {
        }
        delay(retryWait(misses))
    }
    return send()
}

/** Names a call wherever it's sent, so sent again it runs once. */
fun callId(): String = ByteArray(8).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) }

/** One HTTP request, off the main thread. */
suspend fun http(method: String, url: String, body: ByteArray? = null, type: String? = null, headers: Map<String, String> = emptyMap(), timeout: Int = 30_000): Reply =
    withContext(Dispatchers.IO) {
        val c = URL(url).openConnection() as HttpURLConnection
        try {
            c.requestMethod = method
            c.connectTimeout = 15_000
            c.readTimeout = timeout
            c.instanceFollowRedirects = false
            headers.forEach { (k, v) -> c.setRequestProperty(k, v) }
            if (body != null) {
                c.doOutput = true
                c.setRequestProperty("content-type", type ?: "application/json")
                c.outputStream.use { it.write(body) }
            }
            val status = c.responseCode
            val stream = if (status >= 400) c.errorStream else c.inputStream
            Reply(status, stream?.use { it.readBytes() } ?: ByteArray(0), c.contentType)
        } finally {
            c.disconnect()
        }
    }

private fun form(fields: Map<String, String>) = fields.entries.joinToString("&") { (k, v) -> "${URLEncoder.encode(k, "UTF-8")}=${URLEncoder.encode(v, "UTF-8")}" }

private fun b64u(bytes: ByteArray) = Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)

private fun random(n: Int) = b64u(ByteArray(n).also { SecureRandom().nextBytes(it) })

/** Seals text with an AES key that lives in this device's keystore and can't be taken out of it. */
private object Keystore {
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
