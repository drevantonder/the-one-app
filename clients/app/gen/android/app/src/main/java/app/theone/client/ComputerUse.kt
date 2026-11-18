package app.theone.client

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Path
import android.graphics.Point
import android.graphics.Rect
import android.hardware.display.DisplayManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.Display
import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.View
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
  /**
   * Android keeps the app's process running while the service is on, from as the phone starts, so the app's client
   * starts with it, as a push would start it: the phone can then be asked without a push to wake it.
   */
  override fun onServiceConnected() {
    running = this
    Thread { Notifications.wake(this, dataDir.absolutePath) }.start()
  }

  override fun onUnbind(intent: Intent?): Boolean {
    running = null
    return super.onUnbind(intent)
  }

  /** Where the focus moved last, on any screen, as Android says, and who waits to hear the screen change. */
  @Volatile private var moved: AccessibilityNodeInfo? = null
  @Volatile private var awaited: CountDownLatch? = null

  override fun onAccessibilityEvent(event: AccessibilityEvent?) {
    if (event?.eventType == AccessibilityEvent.TYPE_VIEW_FOCUSED) event.source?.let { moved = it }
    awaited?.countDown()
  }

  override fun onInterrupt() {}

  companion object {
    @Volatile private var running: ComputerUse? = null

    private const val CHANNEL = "computer-use"
    private const val ASKED = 7700

    /** The screen, for `screens`: the screenshot's width and height, and whether its person turned it on. */
    @JvmStatic
    fun screen(context: Context): String {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) throw IllegalStateException("Computer Use needs Android 11 or later.")
      val (width, height) = picture(real(context))
      return """{"width":$width,"height":$height,"on":${running != null},"tv":${tv(context)}}"""
    }

    /** Whether this is a TV, which its remote's D-pad drives: Android TV's home is leanback, which no phone has. */
    private fun tv(context: Context) = context.packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)

    /** What this device is, as its person would say it. */
    private fun kind(context: Context) = if (tv(context)) "TV" else "phone"

    /** A PNG of the screen as it is now. */
    @JvmStatic
    fun screenshot(context: Context): ByteArray = service(context).shot()

    /**
     * A tap, or with `right` a long press, as a phone has for what a right-click is on a computer. A TV has no
     * touchscreen, and Android cancels a gesture made on one, so there it's what its remote does: the focus moves to
     * what's there, and it's pressed, or held.
     */
    @JvmStatic
    fun click(context: Context, x: Int, y: Int, button: String): String {
      val service = service(context)
      if (tv(service)) {
        val held = when (button) {
          "left" -> false
          "right" -> true
          else -> throw IllegalStateException("A TV's remote has no $button button: click presses what's there, and right holds it.")
        }
        return "${if (held) "Held" else "Pressed"} ${service.press(x, y, held)}"
      }
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
      if (tv(service)) return "Pressed ${service.press(x, y, false).also { service.press(x, y, false) }} twice"
      val at = service.at(x, y)
      service.gesture(GestureDescription.StrokeDescription(dot(at), 0, 50), GestureDescription.StrokeDescription(dot(at), 150, 50))
      return "Double-tapped at $x, $y"
    }

    /** A finger down at the first point, through the rest, and up at the last, as a drag is on a touch screen. */
    @JvmStatic
    fun drag(context: Context, path: IntArray): String {
      val service = service(context)
      if (tv(service)) throw IllegalStateException("A TV has no touchscreen to drag on: move the focus with `key` Up, Down, Left and Right.")
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
      if (tv(service)) return service.scrollTv(x, y, direction, amount)
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
     * Presses `keys`, space between, one after another. Back (or Escape), Home, Recents and Notifications are the
     * system's. Up, Down, Left, Right and Return are a D-pad's: on the app's own window they're pressed as its remote
     * presses them (`remote`); an accessibility service can't press a key on another app's, so there Up, Down, Left
     * and Right move the focus to what's next that way, and Return is typing's Return in a field, or anywhere else a
     * click on what has the focus.
     */
    @JvmStatic
    fun key(context: Context, keys: String): String {
      val service = service(context)
      val pressed = keys.split(" ").filter { it.isNotEmpty() }
      val actions: List<() -> String?> = pressed.map {
        when (it.lowercase()) {
          "back", "escape" -> { -> service.global(GLOBAL_ACTION_BACK, it) }
          "home" -> { -> service.global(GLOBAL_ACTION_HOME, it) }
          "recents" -> { -> service.global(GLOBAL_ACTION_RECENTS, it) }
          "notifications" -> { -> service.global(GLOBAL_ACTION_NOTIFICATIONS, it) }
          "up" -> { -> service.move(View.FOCUS_UP, KeyEvent.KEYCODE_DPAD_UP) }
          "down" -> { -> service.move(View.FOCUS_DOWN, KeyEvent.KEYCODE_DPAD_DOWN) }
          "left" -> { -> service.move(View.FOCUS_LEFT, KeyEvent.KEYCODE_DPAD_LEFT) }
          "right" -> { -> service.move(View.FOCUS_RIGHT, KeyEvent.KEYCODE_DPAD_RIGHT) }
          "return", "enter" -> service::enter
          else -> throw IllegalStateException("A ${kind(service)} has no $it key: it has Back, Home, Recents, Notifications, Up, Down, Left, Right and Return. Type text with `type`.")
        }
      }
      val now = actions.map { it() }.lastOrNull()
      return if (now != null) "Pressed $keys: the focus is on $now" else "Pressed $keys"
    }

    /**
     * The service, while its person has it on; and if they haven't, they're told on the phone why nothing happened.
     * Android has taken screenshots for an accessibility service since 11, which is what Computer Use needs.
     */
    private fun service(context: Context): ComputerUse {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) throw IllegalStateException("Computer Use needs Android 11 or later.")
      return running ?: run {
        asked(context)
        throw IllegalStateException("Computer Use isn't on for this ${kind(context)}: its person turns on The One App in Settings → Accessibility, as Computer Use in the app shows them.")
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
        .setContentTitle("Your AI asked to use this ${kind(context)}")
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

  private fun global(action: Int, key: String): String? {
    if (!performGlobalAction(action)) throw IllegalStateException("Android didn't press $key.")
    return null
  }

  /**
   * What has the focus: where it last moved, while it's still there; or the input focus a D-pad moves and a
   * keyboard types into, or else what TalkBack reads, or else the active window. A web view says the input focus is
   * on itself, its frame or the element at different times, of the same focus, so it's the innermost focused element.
   */
  private fun current(): AccessibilityNodeInfo {
    moved?.takeIf { it.refresh() && it.isFocused }?.let { return it }
    val focus = focused() ?: findFocus(AccessibilityNodeInfo.FOCUS_ACCESSIBILITY) ?: rootInActiveWindow
      ?: throw IllegalStateException("Nothing is on the screen.")
    return innermost(focus) ?: focus
  }

  private fun innermost(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
    for (i in 0 until node.childCount) node.getChild(i)?.let(::innermost)?.let { return it }
    return node.takeIf { it.isFocused }
  }

  /**
   * Moves the focus to what's next in `direction` from what has it, as a D-pad does, and says what that is. The app's
   * own window has its remote's key pressed, and moves it itself.
   */
  private fun move(direction: Int, code: Int): String {
    MainActivity.focused?.let { window ->
      val was = current()
      remote(window, code)
      return if (until { current() != was }) named(current()) else "${named(was)}, as nothing is there to move it to"
    }
    val from = current()
    val at = Rect().also(from::getBoundsInScreen)
    // From an element of another app's web page, Android's own search gives the web view, which holds it, or itself.
    val next = from.focusSearch(direction)?.takeUnless { Rect().also(it::getBoundsInScreen).contains(at) }
      ?: throw IllegalStateException("Nothing is there to move the focus to.")
    if (!next.performAction(AccessibilityNodeInfo.ACTION_FOCUS)) throw IllegalStateException("Android didn't move the focus.")
    moved = next
    return named(next)
  }

  /**
   * Presses `code` on the app's own window, down and up, as its remote does. An accessibility service can't press a
   * key, Android 14 lets no app inject one, and Android's own click doesn't reach a web page's elements in WebView
   * 157 (no click event comes); but this service runs in the app, beside its window, which takes the key as it would
   * the remote's: its web page moves the focus, or presses what has it, as the page has it do. A remote's first key
   * gives the window's first view the focus where nothing has it, which Android does on the way to the window, so
   * it's done here first: the web view is added once the window is first laid out, after Android gave the focus.
   */
  private fun remote(window: MainActivity, code: Int) {
    val pressed = CountDownLatch(1)
    mainExecutor.execute {
      if (window.currentFocus == null) window.window.decorView.requestFocus()
      val down = SystemClock.uptimeMillis()
      for (action in listOf(KeyEvent.ACTION_DOWN, KeyEvent.ACTION_UP)) {
        window.dispatchKeyEvent(KeyEvent(down, SystemClock.uptimeMillis(), action, code, 0, 0, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_DPAD))
      }
      pressed.countDown()
    }
    pressed.await(5, TimeUnit.SECONDS)
  }

  /**
   * Waits until `done`, as Android tells this the screen changes, a second at most, and says whether it is. A web
   * view says where its page's focus went only once it has drawn it again for accessibility, which just after the
   * app starts can take it a moment, and as a change of its content, not of the focus.
   */
  private fun until(done: () -> Boolean): Boolean {
    val end = SystemClock.uptimeMillis() + 1000
    while (true) {
      val heard = CountDownLatch(1).also { awaited = it }
      if (done()) return true
      val left = end - SystemClock.uptimeMillis()
      if (left <= 0 || !heard.await(left, TimeUnit.MILLISECONDS)) return done()
    }
  }

  /** What a node is, as its person would read it: its text, or what it's labelled, or else what kind of thing it is. */
  private fun named(node: AccessibilityNodeInfo): String {
    val at = Rect().also(node::getBoundsInScreen)
    val name = (node.text ?: node.contentDescription)?.toString()?.trim()?.takeIf { it.isNotEmpty() }?.let { "“${it.take(60)}”" }
      ?: node.className?.toString()?.substringAfterLast('.') ?: "something"
    val size = real(this)
    val (width, height) = picture(size)
    return "$name at ${at.centerX() * width / size.x}, ${at.centerY() * height / size.y}"
  }

  /**
   * What the focus moves to at `x`, `y` of the screenshot, as a remote's press would be on it: the innermost thing
   * there that's pressed or takes the focus, in the frontmost window that has one there. It's focused, then pressed,
   * or held, and said.
   */
  private fun press(x: Int, y: Int, held: Boolean): String {
    val point = at(x, y)
    val node = windows.firstNotNullOfOrNull { it.root?.let { root -> under(root, point) } }
      ?: throw IllegalStateException("Nothing at $x, $y is pressed or takes the focus.")
    val window = MainActivity.focused?.takeIf { node.packageName == packageName && !held }
    node.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
    moved = node
    if (window != null) {
      // The page takes the focus before the press, which come to it by different ways.
      until { node.refresh() && node.isFocused }
      remote(window, KeyEvent.KEYCODE_DPAD_CENTER)
      return named(node)
    }
    val action = if (held) AccessibilityNodeInfo.ACTION_LONG_CLICK else AccessibilityNodeInfo.ACTION_CLICK
    if (!node.performAction(action)) throw IllegalStateException("Android didn't press ${named(node)}.")
    return named(node)
  }

  private fun under(node: AccessibilityNodeInfo, point: Point): AccessibilityNodeInfo? {
    if (!node.isVisibleToUser || !Rect().also(node::getBoundsInScreen).contains(point.x, point.y)) return null
    for (i in node.childCount - 1 downTo 0) node.getChild(i)?.let { under(it, point) }?.let { return it }
    return node.takeIf { it.isClickable || it.isFocusable }
  }

  /** Scrolls what scrolls at `x`, `y` a page at a time, `amount` times at most, as a remote's D-pad would. */
  private fun scrollTv(x: Int, y: Int, direction: String, amount: Int): String {
    val point = at(x, y)
    val action = when (direction) {
      "up" -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_UP
      "down" -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_DOWN
      "left" -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_LEFT
      "right" -> AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_RIGHT
      else -> throw IllegalStateException("Scroll up, down, left or right.")
    }
    fun scrolls(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
      if (!node.isVisibleToUser || !Rect().also(node::getBoundsInScreen).contains(point.x, point.y)) return null
      for (i in node.childCount - 1 downTo 0) node.getChild(i)?.let(::scrolls)?.let { return it }
      return node.takeIf { action in it.actionList }
    }
    val node = windows.firstNotNullOfOrNull { it.root?.let(::scrolls) } ?: throw IllegalStateException("Nothing at $x, $y scrolls $direction.")
    val scrolled = (1..amount).takeWhile { node.performAction(action.id) }.size
    return "Scrolled ${named(node)} $direction $scrolled"
  }

  /**
   * Return: in a field, its keyboard's action key; anywhere else, a click on what has the focus, as a D-pad's centre.
   * On the app's own window, its remote's Enter or centre.
   */
  private fun enter(): String {
    val at = current()
    MainActivity.focused?.let { window ->
      remote(window, if (at.isEditable) KeyEvent.KEYCODE_ENTER else KeyEvent.KEYCODE_DPAD_CENTER)
      return named(at)
    }
    val done = if (at.isEditable) {
      at.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
    } else {
      at.performAction(AccessibilityNodeInfo.ACTION_CLICK)
    }
    if (!done) throw IllegalStateException("Android didn't press Return on ${named(at)}.")
    return named(at)
  }
}
