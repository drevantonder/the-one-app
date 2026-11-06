package app.client

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.Typeface
import android.util.Base64
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asComposePath
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.graphics.PathParser
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive

// What the app draws itself, the home-screen widgets and where one is chosen: the web client's look (@one/ui's neutral
// theme): content first, weight and size for order, colour only for app icons and for warn and danger.

/** @one/ui's neutral colours, light then dark. */
object Kit {
    val bg = 0xffffff to 0x0a0a0a
    val card = 0xf5f5f5 to 0x171717
    val fill = 0xe5e5e5 to 0x262626
    val line = 0xe5e5e5 to 0x262626
    val text = 0x0a0a0a to 0xfafafa
    val muted = 0x737373 to 0xa1a1a1
    val faint = 0xa3a3a3 to 0x525252
    val warn = 0xb45309 to 0xfbbf24
    val danger = 0xdc2626 to 0xf87171
}

private fun Pair<Int, Int>.of(dark: Boolean) = Color(0xFF000000.toInt() or (if (dark) second else first))

/** The system colours a page is drawn in, light or dark, as the iPhone's are. */
@Immutable
data class Palette(
    val dark: Boolean,
    val background: Color,
    val card: Color,
    val label: Color,
    val secondary: Color,
    val tertiary: Color,
    val separator: Color,
    val fill: Color,
) {
    /** Nothing says "tap this" by colour: what's an accent is as strong as text. */
    val tint = label
    val orange = Kit.warn.of(dark)
    val red = Kit.danger.of(dark)
}

val LocalPalette = staticCompositionLocalOf { palette(false) }

fun palette(dark: Boolean) = Palette(
    dark = dark, background = Kit.bg.of(dark), card = Kit.card.of(dark), label = Kit.text.of(dark), secondary = Kit.muted.of(dark),
    tertiary = Kit.faint.of(dark), separator = Kit.line.of(dark), fill = Kit.fill.of(dark),
)

/** A colour from `#rrggbb`. */
fun hexColor(hex: String): Color {
    val v = hex.trimStart('#').toLongOrNull(16) ?: 0x808080
    return Color(((v shr 16) and 0xFF).toInt(), ((v shr 8) and 0xFF).toInt(), (v and 0xFF).toInt())
}

/** Text styles, in Android's units. */
object Type {
    val largeTitle = TextStyle(fontSize = 34.sp, fontWeight = FontWeight.Bold, lineHeight = 41.sp)
    val title2 = TextStyle(fontSize = 22.sp, fontWeight = FontWeight.SemiBold, lineHeight = 28.sp)
    val headline = TextStyle(fontSize = 17.sp, fontWeight = FontWeight.SemiBold, lineHeight = 22.sp)
    val body = TextStyle(fontSize = 17.sp, lineHeight = 22.sp)
    val callout = TextStyle(fontSize = 16.sp, lineHeight = 21.sp)
    val subheadline = TextStyle(fontSize = 15.sp, lineHeight = 20.sp)
    val footnote = TextStyle(fontSize = 13.sp, lineHeight = 18.sp)
    val caption = TextStyle(fontSize = 12.sp, lineHeight = 16.sp)
    val caption2 = TextStyle(fontSize = 11.sp, lineHeight = 13.sp)
}

/** The phone's own light or dark. */
@Composable
fun Theme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val p = palette(dark)
    val base = if (dark) darkColorScheme() else lightColorScheme()
    val scheme = base.copy(
        primary = p.tint, onPrimary = p.background, secondary = p.tint, background = p.background, onBackground = p.label,
        surface = p.card, onSurface = p.label, onSurfaceVariant = p.secondary, surfaceVariant = p.fill, surfaceContainer = p.card,
        surfaceContainerLow = p.background, surfaceContainerHigh = p.card, surfaceContainerHighest = p.fill, surfaceTint = Color.Transparent,
        outline = p.separator, outlineVariant = p.separator, error = p.red,
    )
    CompositionLocalProvider(LocalPalette provides p) {
        MaterialTheme(colorScheme = scheme, typography = MaterialTheme.typography.copy(bodyLarge = Type.body, bodyMedium = Type.subheadline), content = content)
    }
}

// MARK: Symbols

/** Phosphor's icons (vendor/phosphor), which apps name for their tiles, screens and widgets, so every device draws the
 *  same ones: each a path on a 256-point grid, drawn filled. `git-branch` is the regular one, `git-branch-fill` filled.
 *  The app keeps those the widgets draw beside them (`widgets/icons.json`, src/widgets.rs). */
object Glyphs {
    /** Read on a thread of its own (1 MB of JSON): until then icons draw as nothing, and once it is, they draw again. */
    private var all by mutableStateOf<Map<String, String>>(emptyMap())
    private val paths = LruCache<String, Path>(200)

    /** Reads them, again: for what draws off the main thread. */
    @Synchronized
    fun load(context: Context) {
        val file = java.io.File(context.filesDir, "widgets/icons.json")
        val text = runCatching { file.readText() }.getOrNull() ?: return
        all = (json.parseToJsonElement(text) as JsonObject).mapValues { it.value.jsonPrimitive.content }
        paths.evictAll()
    }

    /** Reads them in the background, for what's drawn on the main thread. */
    fun loadSoon(context: Context) {
        Thread { runCatching { load(context.applicationContext) } }.start()
    }

    fun has(name: String?) = name != null && all.containsKey(name)

    val ready get() = all.isNotEmpty()

    /** An icon's path data, as screens draw it. */
    fun data(name: String) = all[name]

    fun path(name: String): Path? = paths.get(name)
        ?: all[name]?.let { runCatching { PathParser.createPathFromPathData(it) }.getOrNull() }?.also { paths.put(name, it) }

