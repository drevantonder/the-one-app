package app.client

import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.Star
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

// Settings, the last page, and the views it opens.

/** The cog's page: who you are, then what this phone offers of the iPhone app's Settings, signing out, and this build. */
@Composable
fun SettingsPage(store: Store) {
    val p = LocalPalette.current
    val context = LocalContext.current
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).windowInsetsPadding(WindowInsets.statusBars)
            .padding(horizontal = 20.dp).padding(top = 20.dp, bottom = 110.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        Profile(store)
        if (store.requests > 0) Section {
            SettingsRow("Requests", glyph = null, icon = Icons.Rounded.Notifications, color = Color(0xFFFF9500), value = "${store.requests}", badge = true) { store.path.add(Route.Requests) }
        }
        Section {
            SettingsRow("App Store", glyph = "square.grid.2x2", color = Color(0xFF007AFF)) { store.path.add(store.storeApp?.let { Route.Screen(it.name) } ?: Route.AppStore) }
            SettingsRow("Devices", glyph = "desktopcomputer", color = Color(0xFF8E8E93)) { store.path.add(Route.Devices) }
        }
        Section {
            SettingsRow("Passkeys", icon = Icons.Rounded.Lock, color = Color(0xFF34C759)) { SignIn.openInTab(context, Uri.parse("${store.signIn.address}/passkeys")) }
            SettingsRow("Appearance", icon = Icons.Rounded.Star, color = Color(0xFF5856D6), value = store.appearance.title) { store.path.add(Route.Appearance) }
        }
        Text(
            "${store.config?.name ?: BuildConfig.NAME}, app ${BuildConfig.COMMIT}" + (store.signIn.address?.let { "\n" + Uri.parse(it).host } ?: ""),
            style = Type.footnote, color = p.secondary, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth(),
        )
    }
}

/** Who you are, at the top of Settings: your initials, name, role and install, this phone, and Sign Out. */
@Composable
private fun Profile(store: Store) {
    val p = LocalPalette.current
    val me = store.me ?: return
    val context = LocalContext.current
    Row(Modifier.fillMaxWidth().background(p.card, RoundedCornerShape(24.dp)).padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
        Avatar(me.id, me.name, 60.dp)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(me.name, style = Type.title2, color = p.label, maxLines = 2)
            Text(if (me.admin) "Admin of ${store.install?.name ?: ""}" else "Member of ${store.install?.name ?: ""}", style = Type.subheadline, color = p.secondary)
            Text(SignIn.deviceName(context), style = Type.footnote, color = p.secondary)
        }
        TextButton({ store.signOut() }) { Text("Sign Out", color = p.accent, style = Type.callout, fontWeight = FontWeight.Medium) }
    }
}

/** A grouped container of rows, as a list's inset section draws one, with a footer under it. */
@Composable
fun Section(footer: String? = null, header: String? = null, content: @Composable ColumnScope.() -> Unit) {
    val p = LocalPalette.current
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        header?.let { Text(it, style = Type.footnote, color = p.secondary, modifier = Modifier.padding(horizontal = 16.dp)) }
        Column(Modifier.fillMaxWidth().background(p.card, RoundedCornerShape(24.dp)).padding(vertical = 4.dp), content = content)
        footer?.let { Text(it, style = Type.footnote, color = p.secondary, modifier = Modifier.padding(horizontal = 16.dp)) }
    }
}

@Composable
private fun SettingsRow(title: String, glyph: String? = null, icon: androidx.compose.ui.graphics.vector.ImageVector? = null, color: Color, value: String = "", badge: Boolean = false, open: () -> Unit) {
    val p = LocalPalette.current
    Row(Modifier.fillMaxWidth().clickable(onClick = open).heightIn(min = 50.dp).padding(horizontal = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
        SymbolTile(icon, glyph, color)
        Text(title, style = Type.body, color = p.label, modifier = Modifier.weight(1f))
        if (badge) {
            Box(Modifier.background(p.red, CircleShape).padding(horizontal = 8.dp, vertical = 2.dp)) { Text(value, style = Type.callout, fontWeight = FontWeight.SemiBold, color = Color.White) }
        } else if (value.isNotEmpty()) Text(value, style = Type.callout, color = p.secondary)
        Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, null, tint = p.tertiary)
    }
}

