package app.theone.client

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Path
import android.graphics.Point
import android.hardware.display.DisplayManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Display
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import androidx.annotation.RequiresApi
import androidx.core.app.NotificationCompat
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sqrt

/**
 * Computer Use on this phone: what an agent sees of its screen and does on it, as its person asked, through Android's
 * accessibility service, the one way Android lets an app see the screen and touch it without asking again each time.
 * Its person turns it on in Settings → Accessibility, where it's The One App. While it's on, Android keeps it
 * running. A server acting for its person asks the app's Rust (`apps/computer-use/client/native`), which asks this,
 * on a thread of its own, which waits here for Android to have done it.
 *
 * x and y are pixels of the screenshot, which is the screen scaled down to what a model sees unshrunk, as a
 * computer's is (`ComputerUse.Desktop`). Each action says what it did, or throws why it couldn't.
 */
class ComputerUse : AccessibilityService() {
  override fun onServiceConnected() {
    running = this
  }

  override fun onUnbind(intent: Intent?): Boolean {
    running = null
    return super.onUnbind(intent)
  }

  override fun onAccessibilityEvent(event: AccessibilityEvent?) {}

  override fun onInterrupt() {}

  companion object {
    @Volatile private var running: ComputerUse? = null

    /** What's said wherever it's off, as the switch is in Android's Settings. */
    private const val OFF = "Computer Use isn't on for this phone: its person turns on The One App in Settings → Accessibility, as Computer Use in the app shows them."
    private const val CHANNEL = "computer-use"
    private const val ASKED = 7700

    /** The screen, for `screens`: the screenshot's width and height, and whether its person turned it on. */
    @JvmStatic
    fun screen(context: Context): String {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) throw IllegalStateException("Computer Use needs Android 11 or later.")
      val (width, height) = picture(real(context))
      return """{"width":$width,"height":$height,"on":${running != null}}"""
    }

    /** A PNG of the screen as it is now. */
    @JvmStatic
    fun screenshot(context: Context): ByteArray = service(context).shot()

    /** A tap, or with `right` a long press, as a phone has for what a right-click is on a computer. */
    @JvmStatic
    fun click(context: Context, x: Int, y: Int, button: String): String {
      val service = service(context)
      val held = when (button) {
        "left" -> 50L
        "right" -> 800L
        else -> throw IllegalStateException("A phone has no $button button: click is a tap, and right a long press.")
      }
      val at = service.at(x, y)
      service.gesture(GestureDescription.StrokeDescription(dot(at), 0, held))
      return if (button == "right") "Long-pressed at $x, $y" else "Tapped at $x, $y"
    }

    @JvmStatic
    fun doubleClick(context: Context, x: Int, y: Int): String {
      val service = service(context)
      val at = service.at(x, y)
      service.gesture(GestureDescription.StrokeDescription(dot(at), 0, 50), GestureDescription.StrokeDescription(dot(at), 150, 50))
      return "Double-tapped at $x, $y"
    }

    /** A finger down at the first point, through the rest, and up at the last, as a drag is on a touch screen. */
    @JvmStatic
    fun drag(context: Context, path: IntArray): String {
      val service = service(context)
      val through = Path()
      for (i in path.indices step 2) {
        val at = service.at(path[i], path[i + 1])
        if (i == 0) through.moveTo(at.x.toFloat(), at.y.toFloat()) else through.lineTo(at.x.toFloat(), at.y.toFloat())
      }
      service.gesture(GestureDescription.StrokeDescription(through, 0, 600))
      return "Dragged through ${path.size / 2} points"
    }

