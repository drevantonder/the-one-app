package app.client

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.windowInsetsTopHeight
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/** The pages side by side, then Settings: swipe between them, scroll within one. At the bottom, a dot per page, with a
 *  cog as Settings' dot, and the current page's name above them; the pages have no titles of their own. What a page
 *  opens is pushed over them, with Back. */
@Composable
fun PagesView(store: Store) {
    val top = store.path.lastOrNull()
    BackHandler(enabled = top != null) { store.path.removeAt(store.path.lastIndex) }
    AnimatedContent(
        targetState = top,
        transitionSpec = {
            val forward = (targetState != null && initialState == null) || store.path.size > 1 && targetState != null
            if (forward) (slideInHorizontally { it / 3 } + fadeIn()) togetherWith (slideOutHorizontally { -it / 6 } + fadeOut())
            else (slideInHorizontally { -it / 6 } + fadeIn()) togetherWith (slideOutHorizontally { it / 3 } + fadeOut())
        },
        label = "route",
    ) { route ->
        if (route == null) Home(store) else RouteView(store, route)
    }
    store.sheet?.let { VaultRequestSheet(store, it) }
}

@Composable
private fun Home(store: Store) {
    val p = LocalPalette.current
    val pages = store.pages
    val positions = pages.map { it.scope } + Store.SETTINGS
    val pager = rememberPagerState(initialPage = positions.indexOf(store.current).coerceAtLeast(0)) { positions.size }
    val scope = rememberCoroutineScope()
    LaunchedEffect(pager.currentPage, positions.size) { store.current = positions.getOrElse(pager.currentPage) { "user" } }
    LaunchedEffect(store.current) {
        val i = positions.indexOf(store.current)
        if (i >= 0 && i != pager.currentPage) pager.animateScrollToPage(i)
    }
    Box(Modifier.fillMaxSize().background(p.background)) {
        HorizontalPager(pager, Modifier.fillMaxSize(), beyondViewportPageCount = 1) { i ->
            val page = pages.getOrNull(i)
            if (page != null) PageView(store, page) else SettingsPage(store)
        }
        BottomBar(
            titles = pages.map { it.title } + "Settings",
            index = pager.currentPage,
            attention = store.requests > 0,
            go = { scope.launch { pager.animateScrollToPage(it) } },
            modifier = Modifier.align(Alignment.BottomCenter),
        )
        if (store.state == null || store.install == null) Starting(store)
    }
}

