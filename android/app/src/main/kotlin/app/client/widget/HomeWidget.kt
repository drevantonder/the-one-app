package app.client.widget

import android.appwidget.AppWidgetManager
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.text.format.DateUtils
import android.widget.Toast
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.ColorFilter
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.LocalContext
import androidx.glance.LocalSize
import androidx.glance.action.Action
import androidx.glance.action.ActionParameters
import androidx.glance.action.actionParametersOf
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.appWidgetBackground
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.ColumnScope
import androidx.glance.layout.ContentScale
import androidx.glance.layout.Row
import androidx.glance.layout.RowScope
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextAlign
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider
import app.client.BuildConfig
import app.client.ClientApp
import app.client.Glyphs
import app.client.MainActivity
import app.client.Palette
import app.client.WidgetNode
import app.client.json
import app.client.number
import app.client.palette
import app.client.text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

// A widget from your pages on the home screen, the one chosen (the first, until one is), drawn at the size it's placed
// at as the app's pages draw it: text and layout as the home screen's own views, and what they can't draw as shapes.

private val SMALL = DpSize(110.dp, 110.dp)
private val MEDIUM = DpSize(250.dp, 110.dp)
private val LARGE = DpSize(250.dp, 250.dp)

class HomeWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(SMALL, MEDIUM, LARGE))

    override suspend fun provideGlance(context: Context, id: GlanceId) {
        HomeWidgets.start(context)
        withContext(Dispatchers.IO) { Glyphs.load(context) }
        val widgetId = GlanceAppWidgetManager(context).getAppWidgetId(id)
        provideContent {
            val shown by HomeWidgets.shown.collectAsState()
            Body(widgetId, shown)
        }
    }
}

class HomeWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = HomeWidget()

    override fun onUpdate(context: Context, appWidgetManager: AppWidgetManager, appWidgetIds: IntArray) {
        HomeWidgets.start(context)
        HomeWidgets.schedule(context)
        super.onUpdate(context, appWidgetManager, appWidgetIds)
    }

    override fun onDeleted(context: Context, appWidgetIds: IntArray) {
        super.onDeleted(context, appWidgetIds)
        HomeWidgets.start(context)
        HomeWidgets.forget(appWidgetIds)
    }

    override fun onDisabled(context: Context) {
        super.onDisabled(context)
        HomeWidgets.unschedule(context)
    }
}

private object Keys {
    val app = ActionParameters.Key<String>("app")
    val op = ActionParameters.Key<String>("op")
    val input = ActionParameters.Key<String>("input")
}

/** A widget's button or toggle: its operation runs, then every widget redraws. */
class RunOperation : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        HomeWidgets.start(context)
        val app = parameters[Keys.app] ?: return
        val op = parameters[Keys.op] ?: return
        val input = parameters[Keys.input]?.let { runCatching { json.parseToJsonElement(it) }.getOrNull() } ?: JsonObject(emptyMap())
        val store = (context.applicationContext as ClientApp).store
        try {
            store.call(app, op, input)
        } catch (e: Exception) {
            withContext(Dispatchers.Main) { Toast.makeText(context, "Couldn’t do that. ${e.message ?: ""}".trim(), Toast.LENGTH_LONG).show() }
            HomeWidgets.redraw()
        }
        runCatching {
            if (store.state == null) store.refreshState()
            store.refreshWidgets(always = true)
        }
    }
}

/** The colours of the app's pages, for light and dark, as the home screen picks them with the phone. */
private class Colors(accent: String) {
    val light = palette(false, accent)
    val dark = palette(true, accent)

    fun of(alpha: Float = 1f, pick: (Palette) -> Color): ColorProvider =
        androidx.glance.color.ColorProvider(day = pick(light).let { it.copy(alpha = it.alpha * alpha) }, night = pick(dark).let { it.copy(alpha = it.alpha * alpha) })

