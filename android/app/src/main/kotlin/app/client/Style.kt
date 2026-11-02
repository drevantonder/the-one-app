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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Call
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.DateRange
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Email
import androidx.compose.material.icons.rounded.Favorite
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Send
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Star
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.vector.ImageVector
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

// The Apple apps' calm look, on Android: their grouped greys and white cards, their type sizes, the install's accent.

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
    val accent: Color,
) {
    val green = if (dark) Color(0xFF30D158) else Color(0xFF34C759)
    val orange = if (dark) Color(0xFFFF9F0A) else Color(0xFFFF9500)
    val red = if (dark) Color(0xFFFF453A) else Color(0xFFFF3B30)
    val blue = Color(0xFF0A9EDC)
    val purple = Color(0xFFA550D9)
}

val LocalPalette = staticCompositionLocalOf { palette(false, BuildConfig.ACCENT) }

fun palette(dark: Boolean, accent: String) = if (dark) Palette(
    dark = true, background = Color.Black, card = Color(0xFF1C1C1E), label = Color.White, secondary = Color(0x99EBEBF5),
    tertiary = Color(0x4DEBEBF5), separator = Color(0x5C545458), fill = Color(0x3D767680), accent = accentColor(accent, true),
) else Palette(
    dark = false, background = Color(0xFFF2F2F7), card = Color.White, label = Color.Black, secondary = Color(0x993C3C43),
    tertiary = Color(0x4D3C3C43), separator = Color(0x4A3C3C43), fill = Color(0x1F787880), accent = accentColor(accent, false),
)

/** An accent from hex, lifted a little in dark mode so it keeps its contrast there, yet still carries white text. */
fun accentColor(hex: String, dark: Boolean): Color {
    val v = hex.trimStart('#').toLongOrNull(16) ?: 0x808080
    val rgb = listOf(16, 8, 0).map { ((v shr it) and 0xFF) / 255f }.map { if (dark) it + (1 - it) * 0.2f else it }
    return Color(rgb[0], rgb[1], rgb[2])
}

/** The iPhone's text styles, in Android's units. */
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

/** Light, Dark or Automatic, as chosen in Settings on this phone. */
enum class Appearance(val title: String) { Automatic("Automatic"), Light("Light"), Dark("Dark") }

@Composable
fun Theme(accent: String, appearance: Appearance, content: @Composable () -> Unit) {
    val dark = when (appearance) {
        Appearance.Automatic -> isSystemInDarkTheme()
        Appearance.Light -> false
        Appearance.Dark -> true
    }
    val p = palette(dark, accent)
    val base = if (dark) darkColorScheme() else lightColorScheme()
    val scheme = base.copy(
        primary = p.accent, onPrimary = Color.White, secondary = p.accent, background = p.background, onBackground = p.label,
        surface = p.card, onSurface = p.label, onSurfaceVariant = p.secondary, surfaceVariant = p.fill, surfaceContainer = p.card,
        surfaceContainerLow = p.background, surfaceContainerHigh = p.card, surfaceContainerHighest = p.fill, surfaceTint = Color.Transparent,
        outline = p.separator, outlineVariant = p.separator, error = p.red, primaryContainer = p.accent.copy(alpha = 0.16f), onPrimaryContainer = p.accent,
    )
    CompositionLocalProvider(LocalPalette provides p) {
        MaterialTheme(colorScheme = scheme, typography = MaterialTheme.typography.copy(bodyLarge = Type.body, bodyMedium = Type.subheadline), content = content)
    }
}

// MARK: Symbols

/** The symbols app icons are drawn with: the screens' bridge's own (src/screen/bridge.js), so every place draws the
 *  same ones, each on a 24-point grid. */
object Glyphs {
    private var all: Map<String, String> = emptyMap()

    fun load(context: Context) {
        if (all.isNotEmpty()) return
        val text = context.assets.open("glyphs.json").bufferedReader().readText()
        all = (json.parseToJsonElement(text) as JsonObject).mapValues { it.value.jsonPrimitive.content }
    }

    fun has(name: String?) = name != null && all.containsKey(name)