/** A dot per page, the last a cog for Settings, in a capsule, with the current page's name above it. */
@Composable
private fun BottomBar(titles: List<String>, index: Int, attention: Boolean, go: (Int) -> Unit, modifier: Modifier) {
    val p = LocalPalette.current
    var menu by remember { mutableStateOf(false) }
    Column(modifier.windowInsetsPadding(WindowInsets.navigationBars).padding(bottom = 10.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(titles.getOrElse(index) { "" }, style = Type.footnote, fontWeight = FontWeight.SemiBold, color = p.secondary, maxLines = 2, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 40.dp))
        Spacer(Modifier.height(6.dp))
        Row(
            Modifier.shadow(10.dp, CircleShape, ambientColor = Color.Black.copy(alpha = 0.2f), spotColor = Color.Black.copy(alpha = 0.2f))
                .clip(CircleShape).background(if (p.dark) Color(0xE62C2C2E) else Color(0xEBFFFFFF))
                .border(0.5.dp, p.separator.copy(alpha = 0.3f), CircleShape).height(44.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box {
                Row(
                    Modifier.clip(CircleShape).combinedClickable(onClick = { menu = true }).padding(start = 18.dp, end = 6.dp).height(44.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    for (i in 0 until titles.size - 1) {
                        Box(Modifier.width(if (i == index) 16.dp else 7.dp).height(7.dp).background(if (i == index) p.label else p.tertiary, CircleShape))
                    }
                }
                DropdownMenu(menu, { menu = false }) {
                    titles.forEachIndexed { i, t ->
                        DropdownMenuItem(text = { Text(t, fontWeight = if (i == index) FontWeight.SemiBold else null) }, onClick = { menu = false; go(i) })
                    }
                }
            }
            Box(Modifier.clip(CircleShape).combinedClickable(onClick = { go(titles.lastIndex) }).padding(start = 6.dp, end = 18.dp).height(44.dp), contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.Settings, "Settings", Modifier.size(17.dp), tint = if (index == titles.lastIndex) p.label else p.tertiary)
                if (attention) Box(Modifier.align(Alignment.TopEnd).offset(x = 3.dp, y = 12.dp).size(7.dp).background(p.red, CircleShape))
            }
        }
    }
}

/** Before the node has answered: a spinner, or that it can't be reached and keeps trying. */
@Composable
private fun Starting(store: Store) {
    val p = LocalPalette.current
    Box(Modifier.fillMaxSize().background(p.background), contentAlignment = Alignment.Center) {
        if (store.phase == Store.Phase.Unreachable) Unavailable("Can’t Reach Your Servers", (store.failure ?: "") + "\nIt keeps trying.")
        else if (store.state != null && store.install == null) Unavailable("Nothing Here Yet", "This server isn’t in an install yet.")
        else androidx.compose.material3.CircularProgressIndicator(color = p.secondary)
    }
}

// MARK: A page

/** The Home Screen's grid: square cells four across a phone, items placed row by row in the first place each fits,
 *  widgets starting on an even column; an xl widget where it doesn't fit is large. */
class GridMetrics(width: Dp) {
    val gap: Dp
    val columns: Int
    val cell: Dp
    val icon: Dp

    init {
        if (width < 500.dp) {
            gap = 12.dp
            columns = 4
            cell = (width - gap * 3) / 4
        } else {
            gap = 14.dp
            cell = 82.dp
            columns = maxOf(4, ((width + gap) / (cell + gap)).toInt())
        }
        icon = (cell - 19.dp).coerceIn(40.dp, 60.dp)
    }

    fun span(n: Int) = cell * n + gap * (n - 1)

    /** Each item's cell, (x, y), for its span: the first place it fits, row by row. */
    fun place(spans: List<Pair<Int, Int>>): List<Pair<Int, Int>> {
        val taken = mutableListOf<BooleanArray>()
        fun free(x: Int, y: Int, w: Int, h: Int) = (y until y + h).all { row -> (x until x + w).none { col -> taken.getOrNull(row)?.get(col) == true } }
        return spans.map { (w0, h) ->
            val w = minOf(w0, columns)
            val (x, y) = generateSequence(0) { it + 1 }.firstNotNullOf { y -> (0..columns - w step if (w > 1) 2 else 1).firstOrNull { free(it, y, w, h) }?.let { it to y } }
            while (taken.size < y + h) taken.add(BooleanArray(columns))
            for (row in y until y + h) for (col in x until x + w) taken[row][col] = true
            x to y
        }
    }
}

val CELLS = mapOf("small" to (2 to 2), "medium" to (4 to 2), "large" to (4 to 4), "xl" to (8 to 4))

@Composable
fun PageView(store: Store, page: Page) {
    val p = LocalPalette.current
    val items = store.items(page)
    val empty = store.appsOn(page).isEmpty() && store.widgetsOn(page).isEmpty()
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).windowInsetsPadding(WindowInsets.statusBars).padding(horizontal = 20.dp).padding(top = 20.dp, bottom = 110.dp)) {
        if (empty) {
            EmptyPage(store, page)
            Spacer(Modifier.height(20.dp))
        }
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val grid = GridMetrics(maxWidth)
            val sizes = items.map { if (it is PageItem.Widget && CELLS[it.size]!!.first > grid.columns) "large" else (it as? PageItem.Widget)?.size }
            val spans = items.mapIndexed { i, it -> if (it is PageItem.Widget) CELLS[sizes[i]] ?: (4 to 2) else 1 to 1 } + (1 to 1)
            val cells = grid.place(spans)
            Layout(content = {
                items.forEachIndexed { i, item ->
                    when (item) {
                        is PageItem.Icon -> AppIcon(store, page, item.app, grid)
                        is PageItem.Widget -> PageWidget(store, page, item, sizes[i]!!)
                    }
                }
                IconCell("Services", 0, { store.path.add(Route.Services(page.scope)) }) { FolderTile(store.servicesOn(page), grid.icon) }
            }) { measurables, _ ->
                val cell = grid.cell.roundToPx()
                val gap = grid.gap.roundToPx()
                val placeables = measurables.mapIndexed { i, m ->
                    val (w, h) = spans[i]
                    m.measure(Constraints.fixed(cell * w + gap * (w - 1), cell * h + gap * (h - 1)))
                }
                val rows = cells.zip(spans).maxOfOrNull { (c, s) -> c.second + s.second } ?: 0
                layout(cell * grid.columns + gap * (grid.columns - 1), if (rows > 0) cell * rows + gap * (rows - 1) else 0) {
                    placeables.forEachIndexed { i, placeable -> placeable.place(cells[i].first * (cell + gap), cells[i].second * (cell + gap)) }
                }
            }
        }
    }
}