    /** A tone's colour: `muted` is secondary, and with no tone, `otherwise`. */
    fun tone(tone: String?, alpha: Float = 1f, otherwise: (Palette) -> Color): ColorProvider = of(alpha) { p ->
        when (tone) {
            "muted" -> p.secondary
            "accent" -> p.accent
            "good" -> p.green
            "warn" -> p.orange
            "bad" -> p.red
            "blue" -> p.blue
            "purple" -> p.purple
            else -> otherwise(p)
        }
    }
}

/** What a node is drawn with: the widget it's in, the width it has (in dp, roughly), and whether it's dimmed or in a row. */
private data class Ctx(
    val widget: KeptWidget,
    val colors: Colors,
    val masks: Masks,
    val width: Float,
    val now: Long,
    val inRow: Boolean = false,
    val alpha: Float = 1f,
) {
    fun tone(tone: String?, otherwise: (Palette) -> Color, alpha: Float = 1f) = colors.tone(tone, this.alpha * alpha, otherwise)
    fun color(alpha: Float = 1f, pick: (Palette) -> Color) = colors.of(this.alpha * alpha, pick)
}

private val appName get() = BuildConfig.NAME

private fun opening(uri: Uri): Action = actionStartActivity(Intent(Intent.ACTION_VIEW, uri).setPackage(BuildConfig.APPLICATION_ID))

/** A widget's link: a web address in the browser, `{ screen, params }` that screen of its app, on its page. */
private fun follow(w: KeptWidget, link: JsonElement?): Action {
    val text = (link as? JsonPrimitive)?.contentOrNull
    if (text != null && (text.startsWith("https://") || text.startsWith("http://"))) return actionStartActivity(Intent(Intent.ACTION_VIEW, Uri.parse(text)))
    val o = link as? JsonObject
    val screen = (o?.get("screen") as? JsonPrimitive)?.contentOrNull
    return opening(HomeWidgets.link(w, screen ?: "main".takeIf { w.opens }, o?.get("params")))
}

@Composable
private fun Body(widgetId: Int, shown: HomeWidgets.Shown) {
    val context = LocalContext.current
    val kept = shown.kept
    val colors = Colors(kept?.accent ?: BuildConfig.ACCENT)
    val chosen = shown.choices[widgetId]
    val widget = if (chosen == null) kept?.widgets?.firstOrNull() else kept?.widgets?.firstOrNull { it.id == chosen }
    val app = opening(Uri.Builder().scheme(BuildConfig.CLI).authority("open").build())
    Box(
        GlanceModifier.fillMaxSize().appWidgetBackground().background(colors.of { it.card })
            .cornerRadius(android.R.dimen.system_app_widget_background_radius).padding(16.dp),
    ) {
        when {
            !shown.signedIn -> Waiting(colors, "Sign in to $appName to see your pages.", app)
            widget == null && chosen != null && !kept?.widgets.isNullOrEmpty() ->
                Broken(colors, appName, "${HomeWidgets.title(widgetId) ?: "This widget"} isn’t on your pages any more. Touch and hold to choose another.", app)
            widget == null -> Waiting(colors, "Open $appName to see your pages.", app)
            else -> Drawn(context, widget, colors)
        }
    }
}

@Composable
private fun Drawn(context: Context, w: KeptWidget, colors: Colors) {
    val info = w.decoded()
    val error = info?.error
    if (info == null || error != null) {
        return Broken(colors, w.title, "This widget stopped working. Open $appName to see why.", follow(w, null))
    }
    val size = LocalSize.current
    val name = when {
        size.width >= LARGE.width && size.height >= LARGE.height -> "large"
        size.width >= MEDIUM.width -> "medium"
        else -> "small"
    }
    val tree = info.tree(name)
    // As wide as the launcher's cells at that size, less the padding.
    val width = if (name == "small") 140f else 320f
    val c = Ctx(w, colors, Masks(context.resources.displayMetrics.density), width, System.currentTimeMillis())
    Column(GlanceModifier.fillMaxSize().clickable(follow(w, tree.link))) {
        if (tree.header != false) {
            Text(w.title, style = TextStyle(color = colors.of { it.secondary }, fontSize = 11.sp, fontWeight = FontWeight.Medium), maxLines = 1)
            Spacer(GlanceModifier.height(6.dp))
        }
        // At the top, as the app's pages draw it; a column takes the height, for what grows in it.
        Box(GlanceModifier.fillMaxWidth().defaultWeight(), contentAlignment = Alignment.TopStart) {
            Node(tree, c, if (tree.t == "col") GlanceModifier.fillMaxSize() else GlanceModifier.fillMaxWidth())
        }
    }
}