/** A row of a section: an icon, a title and a line under it, something at its end. */
@Composable
fun ListRow(icon: (@Composable () -> Unit)? = null, title: String, subtitle: String? = null, end: (@Composable () -> Unit)? = null, color: Color? = null, onClick: (() -> Unit)? = null) {
    val p = LocalPalette.current
    Row(
        Modifier.fillMaxWidth().let { if (onClick != null) it.clickable(onClick = onClick) else it }.heightIn(min = 50.dp).padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        icon?.invoke()
        Column(Modifier.weight(1f)) {
            Text(title, style = Type.body, color = color ?: p.label, maxLines = 2, overflow = TextOverflow.Ellipsis)
            subtitle?.let { Text(it, style = Type.subheadline, color = p.secondary, maxLines = 3, overflow = TextOverflow.Ellipsis) }
        }
        end?.invoke()
    }
}

@Composable
fun Divided(first: Boolean) {
    if (!first) HorizontalDivider(Modifier.padding(start = 16.dp), color = LocalPalette.current.separator, thickness = 0.5.dp)
}

@Composable
private fun Scrolling(content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(top = 8.dp, bottom = 40.dp), verticalArrangement = Arrangement.spacedBy(24.dp), content = content)
}

// MARK: Devices

/** This person's devices, every server they can see, and which of them keep their data. */
@Composable
fun DevicesView(store: Store) {
    val p = LocalPalette.current
    val install = store.install ?: return
    val me = store.me?.id ?: ""
    val mine = install.servers.filter { it.user == me }
    val others = install.servers.filter { it.user != me }
    val phones = install.clients.filter { it.user == me }
    Scrolling {
        Section(header = "Your Devices") {
            mine.forEachIndexed { i, s -> Divided(i == 0); ServerRow(store, s) }
            phones.forEachIndexed { i, c -> Divided(i == 0 && mine.isEmpty()); ClientRow(c) }
            if (mine.isEmpty() && phones.isEmpty()) ListRow(title = "None yet")
        }
        if (others.isNotEmpty()) Section(header = "Servers") { others.forEachIndexed { i, s -> Divided(i == 0); ServerRow(store, s) } }
        val holding = install.servers.filter { it.privileged == true || it.owners.contains("user:$me") || it.trustedBy?.contains(me) == true }
        Section(header = "Where Your Data Lives", footer = "Only these keep your own data.") {
            ListRow(title = holding.joinToString(", ") { displayName(it.name) }.ifEmpty { "Nowhere yet" })
        }
    }
}

@Composable
private fun ServerRow(store: Store, server: Server) {
    val p = LocalPalette.current
    val online = server.online == true
    val owners = server.owners.joinToString(", ") { o ->
        when {
            o == "install" -> "the install"
            o.startsWith("group:") -> store.install?.group(o.removePrefix("group:"))?.name ?: o
            else -> o.removePrefix("user:").let { if (it == store.me?.id) "you" else store.userName(it) }
        }
    }
    val detail = if (server.privileged == true) "Privileged · keeps everyone’s data" else "Run by $owners"
    ListRow(
        icon = { Symbol(deviceGlyph(server.name), if (online) p.accent else p.tertiary, 26.dp, Modifier.width(34.dp)) },
        title = displayName(server.name),
        subtitle = listOfNotNull(detail, server.load?.summary?.takeIf { online }).joinToString("\n"),
        end = {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Box(Modifier.size(8.dp).background(if (online) p.green else p.fill, CircleShape))
                Text(if (online) "Online" else "Offline", style = Type.callout, color = p.secondary)
            }
        },
    )
}

@Composable
private fun ClientRow(client: ClientInfo) {
    val p = LocalPalette.current
    ListRow(
        icon = { PhoneGlyph(p.accent) },
        title = displayName(client.name.ifEmpty { "Phone" }),
        subtitle = client.asked?.let { "Phone · asked ${ago(it.toLong())} ago".replace("now ago", "just now") } ?: "Phone",
    )
}