    /** Draws an icon `size` pixels across at (x, y). */
    fun draw(canvas: Canvas, name: String, x: Float, y: Float, size: Float, color: Int) {
        val path = path(name) ?: return
        canvas.save()
        canvas.translate(x, y)
        canvas.scale(size / 256f, size / 256f)
        canvas.drawPath(path, Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL; this.color = color })
        canvas.restore()
    }
}

// MARK: App icons

/** What an app's tile shows: its icon on its colour, or its own picture. */
data class TileLook(val icon: String?, val seed: String, val color: String? = null, val image: String? = null, val imageKind: String? = null)

private val colors = listOf("#007aff", "#34c759", "#ff9500", "#ff3b30", "#af52de", "#5856d6", "#30b0c7", "#ff2d55", "#a2845e", "#32ade6", "#00c7be", "#8e8e93")

/** A steady number for a name, the same on every device. */
fun steadyHash(text: String): Int = text.codePoints().toArray().fold(5381) { h, c -> (h * 33 + c) and 0x7FFF_FFFF }

private val tiles = LruCache<String, Bitmap>(80)

/** An app's icon tile as the iPhone's Home Screen draws one, `px` pixels across: a finished app icon edge to edge, a
 *  logo inset on white, or its icon filled (or first letter) in white on its colour. */
fun tileBitmap(look: TileLook, px: Int, title: String = look.seed): Bitmap {
    val key = "${look.icon}|${look.seed}|${look.color}|${look.image?.length}|${look.image?.hashCode()}|${look.imageKind}|$px|$title|${Glyphs.ready}"
    tiles.get(key)?.let { return it }
    val bitmap = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(bitmap)
    val s = px.toFloat()
    val shape = Path().apply { addRoundRect(RectF(0f, 0f, s, s), s * 0.225f, s * 0.225f, Path.Direction.CW) }
    val picture = look.image?.let(::pictureOf)
    val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)
    canvas.save()
    canvas.clipPath(shape)
    if (picture != null && look.imageKind == "icon") {
        canvas.drawBitmap(picture, null, RectF(0f, 0f, s, s), paint)
    } else if (picture != null) {
        canvas.drawColor(android.graphics.Color.WHITE)
        val clear = clearCorners(picture)
        val inset = if (clear) s * 0.16f else 0f
        val fit = minOf((s - 2 * inset) / picture.width, (s - 2 * inset) / picture.height)
        val (w, h) = if (clear) picture.width * fit to picture.height * fit else s to s
        canvas.drawBitmap(picture, null, RectF((s - w) / 2, (s - h) / 2, (s + w) / 2, (s + h) / 2), paint)
    } else {
        val base = hexColor(look.color ?: colors[steadyHash(look.seed) % colors.size])
        val top = Color(minOf(1f, base.red * 1.12f + 0.04f), minOf(1f, base.green * 1.12f + 0.04f), minOf(1f, base.blue * 1.12f + 0.04f))
        paint.shader = LinearGradient(0f, 0f, 0f, s, top.toArgb(), base.toArgb(), Shader.TileMode.CLAMP)
        canvas.drawRect(0f, 0f, s, s, paint)
        paint.shader = null
        if (Glyphs.has("${look.icon}-fill")) {
            val g = s * 0.5f
            Glyphs.draw(canvas, "${look.icon}-fill", (s - g) / 2, (s - g) / 2, g, android.graphics.Color.WHITE)
        } else {
            val text = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = android.graphics.Color.WHITE; textSize = s * 0.46f; typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD); textAlign = Paint.Align.CENTER }
            canvas.drawText(title.take(1).uppercase(), s / 2, s / 2 - (text.descent() + text.ascent()) / 2, text)
        }
    }
    canvas.restore()
    tiles.put(key, bitmap)
    return bitmap
}

private val pictures = LruCache<String, Bitmap>(40)

/** An app's picture, from the data URL its node gives. */
private fun pictureOf(url: String): Bitmap? {
    pictures.get(url)?.let { return it }
    val bytes = runCatching { Base64.decode(url.substringAfter(','), Base64.DEFAULT) }.getOrNull() ?: return null
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.also { pictures.put(url, it) }
}

/** Whether any of a picture's corners is see-through. */
private fun clearCorners(b: Bitmap): Boolean =
    listOf(0 to 0, b.width - 1 to 0, 0 to b.height - 1, b.width - 1 to b.height - 1).any { (x, y) -> android.graphics.Color.alpha(b.getPixel(x, y)) < 128 }

/** An app's icon tile, with the Home Screen's soft shadow. */
@Composable
fun AppTile(look: TileLook, size: Dp, title: String = look.seed) {
    val px = with(LocalDensity.current) { size.roundToPx() }
    val bitmap = remember(look, px, title, Glyphs.ready) { tileBitmap(look, px, title).asImageBitmap() }
    Image(bitmap, contentDescription = null, modifier = Modifier.size(size).shadow(size * 0.06f, RoundedCornerShape(size * 0.225f), clip = false))
}

/** A Phosphor icon by name, filled in `color`; a dot if there's none by that name, and nothing until they're read. */
@Composable
fun Symbol(name: String, color: Color, size: Dp, modifier: Modifier = Modifier) {
    val ready = Glyphs.has(name)
    val path = remember(name, ready) { Glyphs.path(name)?.asComposePath() }
    if (path != null) {
        androidx.compose.foundation.Canvas(modifier.size(size)) {
            scale(this.size.width / 256f, pivot = Offset.Zero) { drawPath(path, color) }
        }
    } else if (Glyphs.ready) {
        Box(modifier.size(size), contentAlignment = Alignment.Center) { Box(Modifier.size(size / 3).background(color, CircleShape)) }
    } else Spacer(modifier.size(size))
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