/** A widget that can't be drawn, and what to do about it. */
@Composable
private fun Broken(colors: Colors, title: String, text: String, tap: Action) {
    val warn = colors.of { it.orange }
    Column(GlanceModifier.fillMaxSize().clickable(tap)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Image(ImageProvider(Masks(LocalContext.current.resources.displayMetrics.density).glyph("warning-fill", 14f)), null, GlanceModifier.size(14.dp), colorFilter = ColorFilter.tint(warn))
            Spacer(GlanceModifier.width(5.dp))
            Text(title, style = TextStyle(color = warn, fontSize = 12.sp, fontWeight = FontWeight.Medium), maxLines = 1)
        }
        Spacer(GlanceModifier.height(6.dp))
        Text(text, style = TextStyle(color = colors.of { it.secondary }, fontSize = 13.sp))
    }
}

/** Nothing to show yet: signed out, or the app hasn't shown its pages since it was signed in. */
@Composable
private fun Waiting(colors: Colors, text: String, tap: Action) {
    Column(GlanceModifier.fillMaxSize().clickable(tap), horizontalAlignment = Alignment.CenterHorizontally, verticalAlignment = Alignment.CenterVertically) {
        Image(ImageProvider(app.client.R.drawable.logo), null, GlanceModifier.size(44.dp).cornerRadius(10.dp))
        Spacer(GlanceModifier.height(8.dp))
        Text(text, style = TextStyle(color = colors.of { it.secondary }, fontSize = 12.sp, textAlign = TextAlign.Center))
    }
}

// MARK: The tree, as WidgetTree.kt draws it

/** The home screen's views take at most ten children: more go in columns (or rows) of their own, `most` to each (with
 *  the spacers between them, half of ten), as even as can be. */
private fun <T> List<T>.groups(most: Int = 10): List<List<T>> {
    if (size <= most) return listOf(this)
    val n = (size + most - 1) / most
    return chunked((size + n - 1) / n)
}

private fun fontSize(size: String?) = when (size) {
    "xl" -> 22f
    "lg" -> 17f
    "xs" -> 11f
    else -> 13f
}

private fun weight(w: String?) = when (w) {
    "bold" -> FontWeight.Bold
    "medium" -> FontWeight.Medium
    else -> FontWeight.Normal
}

private fun fraction(n: WidgetNode): Float {
    val max = n.max ?: 1.0
    return if (max > 0) ((n.value.number ?: 0.0) / max).coerceIn(0.0, 1.0).toFloat() else 0f
}

/** Roughly how wide a node is that doesn't grow, so what grows beside it knows what's left. */
private fun breadth(n: WidgetNode): Float = n.w?.toFloat() ?: when (n.t) {
    "gauge" -> 46f
    "icon" -> if (n.path != null) 22f else fontSize(n.size)
    "dot" -> 8f
    else -> 0f
}

