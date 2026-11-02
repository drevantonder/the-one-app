package app.client

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// Settings, the last page: its items' icons, each opening its Settings screen (Route.Setting); and the rows the
// shell's own sheets are made of.

/** Settings, as a page: its items' icons in a page's grid, you first, as your picture. */
@Composable
fun SettingsPage(store: Store) {
    val items = store.settingsItems
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).windowInsetsPadding(WindowInsets.statusBars).padding(horizontal = 20.dp).padding(top = 20.dp, bottom = 110.dp)) {
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val grid = GridMetrics(maxWidth)
            val cells = grid.place(items.map { 1 to 1 })
            Layout(content = {
                items.forEach { item -> IconCell(item.title, item.badge ?: 0, { store.path.add(Route.Setting(item.name)) }) { SettingIcon(store, item, grid.icon) } }
            }) { measurables, _ ->
                val cell = grid.cell.roundToPx()
                val gap = grid.gap.roundToPx()
                val placeables = measurables.map { it.measure(Constraints.fixed(cell, cell)) }
                val rows = cells.maxOfOrNull { it.second + 1 } ?: 0
                layout(cell * grid.columns + gap * (grid.columns - 1), if (rows > 0) cell * rows + gap * (rows - 1) else 0) {
                    placeables.forEachIndexed { i, placeable -> placeable.place(cells[i].first * (cell + gap), cells[i].second * (cell + gap)) }
                }
            }
        }
    }
}

/** A Settings item's icon: yours is your picture; the rest, its symbol on its colour, as an app's tile. */
@Composable
fun SettingIcon(store: Store, item: SettingsItem, size: Dp) {
    val me = store.me
    if (item.name == "account" && me != null) Avatar(me.id, me.name, size)
    else AppTile(TileLook(item.icon, item.name, item.color), size, item.title)
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

val LocalStore = androidx.compose.runtime.staticCompositionLocalOf<Store> { error("no store") }
