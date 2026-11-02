package app.client

import android.graphics.BitmapFactory
import android.text.format.DateUtils
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.asComposePath
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.isSpecified
import androidx.compose.ui.unit.sp
import androidx.core.graphics.PathParser
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.text.DateFormat
import java.util.Date

/** What a widget's buttons, toggles and links act through, where it's drawn. */
data class WidgetHost(
    val app: String = "",
    /** Runs an operation of the app → the error's text if it failed. */
    val run: (suspend (op: String, input: JsonElement) -> String?)? = null,
    /** Follows a link: a URL, or `{ screen, params }` of the app. */
    val open: ((JsonElement) -> Unit)? = null,
    /** An image the node keeps, by its address there. */
    val load: (suspend (String) -> ByteArray?)? = null,
)

val LocalWidgetHost = compositionLocalOf { WidgetHost() }

/** What `time` nodes count from. */
val LocalNow = compositionLocalOf { System.currentTimeMillis() }

/** Draws a widget's tree. Text of the default size takes the surrounding style, so each place sets its own. */
@Composable
fun WidgetTree(node: WidgetNode, inRow: Boolean = false, modifier: Modifier = Modifier) {
    val link = node.link
    if (link != null && node.t != "item" && node.t != "link") {
        Linked(link, modifier) { Content(node, inRow) }
    } else {
        Box(modifier) { Content(node, inRow) }
    }
}

@Composable
private fun Content(node: WidgetNode, inRow: Boolean) {
    val p = LocalPalette.current
    val dim = Modifier.alpha(if (node.dim == true) 0.5f else 1f)
    when (node.t) {
        "col" -> Column(
            dim.then(node.w?.let { Modifier.width(it.dp) } ?: Modifier),
            verticalArrangement = Arrangement.spacedBy((node.gap ?: 4.0).dp),
            horizontalAlignment = if (node.align == "center") Alignment.CenterHorizontally else Alignment.Start,
        ) { Children(node, inRow = false) }
        "row" -> Row(
            dim.then(node.w?.let { Modifier.width(it.dp) } ?: Modifier),
            horizontalArrangement = Arrangement.spacedBy((node.gap ?: 6.0).dp, if (node.align == "center") Alignment.CenterHorizontally else Alignment.Start),
            verticalAlignment = Alignment.CenterVertically,
        ) { Children(node, inRow = true) }
        "text" -> Text(
            node.text ?: "", style = widgetStyle(node.size), color = toned(node.tone, p.label),
            fontWeight = when (node.weight) { "bold" -> FontWeight.Bold; "medium" -> FontWeight.SemiBold; else -> null },
            maxLines = node.lines ?: if (inRow) 1 else Int.MAX_VALUE, overflow = TextOverflow.Ellipsis,
        )
        "time" -> Text(time(node, LocalNow.current), style = widgetStyle(node.size), color = toned(node.tone, p.label), maxLines = 1)
        "stat" -> Stat(node)
        "gauge" -> Ring(node)
        "progress" -> Bar(node)
        "spark" -> Spark(node)
        "chart" -> Chart(node)
        "list" -> WidgetList(node)
        "item" -> WidgetItem(node)
        "image" -> WidgetImage(node)
        "icon" -> WidgetIcon(node)
        "dot" -> Box(Modifier.size(8.dp).background(toned(node.tone, p.green), CircleShape))
        "link" -> node.link?.let { Linked(it) { Text(node.text ?: "", style = widgetStyle(node.size), color = p.accent, maxLines = node.lines ?: 1, overflow = TextOverflow.Ellipsis) } }
        "button" -> ActionButton(node)
        "toggle" -> ActionToggle(node)
        "divider" -> HorizontalDivider(color = p.separator, thickness = 0.5.dp)
        else -> {}
    }
}