@Composable
private fun Node(n: WidgetNode, ctx: Ctx, modifier: GlanceModifier = GlanceModifier) {
    val c = if (n.dim == true) ctx.copy(alpha = ctx.alpha * 0.5f) else ctx
    val link = n.link
    val m = if (link != null && n.t != "item" && n.t != "link") modifier.clickable(follow(c.widget, link)) else modifier
    val width = n.w?.toFloat()
    val sized = if (width != null) m.width(width.dp) else m
    // An empty column or row only takes up room, which the home screen leaves out unless it's a spacer.
    if ((n.t == "col" || n.t == "row") && n.c.isNullOrEmpty()) return Spacer(sized)
    when (n.t) {
        "col" -> Col(n, c.copy(width = width ?: c.width, inRow = false), sized)
        "row" -> Line(n, c.copy(width = width ?: c.width, inRow = true), sized)
        "text" -> Text(n.text ?: "", m, style = text(c, n.size, n.weight, n.tone), maxLines = n.lines ?: if (c.inRow) 1 else Int.MAX_VALUE)
        "time" -> Text(time(n, c.now), m, style = text(c, n.size, n.weight, n.tone), maxLines = 1)
        "stat" -> Stat(n, c, m)
        "gauge" -> Ring(n, c, m)
        "progress" -> Bar(n, c, m)
        "spark" -> Spark(n, c, m)
        "chart" -> Chart(n, c, m)
        "list" -> WidgetList(n, c, m)
        "item" -> Item(n, c, m)
        "image" -> Picture(n, c, m)
        "icon" -> Icon(n, c, m)
        "dot" -> Mask(c.masks.circle(8f), c.tone(n.tone, { it.green }), m.size(8.dp))
        "link" -> Text(n.text ?: "", if (link != null) m.clickable(follow(c.widget, link)) else m, style = text(c, n.size, null, "accent"), maxLines = n.lines ?: 1)
        "button" -> Button(n, c, m)
        "toggle" -> Toggle(n, c, m)
        "divider" -> Spacer(m.fillMaxWidth().height(0.5.dp).background(c.color { it.separator }))
        else -> {}
    }
}

private fun text(c: Ctx, size: String?, weight: String?, tone: String?) =
    TextStyle(color = c.tone(tone, { it.label }), fontSize = fontSize(size).sp, fontWeight = weight(weight))

@Composable
private fun Mask(bitmap: Bitmap, color: ColorProvider, modifier: GlanceModifier, scale: ContentScale = ContentScale.Fit) {
    Image(ImageProvider(bitmap), null, modifier, contentScale = scale, colorFilter = ColorFilter.tint(color))
}

@Composable
private fun Col(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val gap = (n.gap ?: 4.0).dp
    val align = if (n.align == "center") Alignment.CenterHorizontally else Alignment.Start
    val groups = n.c.orEmpty().groups(5)
    @Composable
    fun ColumnScope.Kids(kids: List<WidgetNode>) = kids.forEachIndexed { i, kid ->
        if (i > 0) Spacer(GlanceModifier.height(gap))
        Node(kid, c, if (kid.grow == true) GlanceModifier.defaultWeight() else GlanceModifier)
    }
    Column(modifier, horizontalAlignment = align) {
        if (groups.size == 1) Kids(groups[0])
        else groups.forEachIndexed { g, kids ->
            if (g > 0) Spacer(GlanceModifier.height(gap))
            Column(if (kids.any { it.grow == true }) GlanceModifier.defaultWeight() else GlanceModifier, horizontalAlignment = align) { Kids(kids) }
        }
    }
}

@Composable
private fun Line(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val gap = (n.gap ?: 6.0).toFloat()
    val all = n.c.orEmpty()
    // What grows takes what the rest leave.
    val growing = all.count { it.grow == true }
    val left = (c.width - all.filter { it.grow != true }.sumOf { breadth(it).toDouble() }.toFloat() - gap * (all.size - 1)).coerceAtLeast(24f)
    val growCtx = c.copy(width = if (growing > 0) left / growing else left)
    val groups = all.groups(5)
    @Composable
    fun RowScope.Kids(kids: List<WidgetNode>) = kids.forEachIndexed { i, kid ->
        if (i > 0) Spacer(GlanceModifier.width(gap.dp))
        Node(kid, if (kid.grow == true) growCtx else c.copy(width = breadth(kid).takeIf { it > 0 } ?: left), if (kid.grow == true) GlanceModifier.defaultWeight() else GlanceModifier)
    }
    // A row whose child grows is as wide as it can be, as it is in the app; the home screen's rows aren't by themselves.
    Row(if (growing > 0 && n.w == null) modifier.fillMaxWidth() else modifier, horizontalAlignment = if (n.align == "center") Alignment.CenterHorizontally else Alignment.Start, verticalAlignment = Alignment.CenterVertically) {
        if (groups.size == 1) Kids(groups[0])
        else groups.forEachIndexed { g, kids ->
            if (g > 0) Spacer(GlanceModifier.width(gap.dp))
            Row(if (kids.any { it.grow == true }) GlanceModifier.defaultWeight() else GlanceModifier, verticalAlignment = Alignment.CenterVertically) { Kids(kids) }
        }
    }
}

