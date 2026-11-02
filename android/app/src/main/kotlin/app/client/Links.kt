package app.client

import android.app.Activity
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

// Links to the install's address, which Android opens here (App Links: the build's manifest names the addresses, and
// each lists this app). They go where the web client would take them (kernel/web/web.js's route()): an invite to the
// welcome, which opens it in a Custom Tab to make a passkey; the rest, once signed in at that address, to the page,
// screen, Settings or Vault request they name. Anything else opens in a Custom Tab, as it would have without the app.
object Links {
    /** An invite this app was opened with, for the welcome to open. */
    var invite by mutableStateOf<String?>(null)

    fun open(activity: Activity, store: Store, uri: Uri) {
        val at = "${uri.scheme}://${uri.authority}"
        val path = uri.pathSegments
        if (uri.path == "/invite" && !uri.fragment.isNullOrEmpty() && store.phase == Store.Phase.SignedOut) {
            invite = uri.toString()
            return
        }
        if (store.phase == Store.Phase.SignedOut || at != store.signIn.address || path.isEmpty()) return SignIn.openInTab(activity, uri)
        val known = when (path[0]) {
            "vault" -> query(uri, "scope") != null && query(uri, "id") != null
            "page", "services", "app" -> path.size > 1
            "settings" -> true
            else -> false
        }
        if (!known) return SignIn.openInTab(activity, uri)
        // Once the pages are there, so the page it names is one of them.
        MainScope().launch {
            snapshotFlow { store.phase }.first { it == Store.Phase.Ready || it == Store.Phase.SignedOut }
            if (store.phase == Store.Phase.Ready) go(store, uri)
        }
    }

    private fun query(uri: Uri, name: String) = uri.getQueryParameter(name)?.ifEmpty { null }

    private suspend fun go(store: Store, uri: Uri) {
        val path = uri.pathSegments
        val scope = query(uri, "scope")
        val params = query(uri, "params") ?: "{}"
        when (path[0]) {
            "vault" -> {
                store.path.clear()
                store.present(Route.Own("request", obj("scope" to scope, "id" to query(uri, "id")).toString()))
            }
            "page" -> store.openPage(path[1])
            "services" -> {
                store.openPage(path[1])
                store.present(Route.Own("services", obj("scope" to path[1]).toString()))
            }
            "app" -> {
                if (scope != null && store.pages.any { it.scope == scope }) store.openPage(scope) else store.path.clear()
                store.push(Route.App(path[1], path.getOrNull(2) ?: "main", params, scope))
            }
            "settings" -> {
                store.openPage("settings")
                path.getOrNull(1)?.let { store.push(Route.Own(it, params)) }
            }
        }
    }
}