@Composable
private fun ColumnScope.Children(node: WidgetNode, inRow: Boolean) {
    for (child in node.c.orEmpty()) {
        WidgetTree(child, inRow, if (child.grow == true) Modifier.weight(1f, fill = false) else Modifier)
    }
}

@Composable
private fun RowScope.Children(node: WidgetNode, inRow: Boolean) {
    // In a row, a node that grows takes what its siblings leave; text keeps to one line.
    for (child in node.c.orEmpty()) {
        WidgetTree(child, inRow, if (child.grow == true) Modifier.weight(1f) else Modifier)
    }
}

/** The text style a node's `size` names; the surrounding one by default. */
@Composable
fun widgetStyle(size: String?) = when (size) {
    "xl" -> Type.title2
    "lg" -> Type.headline
    "sm" -> Type.footnote
    "xs" -> Type.caption2
    else -> LocalTextStyle.current
}

/** A tone's colour: `muted` is secondary, and with no tone, `otherwise`. */
@Composable
fun toned(tone: String?, otherwise: Color): Color {
    val p = LocalPalette.current
    return when (tone) {
        "muted" -> p.secondary
        "accent" -> p.accent
        "good" -> p.green
        "warn" -> p.orange
        "bad" -> p.red
        "blue" -> p.blue
        "purple" -> p.purple
        else -> otherwise
    }
}

/** "now", "5 min", "3 h", then the weekday, then the date; or as the node's `style` asks. */
private fun time(node: WidgetNode, now: Long): String {
    val at = (node.at ?: 0.0).toLong()
    return when (node.style) {
        "relative" -> DateUtils.getRelativeTimeSpanString(at, now, DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_RELATIVE).toString()
        "timer" -> DateUtils.formatElapsedTime(kotlin.math.abs(now - at) / 1000)
        "date" -> DateUtils.formatDateTime(null, at, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_NO_YEAR or DateUtils.FORMAT_ABBREV_MONTH)
        "clock" -> DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(at))
        else -> ago(at, now)
    }
}

fun ago(at: Long, now: Long = System.currentTimeMillis()): String {
    val s = (now - at) / 1000
    return when {
        s < 60 -> "now"
        s < 3600 -> "${s / 60} min"
        s < 86400 -> "${s / 3600} h"
        s < 7 * 86400 -> android.text.format.DateFormat.format("EEE", at).toString()
        else -> DateUtils.formatDateTime(null, at, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_NO_YEAR or DateUtils.FORMAT_ABBREV_MONTH)
    }
}

private fun fraction(node: WidgetNode): Float {
    val max = node.max ?: 1.0
    return if (max > 0) ((node.value.number ?: 0.0) / max).coerceIn(0.0, 1.0).toFloat() else 0f
}

@Composable
private fun Stat(node: WidgetNode) {
    val p = LocalPalette.current
    Column {
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(node.value.text ?: "–", fontSize = 28.sp, fontWeight = FontWeight.SemiBold, color = toned(node.tone, p.label), maxLines = 1, lineHeight = 32.sp)
            node.unit?.let { Text(it, style = Type.subheadline, fontWeight = FontWeight.Medium, color = p.secondary, modifier = Modifier.padding(bottom = 3.dp)) }
        }
        node.label?.let { Text(it, style = Type.caption, color = p.secondary, maxLines = 1) }
    }
}

@Composable
private fun Ring(node: WidgetNode) {
    val p = LocalPalette.current
    val f = fraction(node)
    val size = (node.w ?: 46.0).dp
    val color = toned(node.tone, p.accent)
    val track = p.secondary.copy(alpha = 0.25f)
    Box(Modifier.size(size), contentAlignment = Alignment.Center) {
        Canvas(Modifier.size(size)) {
            val line = minOf(5.dp.toPx(), this.size.width / 6)
            val inset = line / 2
            val arc = androidx.compose.ui.geometry.Size(this.size.width - line, this.size.height - line)
            drawArc(track, 0f, 360f, false, Offset(inset, inset), arc, style = Stroke(line))
            drawArc(color, -90f, 360f * f, false, Offset(inset, inset), arc, style = Stroke(line, cap = StrokeCap.Round))
        }
        if (node.label != "") {
            Text(node.label ?: "${Math.round(f * 100)}%", fontSize = 11.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, color = p.label)
        }
    }
}