@Composable
private fun Stat(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    Column(modifier) {
        Row(verticalAlignment = Alignment.Bottom) {
            Text(n.value.text ?: "–", style = TextStyle(color = c.tone(n.tone, { it.label }), fontSize = 28.sp, fontWeight = FontWeight.Medium), maxLines = 1)
            n.unit?.let {
                Text(it, GlanceModifier.padding(start = 3.dp, bottom = 4.dp), style = TextStyle(color = c.color { p -> p.secondary }, fontSize = 15.sp, fontWeight = FontWeight.Medium), maxLines = 1)
            }
        }
        n.label?.let { Text(it, style = TextStyle(color = c.color { p -> p.secondary }, fontSize = 12.sp), maxLines = 1) }
    }
}

@Composable
private fun Ring(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val f = fraction(n)
    val size = (n.w ?: 46.0).toFloat()
    Box(modifier.size(size.dp), contentAlignment = Alignment.Center) {
        Mask(c.masks.ring(size, 1f, full = true), c.color(0.25f / 0.6f) { it.secondary }, GlanceModifier.size(size.dp))
        Mask(c.masks.ring(size, f, full = false), c.tone(n.tone, { it.accent }), GlanceModifier.size(size.dp))
        if (n.label != "") {
            Text(n.label ?: "${Math.round(f * 100)}%", style = TextStyle(color = c.color { it.label }, fontSize = 11.sp, fontWeight = FontWeight.Medium), maxLines = 1)
        }
    }
}

@Composable
private fun Bar(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val f = fraction(n)
    val width = (n.w?.toFloat() ?: c.width).coerceAtLeast(6f)
    val m = if (n.w != null) GlanceModifier.size(width.dp, 6.dp) else GlanceModifier.fillMaxWidth().height(6.dp)
    Column(modifier) {
        n.label?.let {
            Text(it, style = TextStyle(color = c.color { p -> p.secondary }, fontSize = 12.sp), maxLines = 1)
            Spacer(GlanceModifier.height(4.dp))
        }
        Box(m) {
            Mask(c.masks.bar(width, 6f, 1f), c.color(0.25f / 0.6f) { it.secondary }, m, ContentScale.FillBounds)
            Mask(c.masks.bar(width, 6f, f), c.tone(n.tone, { it.accent }), m, ContentScale.FillBounds)
        }
    }
}

@Composable
private fun Spark(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val values = n.values.orEmpty()
    val width = (n.w?.toFloat() ?: c.width).coerceAtLeast(8f)
    val height = (n.h ?: 28.0).toFloat()
    val m = if (n.w != null) GlanceModifier.size(width.dp, height.dp) else GlanceModifier.fillMaxWidth().height(height.dp)
    val color = c.tone(n.tone, { it.accent })
    Box(modifier.then(m)) {
        if (values.isEmpty()) {
            Mask(c.masks.spark(values, n.max, width, height, area = false), c.color { it.secondary }, m, ContentScale.FillBounds)
        } else {
            Mask(c.masks.spark(values, n.max, width, height, area = true), color, m, ContentScale.FillBounds)
            Mask(c.masks.spark(values, n.max, width, height, area = false), color, m, ContentScale.FillBounds)
        }
    }
}

