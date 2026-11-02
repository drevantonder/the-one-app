package app.client

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull

/** An app's account, as its `accounts` operation lists it. */
private data class Account(val account: String, val ok: Boolean, val why: String?)

/** A page's Services: a section per app that keeps accounts, each with a dot saying whether it's signed in, then Add
 *  Account; then the rest, each opening its page in the App Store; then Add a Service. */
@Composable
fun ServicesView(store: Store, scope: String) {
    val p = LocalPalette.current
    val page = store.page(scope) ?: return
    val all = store.servicesOn(page)
    val withAccounts = all.filter { it.hasOp("accounts") }
    val others = all - withAccounts.toSet()
    val accounts = remember { mutableStateMapOf<String, Result<List<Account>>>() }
    suspend fun list(a: AppInfo) {
        accounts[a.name] = runCatching {
            val out = store.call(a.name, "accounts") as? JsonArray ?: throw NodeError("${a.label} doesn’t list its accounts yet.")
            out.mapNotNull { it as? JsonObject }.map { o ->
                Account((o["account"] as? JsonPrimitive)?.contentOrNull ?: "", (o["ok"] as? JsonPrimitive)?.booleanOrNull == true, (o["why"] as? JsonPrimitive)?.contentOrNull)
            }
        }
    }
    LaunchedEffect(withAccounts.map { it.name }) { withAccounts.filter { !accounts.containsKey(it.name) }.forEach { launch { list(it) } } }
    // A change to an app (a sign-in finished on its settings screen) lists its accounts again.
    LaunchedEffect(Unit) { store.changed.collect { names -> withAccounts.filter { names.isEmpty() || it.name in names }.forEach { launch { list(it) } } } }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(bottom = 40.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
        Text(page.whose, style = Type.footnote, color = p.secondary, modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Center)
        for (a in withAccounts) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(Modifier.padding(start = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    AppTile(a, a.name, 28.dp)
                    Text(a.label, style = Type.headline, color = p.label, modifier = Modifier.weight(1f))
                    AppMenu(store, a, page)
                }
                Section {
                    val list = accounts[a.name]
                    when {
                        list == null -> ListRow(title = "Loading…")
                        list.isFailure -> ListRow(title = list.exceptionOrNull()?.message ?: "")
                        list.getOrThrow().isEmpty() -> ListRow(title = "No accounts yet")
                        else -> list.getOrThrow().forEachIndexed { i, x ->
                            Divided(i == 0)
                            ListRow(
                                icon = { Box(Modifier.size(9.dp).background(if (x.ok) p.green else p.orange, CircleShape)) },
                                title = x.account,
                                subtitle = if (x.ok) null else x.why ?: "Needs signing in again",
                                end = store.settingsRoute(a, page, add = true)?.takeIf { !x.ok }?.let { r -> { TextButton({ store.path.add(r) }) { Text("Sign In", color = p.accent) } } },
                            )
                        }
                    }
                    store.settingsRoute(a, page, add = true)?.let { r ->
                        Divided(false)
                        ListRow(icon = { Icon(Icons.Rounded.Add, null, tint = p.accent) }, title = "Add Account") { store.path.add(r) }
                    }
                }
            }
        }
        if (others.isNotEmpty()) Section {
            others.forEachIndexed { i, a ->
                Divided(i == 0)
                val to = store.storeRoute(a) ?: store.settingsRoute(a, page)
                ListRow(icon = { AppTile(a, a.name, 28.dp) }, title = a.label, subtitle = a.description, end = to?.let { { Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, null, tint = p.tertiary) } }, onClick = to?.let { { store.path.add(it) } })
            }
        }
        if (all.isEmpty()) Text("No services yet.", style = Type.body, color = p.secondary, modifier = Modifier.fillMaxWidth().padding(top = 24.dp), textAlign = TextAlign.Center)
        val shop = store.storeApp
        if (store.canArrange(page.scope) && shop != null) Section {
            ListRow(icon = { Icon(Icons.Rounded.Add, null, tint = p.accent) }, title = "Add a Service") { store.path.add(Route.Screen(shop.name, params = "{\"service\":true}")) }
        }
    }
}

/** An app's ⋯ menu: its settings, its page in the App Store, taking it off this page, and deleting it. */
@Composable
private fun AppMenu(store: Store, a: AppInfo, page: Page) {
    val p = LocalPalette.current
    var open by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<String?>(null) }
    val actions = appActions(store, a, page, { deleting = true }, { problem = it })
    if (actions.isEmpty()) return
    Box {
        IconButton({ open = true }) { Icon(Icons.Rounded.MoreVert, "More for ${a.label}", tint = p.secondary) }
        DropdownMenu(open, { open = false }) {
            for ((label, danger, run) in actions) DropdownMenuItem(text = { Text(label, color = if (danger) p.red else p.label, fontWeight = FontWeight.Normal) }, onClick = { open = false; run() })
        }
    }
    if (deleting) DeleteApp(store, a) { deleting = false }
    problem?.let { Problem(it) { problem = null } }
}