@Composable
private fun Bar(node: WidgetNode) {
    val p = LocalPalette.current
    val f = fraction(node)
    val color = toned(node.tone, p.accent)
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        node.label?.let { Text(it, style = Type.caption, color = p.secondary, maxLines = 1) }
        Canvas((node.w?.let { Modifier.width(it.dp) } ?: Modifier.fillMaxWidth()).height(6.dp)) {
            val r = androidx.compose.ui.geometry.CornerRadius(size.height / 2)
            drawRoundRect(p.secondary.copy(alpha = 0.25f), cornerRadius = r)
            drawRoundRect(color, size = size.copy(width = maxOf(6.dp.toPx(), size.width * f)), cornerRadius = r)
        }
    }
}

/** A line through numbers from 0 to `max`, shaded under; a dashed line when there are none. */
@Composable
private fun Spark(node: WidgetNode) {
    val p = LocalPalette.current
    val color = toned(node.tone, p.accent)
    val values = node.values.orEmpty()
    Canvas((node.w?.let { Modifier.width(it.dp) } ?: Modifier.fillMaxWidth()).height((node.h ?: 28.0).dp)) {
        if (values.isEmpty()) {
            drawLine(p.secondary.copy(alpha = 0.3f), Offset(4f, size.height / 2), Offset(size.width - 4, size.height / 2), 1.5.dp.toPx(),
                pathEffect = androidx.compose.ui.graphics.PathEffect.dashPathEffect(floatArrayOf(2.dp.toPx(), 3.dp.toPx())))
            return@Canvas
        }
        val data = if (values.size == 1) listOf(values[0], values[0]) else values
        val top = node.max ?: maxOf(1.0, data.max())
        val points = data.mapIndexed { i, v -> Offset(i.toFloat() / (data.size - 1) * size.width, size.height - 1 - (v.coerceIn(0.0, top) / top).toFloat() * (size.height - 2)) }
        val line = Path().apply { points.forEachIndexed { i, o -> if (i == 0) moveTo(o.x, o.y) else lineTo(o.x, o.y) } }
        val area = Path().apply { addPath(line); lineTo(size.width, size.height); lineTo(0f, size.height); close() }
        drawPath(area, color.copy(alpha = 0.2f))
        drawPath(line, color, style = Stroke(1.4.dp.toPx(), join = StrokeJoin.Round))
    }
}

@Composable
private fun Chart(node: WidgetNode) {
    val p = LocalPalette.current
    val color = toned(node.tone, p.accent)
    val values = node.values.orEmpty()
    val labels = node.labels.orEmpty()
    Column(Modifier.fillMaxWidth()) {
        Canvas(Modifier.fillMaxWidth().height(((node.h ?: 80.0) - if (labels.isEmpty()) 0.0 else 16.0).dp)) {
            if (values.isEmpty()) return@Canvas
            val top = maxOf(values.max(), 0.0001)
            val slot = size.width / values.size
            if (node.kind == "line") {
                val line = Path().apply {
                    values.forEachIndexed { i, v -> val o = Offset(slot * (i + 0.5f), size.height - (v / top).toFloat() * size.height); if (i == 0) moveTo(o.x, o.y) else lineTo(o.x, o.y) }
                }
                drawPath(line, color, style = Stroke(2.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round))
            } else {
                val w = slot * 0.6f
                values.forEachIndexed { i, v ->
                    val h = maxOf(1f, (v / top).toFloat() * size.height)
                    drawRoundRect(color, Offset(slot * i + (slot - w) / 2, size.height - h), androidx.compose.ui.geometry.Size(w, h), androidx.compose.ui.geometry.CornerRadius(2.dp.toPx()))
                }
            }
        }
        if (labels.isNotEmpty()) Row(Modifier.fillMaxWidth().padding(top = 2.dp)) {
            values.indices.forEach { i -> Text(labels.getOrElse(i) { "" }, Modifier.weight(1f), style = Type.caption2, color = p.secondary, textAlign = TextAlign.Center, maxLines = 1) }
        }
    }
}