    /** A glyph's shapes: each a path, and whether it's filled rather than outlined. */
    fun shapes(name: String): List<Pair<Path, Boolean>> {
        val svg = all[name] ?: return emptyList()
        return Regex("""<(rect|circle|path)([^>]*)/>""").findAll(svg).mapNotNull { m ->
            val a = Regex("""([\w-]+)="([^"]*)"""").findAll(m.groupValues[2]).associate { it.groupValues[1] to it.groupValues[2] }
            fun n(k: String) = a[k]?.toFloatOrNull() ?: 0f
            val path = when (m.groupValues[1]) {
                "rect" -> Path().apply { addRoundRect(RectF(n("x"), n("y"), n("x") + n("width"), n("y") + n("height")), n("rx"), n("rx"), Path.Direction.CW) }
                "circle" -> Path().apply { addCircle(n("cx"), n("cy"), n("r"), Path.Direction.CW) }
                else -> runCatching { PathParser.createPathFromPathData(a["d"] ?: "") }.getOrNull() ?: return@mapNotNull null
            }
            path to (a["fill"] == "currentColor")
        }.toList()
    }

    /** Draws a glyph `size` pixels across at (x, y). */
    fun draw(canvas: Canvas, name: String, x: Float, y: Float, size: Float, color: Int, weight: Float = 1.9f) {
        val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE; strokeWidth = weight; strokeCap = Paint.Cap.ROUND; strokeJoin = Paint.Join.ROUND; this.color = color
        }
        val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL; this.color = color }
        canvas.save()
        canvas.translate(x, y)
        canvas.scale(size / 24f, size / 24f)
        for ((path, filled) in shapes(name)) canvas.drawPath(path, if (filled) fill else stroke)
        canvas.restore()
    }
}

/** A widget's or a row's symbol, by the SF Symbol name apps give: the glyph if there is one, else Android's nearest. */
fun materialSymbol(name: String?): ImageVector? {
    val n = (name ?: return null).removeSuffix(".fill").removeSuffix(".circle")
    return when {
        name.startsWith("checkmark.circle") -> Icons.Rounded.CheckCircle
        n.startsWith("checkmark") -> Icons.Rounded.Check
        n.startsWith("exclamationmark") -> Icons.Rounded.Warning
        n.startsWith("info") -> Icons.Rounded.Info
        n.startsWith("plus") -> Icons.Rounded.Add
        n.startsWith("xmark") -> Icons.Rounded.Close
        n.startsWith("trash") -> Icons.Rounded.Delete
        n.startsWith("gear") -> Icons.Rounded.Settings
        n.startsWith("person") -> Icons.Rounded.Person
        n.startsWith("envelope") -> Icons.Rounded.Email
        n.startsWith("phone") -> Icons.Rounded.Call
        n.startsWith("star") -> Icons.Rounded.Star
        n.startsWith("house") -> Icons.Rounded.Home
        n.startsWith("magnifyingglass") || n == "search" -> Icons.Rounded.Search
        n.startsWith("arrow.clockwise") || n.startsWith("arrow.triangle.2") -> Icons.Rounded.Refresh
        n.startsWith("lock") -> Icons.Rounded.Lock
        n.startsWith("bell") -> Icons.Rounded.Notifications
        n.startsWith("heart") -> Icons.Rounded.Favorite
        n.startsWith("calendar") || n.startsWith("clock") -> Icons.Rounded.DateRange
        n.startsWith("play") -> Icons.Rounded.PlayArrow
        n.startsWith("paperplane") -> Icons.Rounded.Send
        else -> null
    }
}

// MARK: App icons

/** What an app's tile shows: its symbol on its colour, or its own picture. */
data class TileLook(val icon: String?, val seed: String, val color: String? = null, val image: String? = null, val imageKind: String? = null)

fun AppInfo.look() = TileLook(icon, name, color, image, imageKind)

private val colors = listOf("#007aff", "#34c759", "#ff9500", "#ff3b30", "#af52de", "#5856d6", "#30b0c7", "#ff2d55", "#a2845e", "#32ade6", "#00c7be", "#8e8e93")

/** A steady number for a name, the same on every device. */
fun steadyHash(text: String): Int = text.codePoints().toArray().fold(5381) { h, c -> (h * 33 + c) and 0x7FFF_FFFF }

private val tiles = LruCache<String, Bitmap>(80)