    /**
     * A swipe the other way from where it scrolls, as a finger scrolls a phone, `amount` steps of 48 dp, kept on the
     * screen. The finger rests before it lifts, so what it scrolls stops there rather than flinging on.
     */
    @JvmStatic
    fun scroll(context: Context, x: Int, y: Int, direction: String, amount: Int): String {
      val service = service(context)
      val at = service.at(x, y)
      val size = real(service)
      val step = amount * 48 * service.resources.displayMetrics.density
      val (dx, dy) = when (direction) {
        "up" -> 0f to step
        "down" -> 0f to -step
        "left" -> step to 0f
        "right" -> -step to 0f
        else -> throw IllegalStateException("Scroll up, down, left or right.")
      }
      val toX = (at.x + dx).coerceIn(1f, size.x - 2f)
      val toY = (at.y + dy).coerceIn(1f, size.y - 2f)
      val swipe = Path().apply { moveTo(at.x.toFloat(), at.y.toFloat()); lineTo(toX, toY) }
      val moved = GestureDescription.StrokeDescription(swipe, 0, 400, true)
      service.gesture(moved)
      service.gesture(moved.continueStroke(Path().apply { moveTo(toX, toY) }, 0, 150, false))
      return "Scrolled $direction $amount"
    }

    /** Puts `text` where the selection is in the field that has the focus, as typing there would. */
    @JvmStatic
    fun type(context: Context, text: String): String {
      val service = service(context)
      val field = service.focused() ?: throw IllegalStateException("Nothing on the screen has the focus to type into: tap a text field first.")
      if (field.isPassword) throw IllegalStateException("That's a password field: ask your person to type it themselves.")
      val was = if (field.isShowingHintText) "" else field.text?.toString().orEmpty()
      val start = field.textSelectionStart.takeIf { it in 0..was.length } ?: was.length
      val end = field.textSelectionEnd.takeIf { it in start..was.length } ?: start
      val now = was.substring(0, start) + text + was.substring(end)
      val set = Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, now) }
      if (!field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, set)) throw IllegalStateException("The field that has the focus doesn't take text.")
      val after = Bundle().apply {
        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, start + text.length)
        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, start + text.length)
      }
      field.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, after)
      return "Typed ${text.length} characters"
    }

    /**
     * Presses `keys`, space between, one after another: a phone has only Back (or Escape), Home, Recents and
     * Notifications, which Android gives an accessibility service, and Return in the field that has the focus.
     */
    @JvmStatic
    fun key(context: Context, keys: String): String {
      val service = service(context)
      val pressed = keys.split(" ").filter { it.isNotEmpty() }
      val actions = pressed.map {
        when (it.lowercase()) {
          "back", "escape" -> GLOBAL_ACTION_BACK
          "home" -> GLOBAL_ACTION_HOME
          "recents" -> GLOBAL_ACTION_RECENTS
          "notifications" -> GLOBAL_ACTION_NOTIFICATIONS
          "return", "enter" -> null
          else -> throw IllegalStateException("A phone has no $it key: it has Back, Home, Recents, Notifications and Return. Type text with `type`.")
        }
      }
      for (action in actions) {
        val done = if (action != null) service.performGlobalAction(action) else service.enter()
        if (!done) throw IllegalStateException("Android didn't press ${pressed[actions.indexOf(action)]}.")
      }
      return "Pressed $keys"
    }

    /**
     * The service, while its person has it on; and if they haven't, they're told on the phone why nothing happened.
     * Android has taken screenshots for an accessibility service since 11, which is what Computer Use needs.
     */
    private fun service(context: Context): ComputerUse {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) throw IllegalStateException("Computer Use needs Android 11 or later.")
      return running ?: run {
        asked(context)
        throw IllegalStateException(OFF)
      }
    }

    /**
     * Tells the phone's person that their AI asked to use it, and how to let it: a tap opens Computer Use in the app,
     * which says so and opens Settings. Shown again in place each time, never more than one.
     */
    private fun asked(context: Context) {
      val manager = context.getSystemService(NotificationManager::class.java)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Computer Use", NotificationManager.IMPORTANCE_HIGH))
      }
      val tap = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("app", "computer-use")
      val shown = NotificationCompat.Builder(context, CHANNEL)
        .setSmallIcon(R.mipmap.ic_launcher)
        .setContentTitle("Your AI asked to use this phone")
        .setContentText("To let it, turn on The One App in Accessibility settings.")
        .setAutoCancel(true)
        .setContentIntent(PendingIntent.getActivity(context, ASKED, tap, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
        .build()
      manager.notify(ASKED, shown)
    }

    /** The screen's whole size, in its pixels, as it's turned now. */
    @Suppress("DEPRECATION")
    private fun real(context: Context): Point =
      Point().also { context.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY).getRealSize(it) }

    /** The screenshot's size for a screen of `size`: at most 1568 pixels on its long side and 1.15 megapixels. */
    private fun picture(size: Point): Pair<Int, Int> {
      val scale = min(1.0, min(1568.0 / maxOf(size.x, size.y), sqrt(1_150_000.0 / (size.x.toDouble() * size.y))))
      return (size.x * scale).roundToInt() to (size.y * scale).roundToInt()
    }

    private fun dot(at: Point) = Path().apply { moveTo(at.x.toFloat(), at.y.toFloat()) }
  }

  /** The point on the screen that's at `x`, `y` of the screenshot. */
  private fun at(x: Int, y: Int): Point {
    val size = real(this)
    val (width, height) = picture(size)
    if (x !in 0 until width || y !in 0 until height) throw IllegalStateException("$x, $y is off the screen, which is $width×$height")
    return Point((x * size.x / width.toFloat()).roundToInt(), (y * size.y / height.toFloat()).roundToInt())
  }

  /** Has Android make `strokes` as one gesture, and waits until it has. */
  private fun gesture(vararg strokes: GestureDescription.StrokeDescription) {
    val gesture = GestureDescription.Builder().apply { strokes.forEach(::addStroke) }.build()
    val done = CountDownLatch(1)
    var completed = false
    val callback = object : GestureResultCallback() {
      override fun onCompleted(gestureDescription: GestureDescription?) {
        completed = true
        done.countDown()
      }

      override fun onCancelled(gestureDescription: GestureDescription?) = done.countDown()
    }
    if (!dispatchGesture(gesture, callback, Handler(Looper.getMainLooper()))) throw IllegalStateException("Android didn't take the gesture.")
    done.await(70, TimeUnit.SECONDS)
    if (!completed) throw IllegalStateException("Android cancelled the gesture, as something else touched the screen.")
  }

  @RequiresApi(Build.VERSION_CODES.R)
  private fun shot(): ByteArray {
    val done = CountDownLatch(1)
    var png: ByteArray? = null
    var failed: String? = null
    takeScreenshot(Display.DEFAULT_DISPLAY, mainExecutor, object : TakeScreenshotCallback {
      override fun onSuccess(screenshot: ScreenshotResult) {
        val buffer = screenshot.hardwareBuffer
        val shown = Bitmap.wrapHardwareBuffer(buffer, screenshot.colorSpace)?.copy(Bitmap.Config.ARGB_8888, false)
        buffer.close()
        if (shown == null) {
          failed = "Android gave a screenshot that couldn't be read."
        } else {
          val (width, height) = picture(Point(shown.width, shown.height))
          val scaled = Bitmap.createScaledBitmap(shown, width, height, true)
          png = ByteArrayOutputStream().also { scaled.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        }
        done.countDown()
      }

      override fun onFailure(errorCode: Int) {
        failed = when (errorCode) {
          ERROR_TAKE_SCREENSHOT_INTERVAL_TIME_SHORT -> "Android takes a screenshot at most every third of a second: take it again."
          ERROR_TAKE_SCREENSHOT_SECURE_WINDOW -> "What's on the screen is secure, so Android shows no screenshot of it."
          else -> "Android couldn't take a screenshot (error $errorCode)."
        }
        done.countDown()
      }
    })
    done.await(10, TimeUnit.SECONDS)
    return png ?: throw IllegalStateException(failed ?: "Android took no screenshot within 10 seconds.")
  }

  /** The field that has the focus to type into, in any window on the screen. */
  private fun focused(): AccessibilityNodeInfo? = findFocus(AccessibilityNodeInfo.FOCUS_INPUT)

  /** Return in the field that has the focus, as its keyboard's action key: Android 11 or later. */
  private fun enter(): Boolean {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) throw IllegalStateException("Return needs Android 11 or later.")
    val field = focused() ?: throw IllegalStateException("Nothing on the screen has the focus to press Return in: tap a text field first.")
    return field.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
  }
}
