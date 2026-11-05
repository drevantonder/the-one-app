package app.client

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.one_client_ffi.Header
import uniffi.one_client_ffi.OneException
import uniffi.one_client_ffi.Response
import java.io.File
import java.security.MessageDigest

// This phone as a device of its person (client/ffi, the same library every native app uses): its own key, made here
// and kept sealed by a key in the phone's keystore (SignIn), one for each person at each address. It gets in when a
// device of theirs approves the code it shows; then every request reaches their install's nodes over iroh, as this
// device, with the keys each needs. Signing in only says who's asking.

class Client(private val context: Context, private val signIn: SignIn) {
    private var device: uniffi.one_client_ffi.ThisDevice? = null
    private var deviceFor: String? = null

    /** This phone's device for whoever's signed in, at their address. */
    private suspend fun device(): uniffi.one_client_ffi.ThisDevice {
        val token = signIn.token()
        val at = signIn.address ?: throw NodeError("Sign in again.", signedOut = true)
        val id = "$at ${SignIn.subject(token) ?: ""}"
        device?.takeIf { deviceFor == id }?.let { return it }
        val folder = File(context.filesDir, "client/" + MessageDigest.getInstance("SHA-256").digest(id.toByteArray()).joinToString("") { "%02x".format(it) }.take(16))
        val made = io { uniffi.one_client_ffi.ThisDevice(signIn.deviceSecret(id), at, folder.path) }
        made.on(onDevice())
        device?.let { old -> io { old.disconnect() } }
        device = made
        deviceFor = id
        return made
    }

    /** The phone this app is on, which signing in here again replaces its sign-in under: its Android ID for this app,
     *  which outlives reinstalling, hashed. */
    @android.annotation.SuppressLint("HardwareIds")
    private fun onDevice(): String {
        val id = android.provider.Settings.Secure.getString(context.contentResolver, android.provider.Settings.Secure.ANDROID_ID) ?: ""
        return MessageDigest.getInstance("SHA-256").digest("this device $id".toByteArray()).joinToString("") { "%02x".format(it) }
    }

    /** Whether this phone is let in yet: it reaches a node as itself. */
    suspend fun connect(): Boolean = device().let { d -> io { d.connect() } }

    /** Asks to be let in, as the person signed in (with their invite, if they're new) → the code it shows. */
    suspend fun ask(): String {
        val d = device()
        val token = signIn.token()
        io { d.ask(token, SignIn.deviceName(context), signIn.invite) }
        return d.code()
    }

    /** Whether it's been let in, asking again if its request waited too long. */
    suspend fun waited(): Boolean {
        val d = device()
        val token = signIn.token()
        return io { d.wait(token) }
    }

    /** Has pushes reach this phone at `address` (its FCM token), with the push service `via`. */
    suspend fun registerPush(via: String, address: String) = device().let { d -> io { d.registerPush(via, address, "", "") } }

    /** A request to the person's nodes: `/v1/…`, with a body (JSON) or none. Its body is read by whoever asked. */
    suspend fun fetch(method: String, path: String, body: ByteArray? = null, accept: String? = null): Response {
        val d = device()
        val headers = buildList {
            if (body != null) add(Header("content-type", "application/json"))
            if (accept != null) add(Header("accept", accept))
        }
        return try {
            io { d.fetch(method, path, headers, body ?: ByteArray(0)) }
        } catch (_: NotLetIn) {
            throw lost()
        }
    }

    /** Developer mode, on this phone alone (Settings › Developer): whether it's on, first turning it on or off. */
    suspend fun developer(on: Boolean?): Boolean {
        val d = device()
        return io {
            if (on != null) d.setDeveloper(on)
            d.developer()
        }
    }

    /** In developer mode, its latest requests' traces, as JSON, for a screen's `traces`; null out of it. */
    suspend fun traces(): String? = device().let { d -> io { d.traces() } }

    /** What it means that this phone isn't one of the install's: [NotLetIn], `removed` if it was and was taken out, which
     *  ends its sign-in (its key goes with it). */
    suspend fun lost(): NotLetIn {
        val d = device()
        val removed = io { d.removed() }
        if (removed) {
            deviceFor?.let { signIn.forgetDeviceSecret(it) }
            close()
        }
        return NotLetIn(removed)
    }

    /** What an app's call answered last for `input` (JSON), if its tool only reads (client/ffi's `kept`), from
     *  this phone alone; null without a device yet, as before anyone's signed in. */
    suspend fun kept(app: String, op: String, input: ByteArray): uniffi.one_client_ffi.KeptAnswer? {
        val d = device ?: runCatching { device() }.getOrNull() ?: return null
        return runCatching { io { d.kept(app, op, input) } }.getOrNull()
    }

    /** A fresh connection, once the network's back. */
    suspend fun reconnect() {
        device?.let { d -> io { d.reconnect() } }
    }

    /** Signs out: this phone takes itself out of its person's devices (it asks their node to), then its key goes, so signing
     *  in again makes a new device that replaces it, never a second. Asked before the sign-in is revoked. */
    suspend fun signOut() {
        device?.let { d -> io { d.signOut() } }
        deviceFor?.let { signIn.forgetDeviceSecret(it) }
        close()
    }

    /** Its connections close. Its key stays. */
    suspend fun close() {
        device?.let { d -> io { d.disconnect() } }
        device = null
        deviceFor = null
    }
}

/** Runs a call into the library off the main thread, its failure a `NodeError`. */
suspend fun <T> io(work: () -> T): T = withContext(Dispatchers.IO) {
    try {
        work()
    } catch (e: OneException.Failed) {
        throw NodeError(e.reason)
    } catch (_: OneException.NotLetIn) {
        throw NotLetIn(false)
    }
}

/** This phone isn't one of the install's: not yet (`removed` false) or no more. Not an app's problem: the app shows its
 *  own approval screen (`Store.Phase.Joining`), or, once removed, the sign-in. */
class NotLetIn(val removed: Boolean) : Exception("This phone hasn’t been let in yet.")

/** A response's body, whole. */
suspend fun Response.read(): ByteArray = io { bytes() }