/** An app's icon tile as the iPhone's Home Screen draws one, `px` pixels across: a finished app icon edge to edge, a
 *  logo inset on white, or its symbol (or first letter) in white on its colour. */
fun tileBitmap(look: TileLook, px: Int, title: String = look.seed): Bitmap {
    val key = "${look.icon}|${look.seed}|${look.color}|${look.image?.length}|${look.image?.hashCode()}|${look.imageKind}|$px|$title"
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
        val base = accentColor(look.color ?: colors[steadyHash(look.seed) % colors.size], false)
        val top = Color(minOf(1f, base.red * 1.12f + 0.04f), minOf(1f, base.green * 1.12f + 0.04f), minOf(1f, base.blue * 1.12f + 0.04f))
        paint.shader = LinearGradient(0f, 0f, 0f, s, top.toArgb(), base.toArgb(), Shader.TileMode.CLAMP)
        canvas.drawRect(0f, 0f, s, s, paint)
        paint.shader = null
        if (Glyphs.has(look.icon)) {
            val g = s * 0.56f
            Glyphs.draw(canvas, look.icon!!, (s - g) / 2, (s - g) / 2, g, android.graphics.Color.WHITE)
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
    val bitmap = remember(look, px, title) { tileBitmap(look, px, title).asImageBitmap() }
    Image(bitmap, contentDescription = null, modifier = Modifier.size(size).shadow(size * 0.06f, RoundedCornerShape(size * 0.225f), clip = false))
}

@Composable
fun AppTile(app: AppInfo?, seed: String, size: Dp) = AppTile(app?.look() ?: TileLook(null, seed), size, app?.label ?: seed)

/** A row's icon, as Settings draws one: a white glyph (or symbol) on a system colour. */
@Composable
fun SymbolTile(icon: ImageVector? = null, glyph: String? = null, color: Color, size: Dp = 29.dp) {
    val context = LocalContext.current
    Glyphs.load(context)
    Box(Modifier.size(size).background(Brush.verticalGradient(listOf(color.copy(alpha = 0.85f), color)), RoundedCornerShape(size * 0.225f)), contentAlignment = Alignment.Center) {
        if (glyph != null && Glyphs.has(glyph)) {
            val px = with(LocalDensity.current) { (size * 0.62f).roundToPx() }
            val bitmap = remember(glyph, px) {
                Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888).also { Glyphs.draw(Canvas(it), glyph, 0f, 0f, px.toFloat(), android.graphics.Color.WHITE, 2.2f) }.asImageBitmap()
            }
            Image(bitmap, null, Modifier.size(size * 0.62f))
        } else if (icon != null) {
            androidx.compose.material3.Icon(icon, null, tint = Color.White, modifier = Modifier.size(size * 0.62f))
        }
    }
}

/** A person: their initials on a colour picked by their id, the same one app screens draw (`one.avatar`). */
@Composable
fun Avatar(id: String, name: String, size: Dp = 32.dp) {
    val colors = listOf(0x5e5ce6, 0xff9f0a, 0x30b0c7, 0xff375f, 0x34c759, 0xaf52de, 0x0a84ff, 0xa2845e)
    val h = id.fold(0L) { h, c -> (h * 31 + c.code) and 0xFFFFFFFF }
    val color = Color(0xFF000000 or colors[(h % colors.size).toInt()].toLong())
    val initials = name.split(" ").mapNotNull { it.firstOrNull()?.toString() }.take(2).joinToString("").uppercase()
    Box(Modifier.size(size).background(Brush.verticalGradient(listOf(color.copy(alpha = 0.85f), color)), CircleShape), contentAlignment = Alignment.Center) {
        Text(initials.ifEmpty { "?" }, color = Color.White, style = TextStyle(fontSize = (size.value * 0.4f).sp, fontWeight = FontWeight.SemiBold))
    }
}

/** A device's symbol, guessed from its name. */
fun deviceGlyph(name: String): String {
    val n = name.lowercase()
    return if (n.contains("mac") || n.contains("laptop") || n.contains("book")) "desktopcomputer" else if (n.contains("phone") || n.contains("pixel") || n.contains("android")) "phone" else "cpu"
}

fun displayName(name: String) = name.removeSuffix(".local")
