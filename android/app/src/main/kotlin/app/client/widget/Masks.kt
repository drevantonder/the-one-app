package app.client.widget

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.DashPathEffect
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import androidx.core.graphics.PathParser
import app.client.Glyphs
import kotlin.math.max
import kotlin.math.roundToInt

// What a home-screen widget can't draw with views: rings, bars, sparklines, charts and icons, as white shapes on clear,
// which the widget tints with a colour for light and one for dark, so they follow the phone as its text does. Two
// colours are two shapes, one over the other.

class Masks(private val scale: Float) {
    fun px(dp: Float) = max(1, (dp * scale).roundToInt())

    private fun paint(alpha: Float = 1f, stroke: Float? = null) = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = android.graphics.Color.WHITE
        this.alpha = (alpha * 255).roundToInt()
        if (stroke != null) {
            style = Paint.Style.STROKE
            strokeWidth = stroke
            strokeCap = Paint.Cap.ROUND
            strokeJoin = Paint.Join.ROUND
        }
    }

    private fun draw(w: Float, h: Float, block: Canvas.(Float, Float) -> Unit): Bitmap {
        val bitmap = Bitmap.createBitmap(px(w), px(h), Bitmap.Config.ARGB_8888)
        Canvas(bitmap).block(bitmap.width.toFloat(), bitmap.height.toFloat())
        return bitmap
    }

    /** A ring's track (`full`) or its arc, from the top, `f` round. */
    fun ring(size: Float, f: Float, full: Boolean, alpha: Float = 1f) = draw(size, size) { w, _ ->
        val line = minOf(5 * scale, w / 6)
        val r = RectF(line / 2, line / 2, w - line / 2, w - line / 2)
        if (full) drawArc(r, 0f, 360f, false, paint(alpha, line).apply { strokeCap = Paint.Cap.BUTT })
        else if (f > 0) drawArc(r, -90f, 360f * f, false, paint(alpha, line))
    }

    /** A bar's track (`f` 1) or how far it's filled. */
    fun bar(width: Float, height: Float, f: Float, alpha: Float = 1f) = draw(width, height) { w, h ->
        val filled = if (f >= 1f) w else max(h, w * f)
        drawRoundRect(RectF(0f, 0f, filled, h), h / 2, h / 2, paint(alpha))
    }

    /** A line through numbers from 0 to `top`, or the shade under it; a dashed line when there are none. */
    fun spark(values: List<Double>, top: Double?, width: Float, height: Float, area: Boolean) = draw(width, height) { w, h ->
        if (values.isEmpty()) {
            if (!area) drawLine(4f, h / 2, w - 4, h / 2, paint(0.5f, 1.5f * scale).apply { pathEffect = DashPathEffect(floatArrayOf(2 * scale, 3 * scale), 0f) })
            return@draw
        }
        val data = if (values.size == 1) listOf(values[0], values[0]) else values
        val most = top ?: maxOf(1.0, data.max())
        val line = Path()
        data.forEachIndexed { i, v ->
            val x = i.toFloat() / (data.size - 1) * w
            val y = h - 1 - (v.coerceIn(0.0, most) / most).toFloat() * (h - 2)
            if (i == 0) line.moveTo(x, y) else line.lineTo(x, y)
        }
        if (area) {
            line.lineTo(w, h)
            line.lineTo(0f, h)
            line.close()
            drawPath(line, paint(0.2f))
        } else drawPath(line, paint(1f, 1.4f * scale))
    }

    /** Bars, or a line, for numbers from 0 to the most of them. */
    fun chart(values: List<Double>, line: Boolean, width: Float, height: Float) = draw(width, height) { w, h ->
        if (values.isEmpty()) return@draw
        val top = maxOf(values.max(), 0.0001)
        val slot = w / values.size
        if (line) {
            val path = Path()
            values.forEachIndexed { i, v ->
                val x = slot * (i + 0.5f)
                val y = h - (v / top).toFloat() * (h - 2 * scale) - scale
                if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
            }
            drawPath(path, paint(1f, 2 * scale))
        } else {
            val bw = slot * 0.6f
            values.forEachIndexed { i, v ->
                val bh = max(scale, (v / top).toFloat() * h)
                drawRoundRect(RectF(slot * i + (slot - bw) / 2, h - bh, slot * i + (slot + bw) / 2, h), 2 * scale, 2 * scale, paint())
            }
        }
    }

    /** A Phosphor icon by name, filled; a dot if there's none by that name. */
    fun glyph(name: String, size: Float) = draw(size, size) { w, _ ->
        val path = Glyphs.path(name)
        if (path == null) {
            drawCircle(w / 2, w / 2, w / 6, paint())
            return@draw
        }
        drawPath(Path(path).apply { transform(Matrix().apply { setScale(w / 256f, w / 256f) }) }, paint())
    }

    /** An SVG path on a 24-point grid, outlined (as an icon's `path`) or filled (its badge, and the disc behind it). */
    fun svg(d: String, size: Float, stroke: Boolean) = draw(size, size) { w, _ ->
        val path = runCatching { PathParser.createPathFromPathData(d) }.getOrNull() ?: return@draw
        val s = w / 24f
        path.transform(Matrix().apply { setScale(s, s) })
        drawPath(path, if (stroke) paint(1f, 1.5f * s) else paint())
    }

    fun circle(size: Float) = draw(size, size) { w, _ -> drawCircle(w / 2, w / 2, w / 2, paint()) }
}
