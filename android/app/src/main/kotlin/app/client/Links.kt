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

// Links to the network's address, which Android opens here (App Links: the build's manifest names the addresses, and
// each lists this app). They go where the web client would take them (cloudflare/web/web.js's route()): an invite to the
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
        if (!known(uri, path)) return SignIn.openInTab(activity, uri)
        later(store, uri, path)
    }

    /** `<cli>://vault?id=…&scope=…`, `<cli>://page?scope=…` (with `folder=accounts`, its Accounts) and
     *  `<cli>://settings[/<item>]`, as the iPhone takes them: the same as the address's own link. */
    fun own(store: Store, uri: Uri) {
        val scope = query(uri, "scope")
        val path = when {
            uri.host == "page" && scope != null -> listOf(if (query(uri, "folder") == "accounts") "accounts" else "page", scope)
            else -> listOfNotNull(uri.host) + uri.pathSegments
        }
        if (store.phase != Store.Phase.SignedOut && path.isNotEmpty() && known(uri, path)) later(store, uri, path)
    }

    private fun known(uri: Uri, path: List<String>) = when (path[0]) {
        "vault" -> query(uri, "scope") != null && query(uri, "id") != null
        "page", "accounts", "app" -> path.size > 1
        "settings" -> true
        else -> false
    }

    /** Once the pages are there, so the page it names is one of them. */
    private fun later(store: Store, uri: Uri, path: List<String>) {
        MainScope().launch {
            snapshotFlow { store.phase }.first { it == Store.Phase.Ready || it == Store.Phase.SignedOut }
            if (store.phase == Store.Phase.Ready) go(store, uri, path)
        }
    }

    private fun query(uri: Uri, name: String) = uri.getQueryParameter(name)?.ifEmpty { null }

    private suspend fun go(store: Store, uri: Uri, path: List<String>) {
        val scope = query(uri, "scope")
        val params = query(uri, "params") ?: "{}"
        when (path[0]) {
            "vault" -> {
                store.path.clear()
                store.present(Route.Own("request", obj("scope" to scope, "id" to query(uri, "id")).toString()))
            }
            "page" -> store.openPage(path[1])
            "accounts" -> {
                store.openPage(path[1])
                store.present(Route.Own("accounts", obj("scope" to path[1]).toString()))
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