@Composable
private fun WidgetList(node: WidgetNode) {
    val p = LocalPalette.current
    val items = node.c.orEmpty()
    if (items.isEmpty()) {
        Text(node.empty ?: "Nothing here", color = p.secondary)
        return
    }
    Column {
        items.forEachIndexed { i, item ->
            if (i > 0) HorizontalDivider(Modifier.padding(start = if (item.icon == null) 0.dp else 30.dp), color = p.separator, thickness = 0.5.dp)
            WidgetTree(item, modifier = Modifier.padding(vertical = 6.dp))
        }
    }
}

@Composable
private fun WidgetItem(node: WidgetNode) {
    val p = LocalPalette.current
    val row: @Composable () -> Unit = {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            node.icon?.let { Symbol(it, toned(node.tone, p.accent), 18.dp, Modifier.width(20.dp)) }
            Column(Modifier.weight(1f)) {
                Text(node.title ?: "", maxLines = 1, overflow = TextOverflow.Ellipsis, color = p.label)
                node.subtitle?.let { Text(it, style = Type.caption, color = p.secondary, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
            if (node.at != null) {
                Text(ago(node.at.toLong(), LocalNow.current), style = Type.footnote, color = p.secondary, maxLines = 1)
            } else node.trailing?.let { Text(it, style = Type.callout, color = p.secondary, maxLines = 1) }
        }
    }
    val link = node.link
    if (link != null) Linked(link) { row() } else row()
}

@Composable
private fun WidgetImage(node: WidgetNode) {
    val p = LocalPalette.current
    val host = LocalWidgetHost.current
    val url = node.url
    val image by produceState<android.graphics.Bitmap?>(null, url) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val bytes = when {
                    url == null -> null
                    url.startsWith("data:") -> android.util.Base64.decode(url.substringAfter(','), android.util.Base64.DEFAULT)
                    url.startsWith("http") -> java.net.URL(url).openStream().use { it.readBytes() }
                    else -> host.load?.invoke(url)
                }
                bytes?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
            }.getOrNull()
        }
    }
    val w = node.w?.dp
    val shape = RoundedCornerShape(if (node.round == true) 999.dp else 8.dp)
    val m = (if (w != null) Modifier.width(w) else Modifier.fillMaxWidth()).height(node.h?.dp ?: w ?: 120.dp).clip(shape).background(p.fill)
    image?.let { Image(it.asImageBitmap(), null, m, contentScale = ContentScale.Crop) } ?: Box(m)
}

/** An icon, `w` across or as big as its text: named, or an SVG path outlined; with its badge at its bottom corner. */
@Composable
private fun WidgetIcon(node: WidgetNode) {
    val p = LocalPalette.current
    val color = toned(node.tone, if (node.path == null) p.accent else p.label)
    val size = node.w?.dp ?: if (node.path != null) 22.dp else with(LocalDensity.current) { widgetStyle(node.size).fontSize.let { if (it.isSpecified) it.toDp() else 17.dp } }
    val path = node.path
    if (path == null && node.badge == null) return Symbol(node.name ?: "", color, size)
    Box(Modifier.size(size)) {
        if (path != null) SvgPath(path, color, size, stroke = true) else Symbol(node.name ?: "", color, size)
        node.badge?.let { Box(Modifier.align(Alignment.BottomEnd).size(size * 11 / 24).background(p.card, CircleShape)) { SvgPath(it, color, size * 11 / 24, stroke = false) } }
    }
}