@Composable
private fun Chart(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val values = n.values.orEmpty()
    val labels = n.labels.orEmpty()
    val height = ((n.h ?: 80.0) - if (labels.isEmpty()) 0.0 else 16.0).toFloat()
    Column(modifier.fillMaxWidth()) {
        if (values.isNotEmpty()) {
            Mask(c.masks.chart(values, n.kind == "line", c.width, height), c.tone(n.tone, { it.accent }), GlanceModifier.fillMaxWidth().height(height.dp), ContentScale.FillBounds)
        } else Spacer(GlanceModifier.height(height.dp))
        if (labels.isNotEmpty()) {
            val style = TextStyle(color = c.color { it.secondary }, fontSize = 11.sp, textAlign = TextAlign.Center)
            Row(GlanceModifier.fillMaxWidth().padding(top = 2.dp)) {
                for (group in values.indices.toList().groups()) {
                    Row(GlanceModifier.defaultWeight()) {
                        for (i in group) Text(labels.getOrElse(i) { "" }, GlanceModifier.defaultWeight(), style = style, maxLines = 1)
                    }
                }
            }
        }
    }
}

@Composable
private fun WidgetList(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val items = n.c.orEmpty()
    if (items.isEmpty()) {
        Text(n.empty ?: "Nothing here", modifier, style = TextStyle(color = c.color { it.secondary }, fontSize = 13.sp))
        return
    }
    Column(modifier.fillMaxWidth()) {
        for (group in items.withIndex().toList().groups()) {
            Column(GlanceModifier.fillMaxWidth()) {
                for ((i, item) in group) {
                    Column(GlanceModifier.fillMaxWidth()) {
                        if (i > 0) Spacer(GlanceModifier.fillMaxWidth().padding(start = if (item.icon == null) 0.dp else 30.dp).height(0.5.dp).background(c.color { it.separator }))
                        Node(item, c, GlanceModifier.fillMaxWidth().padding(vertical = 6.dp))
                    }
                }
            }
        }
    }
}

@Composable
private fun Item(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val link = n.link
    Row((if (link != null) modifier.clickable(follow(c.widget, link)) else modifier).fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        n.icon?.let {
            Mask(c.masks.glyph(it, 18f), c.tone(n.tone, { p -> p.accent }), GlanceModifier.size(18.dp))
            Spacer(GlanceModifier.width(12.dp))
        }
        Column(GlanceModifier.defaultWeight()) {
            Text(n.title ?: "", style = TextStyle(color = c.color { it.label }, fontSize = 13.sp), maxLines = 1)
            n.subtitle?.let { Text(it, style = TextStyle(color = c.color { p -> p.secondary }, fontSize = 12.sp), maxLines = 1) }
        }
        val trailing = n.at?.let { ago(it.toLong(), c.now) } ?: n.trailing
        trailing?.let {
            Spacer(GlanceModifier.width(10.dp))
            Text(it, style = TextStyle(color = c.color { p -> p.secondary }, fontSize = if (n.at != null) 13.sp else 15.sp), maxLines = 1)
        }
    }
}

@Composable
private fun Picture(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val url = n.url
    val bitmap = url?.let {
        val bytes = if (it.startsWith("data:")) HomeWidgets.dataImage(it) else HomeWidgets.imageFile(it).takeIf { f -> f.exists() }?.readBytes()
        bytes?.let { b -> decode(b, 600) }
    }
    val w = n.w?.toFloat()
    val h = (n.h ?: n.w ?: 120.0).toFloat()
    val m = (if (w != null) modifier.width(w.dp) else modifier.fillMaxWidth()).height(h.dp).cornerRadius(if (n.round == true) (h / 2).dp else 8.dp)
    if (bitmap != null) Image(ImageProvider(bitmap), null, m, contentScale = ContentScale.Crop)
    else Box(m.background(c.color { it.fill })) {}
}

/** An image no wider or taller than `most` pixels. */
private fun decode(bytes: ByteArray, most: Int): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / sample > most) sample *= 2
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
}