@Composable
private fun EmptyPage(store: Store, page: Page) {
    val p = LocalPalette.current
    Column(Modifier.fillMaxWidth().background(p.card, RoundedCornerShape(24.dp)).padding(28.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text("Nothing here yet", style = Type.headline, color = p.label)
        Text(
            if (store.canArrange(page.scope)) "Apps you add to ${page.title} show up here." else "When its admins add apps to ${page.title}, they show up here.",
            style = Type.subheadline, color = p.secondary, textAlign = TextAlign.Center,
        )
        if (store.canArrange(page.scope)) {
            Spacer(Modifier.height(4.dp))
            Prominent("Open the App Store") { store.path.add(store.storeApp?.let { Route.Screen(it.name) } ?: Route.AppStore) }
        }
    }
}

/** An app's icon on a page: it opens its main screen; touch and hold for what else it offers. */
@Composable
private fun AppIcon(store: Store, page: Page, app: AppInfo, grid: GridMetrics) {
    var menu by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<String?>(null) }
    var deleting by remember { mutableStateOf(false) }
    Box {
        IconCell(app.label, store.badge(app.name, page), { store.path.add(Route.Screen(app.name, scope = page.scope)) }, { menu = true }) {
            AppTile(app, app.name, grid.icon)
        }
        DropdownMenu(menu, { menu = false }) {
            for ((label, danger, run) in appActions(store, app, page, { deleting = true }, { problem = it })) {
                DropdownMenuItem(text = { Text(label, color = if (danger) LocalPalette.current.red else LocalPalette.current.label) }, onClick = { menu = false; run() })
            }
        }
    }
    problem?.let { Problem(it) { problem = null } }
    if (deleting) DeleteApp(store, app) { deleting = false }
}

/** What an app's menu offers: its settings, its page in the App Store, taking it off this page, and deleting it. */
fun appActions(store: Store, a: AppInfo, p: Page, delete: () -> Unit, failed: (String) -> Unit): List<Triple<String, Boolean, () -> Unit>> {
    val out = mutableListOf<Triple<String, Boolean, () -> Unit>>()
    store.settingsRoute(a, p)?.let { out += Triple("Settings", false) { store.path.add(it) } }
    store.storeRoute(a)?.let { out += Triple("Open in App Store", false) { store.path.add(it) } }
    if (store.canArrange(p.scope) && (a.deploys ?: 0) > 0) {
        out += Triple("Remove from ${p.title}", true) {
            store.launch { store.change { store.post("/v1/install", obj("app" to a.name, "scope" to p.scope, "installed" to false)) }?.let(failed) }
        }
    }
    if (store.owns(a) && (a.deploys ?: 0) > 0) out += Triple("Delete App…", true, delete)
    return out
}

@Composable
fun DeleteApp(store: Store, a: AppInfo, dismiss: () -> Unit) {
    val p = LocalPalette.current
    androidx.compose.material3.AlertDialog(
        onDismissRequest = dismiss,
        title = { Text("Delete ${a.label}?") },
        text = { Text("${a.label} leaves every page, for everyone, and stops running. It waits in the App Store’s Recently Deleted, with its data, until its owners erase it.") },
        confirmButton = {
            androidx.compose.material3.TextButton({
                dismiss()
                store.launch { store.change { store.post("/v1/retire", obj("app" to a.name, "how" to "deleted")) } }
            }) { Text("Delete App", color = p.red) }
        },
        dismissButton = { androidx.compose.material3.TextButton(dismiss) { Text("Cancel") } },
    )
}