/** A phone's outline, for devices that are phones. */
@Composable
private fun PhoneGlyph(color: Color) {
    Box(Modifier.width(34.dp), contentAlignment = Alignment.Center) {
        Box(Modifier.size(width = 15.dp, height = 24.dp)) {
            androidx.compose.foundation.Canvas(Modifier.fillMaxSize()) {
                val r = androidx.compose.ui.geometry.CornerRadius(3.5.dp.toPx())
                drawRoundRect(color, cornerRadius = r, style = androidx.compose.ui.graphics.drawscope.Stroke(1.6.dp.toPx()))
                drawLine(color, androidx.compose.ui.geometry.Offset(size.width * 0.38f, size.height - 3.5.dp.toPx()), androidx.compose.ui.geometry.Offset(size.width * 0.62f, size.height - 3.5.dp.toPx()), 1.6.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
            }
        }
    }
}

// MARK: App Store

/** The App Store, where the install has no store app on these pages: every app, and the pages each is on. */
@Composable
fun AppStoreView(store: Store) {
    val p = LocalPalette.current
    var adding by remember { mutableStateOf<AppInfo?>(null) }
    Scrolling {
        Section(footer = "Apps without a screen or widget are in each page’s Services folder.") {
            store.apps.forEachIndexed { i, app ->
                Divided(i == 0)
                val on = store.pages.filter { app.scopes.contains(it.scope) }.map { it.title }
                ListRow(
                    icon = { AppTile(app, app.name, 44.dp) },
                    title = app.label,
                    subtitle = (if (on.isEmpty()) "Not on any of your pages" else "On ${on.joinToString(", ")}") + if (app.hasScreens) "" else " · no screen",
                    end = { TextButton({ adding = app }) { Text("Add to…", color = p.accent) } },
                )
            }
        }
    }
    adding?.let { AddToSheet(store, it) { adding = null } }
}

/** The pages this person may add an app to, each with whether it's there: tap to add it or take it off. */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun AddToSheet(store: Store, app: AppInfo, dismiss: () -> Unit) {
    val p = LocalPalette.current
    var busy by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    ModalBottomSheet(dismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = p.background) {
        Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 32.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Add ${app.label} to…", style = Type.headline, color = p.label, modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Center)
            Section(footer = error ?: "Everyone on a page sees what you add to it. Groups you don’t admin are their admins’ to arrange.") {
                store.pages.filter { store.canArrange(it.scope) }.forEachIndexed { i, page ->
                    Divided(i == 0)
                    val on = (store.app(app.name) ?: app).scopes.contains(page.scope)
                    ListRow(title = page.title, end = {
                        if (busy == page.scope) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp, color = p.secondary)
                        else if (on) Icon(Icons.Rounded.Check, "On ${page.title}", tint = p.accent)
                    }) {
                        busy = page.scope
                        error = null
                        store.launch {
                            error = store.change { store.post("/v1/install", obj("app" to app.name, "scope" to page.scope, "installed" to !on)) }
                            busy = null
                        }
                    }
                }
            }
        }
    }
}

// MARK: Requests

/** The requests for Vault items waiting for this person: each opens in the Vault's sheet. */
@Composable
fun RequestsView(store: Store) {
    val p = LocalPalette.current
    var list by remember { mutableStateOf<List<VaultRequest>?>(null) }
    LaunchedEffect(store.state) { list = runCatching { store.get<List<VaultRequest>>("/v1/vault/requests").filter { it.state == "waiting" } }.getOrDefault(emptyList()) }
    Scrolling {
        val waiting = list
        if (waiting == null) Box(Modifier.fillMaxWidth().padding(40.dp), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = p.secondary) }
        else if (waiting.isEmpty()) Text("Nothing waits for you.", style = Type.body, color = p.secondary, modifier = Modifier.fillMaxWidth().padding(top = 40.dp), textAlign = TextAlign.Center)
        else Section(header = "Vault") {
            waiting.forEachIndexed { i, r ->
                Divided(i == 0)
                val app = store.app(r.app)
                ListRow(icon = { AppTile(app, r.app, 30.dp) }, title = "${app?.label ?: r.app} wants ${r.label}", subtitle = "${r.asker} · ${r.scopeName}", end = {
                    Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, null, tint = p.tertiary)
                }) { store.sheet = VaultSheet(r.scope, r.id) }
            }
        }
    }
}

// MARK: Appearance

@Composable
fun AppearanceView() {
    val store = LocalStore.current
    val p = LocalPalette.current
    Scrolling {
        Section(footer = "Automatic follows this phone’s light and dark.") {
            Appearance.entries.forEachIndexed { i, a ->
                Divided(i == 0)
                ListRow(title = a.title, end = { if (store.appearance == a) Icon(Icons.Rounded.Check, null, tint = p.accent) }) { store.choose(a) }
            }
        }
    }
}

val LocalStore = androidx.compose.runtime.staticCompositionLocalOf<Store> { error("no store") }