/** An icon, `w` across or as big as its text: named, or an SVG path outlined; with its badge at its bottom corner. */
@Composable
private fun Icon(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val path = n.path
    val size = n.w?.toFloat() ?: if (path != null) 22f else fontSize(n.size)
    val color = c.tone(n.tone, { if (path == null) it.accent else it.label })
    val shape = if (path != null) c.masks.svg(path, size, stroke = true) else c.masks.glyph(n.name ?: "", size)
    val badge = n.badge
    if (badge == null) return Mask(shape, color, modifier.size(size.dp))
    val small = size * 11 / 24
    Box(modifier.size(size.dp), contentAlignment = Alignment.BottomEnd) {
        Mask(shape, color, GlanceModifier.size(size.dp))
        Box(GlanceModifier.size(small.dp)) {
            Mask(c.masks.circle(small), c.color { it.card }, GlanceModifier.size(small.dp))
            Mask(c.masks.svg(badge, small, stroke = false), color, GlanceModifier.size(small.dp))
        }
    }
}

/** Runs a button's operation, or a toggle's with `on`, the value it's switched to, in its input. */
private fun run(n: WidgetNode, c: Ctx, on: Boolean? = null): Action {
    var input = n.input ?: JsonObject(emptyMap())
    if (on != null) input = JsonObject((input as? JsonObject ?: JsonObject(emptyMap())) + ("on" to JsonPrimitive(on)))
    return actionRunCallback<RunOperation>(actionParametersOf(Keys.app to c.widget.app, Keys.op to (n.op ?: ""), Keys.input to input.toString()))
}

@Composable
private fun Button(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val tint = c.tone(n.tone, { it.accent })
    val m = modifier.cornerRadius(15.dp).background(c.tone(n.tone, { it.accent }, 0.15f)).let { if (n.op != null) it.clickable(run(n, c)) else it }
    Row(m.height(30.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        n.icon?.let {
            Mask(c.masks.glyph(it, 15f), tint, GlanceModifier.size(15.dp))
            Spacer(GlanceModifier.width(5.dp))
        }
        Text(n.label ?: n.text ?: "", style = TextStyle(color = tint, fontSize = 15.sp, fontWeight = FontWeight.Medium), maxLines = 1)
    }
}

/** A toggle as the app's pages draw it, which runs its operation with the value it's switched to. */
@Composable
private fun Toggle(n: WidgetNode, c: Ctx, modifier: GlanceModifier) {
    val on = n.on ?: false
    val m = if (n.op != null) modifier.clickable(run(n, c, !on)) else modifier
    Row(m.fillMaxWidth().padding(vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(n.label ?: "", GlanceModifier.defaultWeight(), style = TextStyle(color = c.color { it.label }, fontSize = 13.sp), maxLines = 1)
        Box(GlanceModifier.size(40.dp, 24.dp), contentAlignment = if (on) Alignment.CenterEnd else Alignment.CenterStart) {
            Mask(c.masks.bar(40f, 24f, 1f), if (on) c.color { it.accent } else c.color { if (it.dark) Color(0xFF39393D) else Color(0xFFDCDCE1) }, GlanceModifier.size(40.dp, 24.dp))
            Box(GlanceModifier.padding(horizontal = 2.dp)) {
                Mask(c.masks.circle(20f), c.color { Color.White }, GlanceModifier.size(20.dp))
            }
        }
    }
}

/** "now", "5 min", "3 h", then the weekday, then the date; or as the node's `style` asks, as src/screen/widgets.js does. */
private fun time(node: WidgetNode, now: Long): String {
    val at = (node.at ?: 0.0).toLong()
    return when (node.style) {
        "relative" -> DateUtils.getRelativeTimeSpanString(at, now, DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_RELATIVE).toString()
        "timer" -> DateUtils.formatElapsedTime(kotlin.math.abs(now - at) / 1000)
        "date" -> DateUtils.formatDateTime(null, at, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_NO_YEAR or DateUtils.FORMAT_ABBREV_MONTH)
        "clock" -> java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(at))
        else -> ago(at, now)
    }
}

private fun ago(at: Long, now: Long): String {
    val s = (now - at) / 1000
    return when {
        s < 60 -> "now"
        s < 3600 -> "${s / 60} min"
        s < 86400 -> "${s / 3600} h"
        s < 7 * 86400 -> android.text.format.DateFormat.format("EEE", at).toString()
        else -> DateUtils.formatDateTime(null, at, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_NO_YEAR or DateUtils.FORMAT_ABBREV_MONTH)
    }
}