/** A large rounded icon with its name under it, as a home screen's, with a count in a red badge. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun IconCell(title: String, badge: Int, open: () -> Unit, hold: (() -> Unit)? = null, icon: @Composable () -> Unit) {
    val p = LocalPalette.current
    Column(
        Modifier.fillMaxWidth().combinedClickable(remember { MutableInteractionSource() }, null, onLongClick = hold, onClick = open),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box {
            icon()
            if (badge > 0) {
                Box(Modifier.align(Alignment.TopEnd).offset(x = 7.dp, y = (-7).dp).height(22.dp).widthIn(min = 22.dp).background(p.red, CircleShape).padding(horizontal = 6.dp), contentAlignment = Alignment.Center) {
                    Text(if (badge > 99) "99+" else "$badge", style = Type.caption, fontWeight = FontWeight.SemiBold, color = Color.White)
                }
            }
        }
        Spacer(Modifier.height(6.dp))
        Text(title, style = Type.caption, color = p.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** A page's Services folder: its services' icons, nine at most, small, on frosted glass; with none, nine faint places. */
@Composable
fun FolderTile(apps: List<AppInfo>, size: Dp) {
    val p = LocalPalette.current
    val small = size / 4.6f
    Box(Modifier.size(size).shadow(size * 0.06f, RoundedCornerShape(size * 0.225f)).background(if (p.dark) Color(0xFF3A3A3C) else Color(0xFFE5E5EA), RoundedCornerShape(size * 0.225f)).padding(size * 0.12f)) {
        Column(verticalArrangement = Arrangement.spacedBy(size * 0.05f)) {
            for (row in 0 until 3) Row(horizontalArrangement = Arrangement.spacedBy(size * 0.05f)) {
                for (col in 0 until 3) {
                    val a = apps.getOrNull(row * 3 + col)
                    if (a != null) AppTile(a.look(), small, a.label) else Box(Modifier.size(small).background(p.label.copy(alpha = 0.06f), RoundedCornerShape(small * 0.225f)))
                }
            }
        }
    }
}

/** A widget on a page: its card, opening its app unless it has a link of its own; touch and hold for its sizes. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PageWidget(store: Store, page: Page, item: PageItem.Widget, size: String) {
    val p = LocalPalette.current
    val w = item.widget
    val app = store.app(w.app)
    val uri = LocalUriHandler.current
    var menu by remember { mutableStateOf(false) }
    val tree = w.tree(size)
    val host = WidgetHost(
        app = w.app,
        run = { op, input ->
            try {
                store.call(w.app, op, input)
                store.refreshWidgets()
                null
            } catch (e: Exception) {
                e.message
            }
        },
        open = { link -> follow(store, page, w.app, link) { uri.openUri(it) } },
        load = { address -> address.removePrefix("/v1/blobs/").takeIf { address.startsWith("/v1/blobs/") }?.let { store.blob(it) } },
    )
    val opens = tree.link == null && app?.hasScreens == true
    Box {
        Box(
            Modifier.fillMaxSize().clip(RoundedCornerShape(22.dp)).combinedClickable(
                remember { MutableInteractionSource() }, null,
                onClick = { if (opens) store.path.add(Route.Screen(w.app, scope = page.scope)) },
                onLongClick = { menu = true },
            ),
        ) {
            androidx.compose.runtime.CompositionLocalProvider(LocalWidgetHost provides host) { WidgetCard(w, app, size) }
        }
        DropdownMenu(menu, { menu = false }) {
            val names = mapOf("small" to "Small", "medium" to "Medium", "large" to "Large", "xl" to "Extra Large")
            for (s in w.supported) {
                DropdownMenuItem(text = { Text((if (s == size) "✓ " else "") + names[s]) }, onClick = {
                    menu = false
                    if (s != size) store.arrange(page, { if (it.id == item.kept.id) it.copy(size = s) else it }) {}
                })
            }
            DropdownMenuItem(text = { Text("Remove Widget", color = p.red) }, onClick = {
                menu = false
                store.arrange(page, { if (it.id == item.kept.id) null else it }) {}
            })
        }
    }
}

/** A widget's link: a URL opens in the browser, `{ screen, params }` the app's screen on this page. */
fun follow(store: Store, page: Page, app: String, link: JsonElement, browse: (String) -> Unit) {
    val text = (link as? JsonPrimitive)?.contentOrNull
    if (text != null) {
        if (text.startsWith("http://") || text.startsWith("https://")) browse(text)
        return
    }
    val o = link as? JsonObject ?: return
    val screen = (o["screen"] as? JsonPrimitive)?.contentOrNull ?: return
    store.path.add(Route.Screen(app, screen, (o["params"] ?: JsonObject(emptyMap())).toString(), page.scope))
}