/** An SVG path on a 24-point grid, outlined or filled, `size` across. */
@Composable
fun SvgPath(d: String, color: Color, size: Dp, stroke: Boolean) {
    val path = remember(d) { runCatching { PathParser.createPathFromPathData(d).asComposePath() }.getOrNull() }
    Canvas(Modifier.size(size)) {
        if (path == null) return@Canvas
        scale(this.size.width / 24f, pivot = Offset.Zero) {
            if (stroke) drawPath(path, color, style = Stroke(1.5f, join = StrokeJoin.Round)) else drawPath(path, color)
        }
    }
}

/** A Phosphor icon by name, filled in `color`; a dot if there's none by that name. */
@Composable
fun Symbol(name: String, color: Color, size: Dp, modifier: Modifier = Modifier) {
    Glyphs.load(LocalContext.current)
    val path = remember(name) { Glyphs.path(name)?.asComposePath() }
    if (path != null) {
        Canvas(modifier.size(size)) {
            scale(this.size.width / 256f, pivot = Offset.Zero) { drawPath(path, color) }
        }
    } else {
        Box(modifier.size(size), contentAlignment = Alignment.Center) { Box(Modifier.size(size / 3).background(color, CircleShape)) }
    }
}

/** A node that follows its link when tapped. */
@Composable
private fun Linked(link: JsonElement, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val host = LocalWidgetHost.current
    val open = host.open
    Box(if (open != null) modifier.clickable(remember { MutableInteractionSource() }, null) { open(link) } else modifier) { content() }
}

@Composable
private fun ActionButton(node: WidgetNode) {
    val p = LocalPalette.current
    val host = LocalWidgetHost.current
    val scope = rememberCoroutineScope()
    var running by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val tint = toned(node.tone, p.accent)
    val enabled = host.run != null && node.op != null
    Row(
        Modifier.clip(CircleShape).background(tint.copy(alpha = 0.15f)).clickable(enabled = enabled && !running) {
            running = true
            scope.launch {
                error = host.run?.invoke(node.op!!, node.input ?: JsonObject(emptyMap()))
                running = false
            }
        }.heightIn(min = 30.dp).padding(PaddingValues(horizontal = 12.dp, vertical = 5.dp)),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        if (running) CircularProgressIndicator(Modifier.size(14.dp), color = tint, strokeWidth = 2.dp)
        else node.icon?.let { Symbol(it, tint, 15.dp) }
        Text(node.label ?: node.text ?: "", style = Type.subheadline, fontWeight = FontWeight.Medium, color = tint, maxLines = 1)
    }
    error?.let { Problem(it) { error = null } }
}

@Composable
private fun ActionToggle(node: WidgetNode) {
    val p = LocalPalette.current
    val host = LocalWidgetHost.current
    val scope = rememberCoroutineScope()
    var pending by remember(node.on) { mutableStateOf<Boolean?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(node.label ?: "", Modifier.weight(1f), maxLines = 1, color = p.label)
        Switch(
            checked = pending ?: node.on ?: false,
            enabled = host.run != null && node.op != null,
            colors = SwitchDefaults.colors(checkedTrackColor = p.accent, checkedBorderColor = p.accent),
            onCheckedChange = { value ->
                pending = value
                scope.launch {
                    val input = (node.input as? JsonObject ?: JsonObject(emptyMap())).let { JsonObject(it + ("on" to JsonPrimitive(value))) }
                    error = host.run?.invoke(node.op!!, input)
                    if (error != null) pending = null
                }
            },
        )
    }
    error?.let { Problem(it) { error = null } }
}

/** "Couldn't Do That", with what went wrong. */
@Composable
fun Problem(text: String, title: String = "Couldn’t Do That", dismiss: () -> Unit) {
    AlertDialog(onDismissRequest = dismiss, confirmButton = { TextButton(dismiss) { Text("OK") } }, title = { Text(title) }, text = { Text(text) })
}