@Composable
fun WidgetCard(widget: WidgetInfo, app: AppInfo?, size: String) {
    val p = LocalPalette.current
    val tree = widget.tree(size)
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) { while (true) { delay(60_000); now = System.currentTimeMillis() } }
    Column(Modifier.fillMaxSize().background(p.card, RoundedCornerShape(22.dp)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (tree.header != false || widget.error != null) {
            Text(widget.title ?: app?.label ?: widget.widget, style = Type.caption2, fontWeight = FontWeight.Medium, color = p.secondary, maxLines = 1)
        }
        val error = widget.error
        if (error != null) {
            Text("This widget stopped working.", style = Type.subheadline, fontWeight = FontWeight.Medium, color = p.orange)
            Text(NodeError.clean(error), style = Type.caption, color = p.secondary, maxLines = 3)
        } else {
            androidx.compose.runtime.CompositionLocalProvider(LocalNow provides now) {
                ProvideTextStyle(Type.footnote.copy(color = p.label)) { WidgetTree(tree, modifier = Modifier.fillMaxWidth()) }
            }
        }
    }
}

// MARK: What's pushed over the pages

/** A pushed view: its bar, with Back and its title, over what it shows. */
@Composable
fun RouteView(store: Store, route: Route) {
    val p = LocalPalette.current
    var title by remember(route) { mutableStateOf<String?>(null) }
    val default = when (route) {
        is Route.Screen -> store.app(route.app).let { a -> a?.screens?.firstOrNull { it.name == route.screen }?.title ?: a?.label ?: "" }
        is Route.Services -> "Services"
        Route.Devices -> "Devices"
        Route.AppStore -> "App Store"
        Route.Requests -> "Requests"
        Route.Appearance -> "Appearance"
    }
    Column(Modifier.fillMaxSize().background(p.background)) {
        Spacer(Modifier.windowInsetsTopHeight(WindowInsets.statusBars))
        Box(Modifier.fillMaxWidth().height(48.dp)) {
            IconButton({ store.path.removeAt(store.path.lastIndex) }, Modifier.align(Alignment.CenterStart)) {
                Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back", tint = p.accent)
            }
            Text(
                title ?: default, style = Type.headline, color = p.label, maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center,
                modifier = Modifier.align(Alignment.Center).padding(horizontal = 56.dp),
            )
        }
        // Above the gesture bar, and the keyboard while it's up.
        Box(Modifier.weight(1f).fillMaxWidth().windowInsetsPadding(WindowInsets.navigationBars.union(WindowInsets.ime))) {
            when (route) {
                is Route.Screen -> AppScreen(store, route) { title = it }
                is Route.Services -> ServicesView(store, route.scope)
                Route.Devices -> DevicesView(store)
                Route.AppStore -> AppStoreView(store)
                Route.Requests -> RequestsView(store)
                Route.Appearance -> AppearanceView()
            }
        }
    }
}

/** A large accent button, as the iPhone's prominent ones. */
@Composable
fun Prominent(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    val p = LocalPalette.current
    Button(onClick, modifier, enabled = enabled, colors = ButtonDefaults.buttonColors(containerColor = p.accent, contentColor = Color.White), shape = CircleShape) {
        Text(text, style = Type.body, fontWeight = FontWeight.SemiBold)
    }
}

/** Nothing to show, and why. */
@Composable
fun Unavailable(title: String, text: String) {
    val p = LocalPalette.current
    Column(Modifier.fillMaxSize().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
        Text(title, style = Type.title2, color = p.label, textAlign = TextAlign.Center)
        Spacer(Modifier.height(8.dp))
        Text(text, style = Type.subheadline, color = p.secondary, textAlign = TextAlign.Center)
    }
}
