package app.theone.renderer;

import android.content.Context;
import android.content.res.Configuration;
import android.graphics.Color;
import android.graphics.Outline;
import android.graphics.Rect;
import android.view.HapticFeedbackConstants;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewOutlineProvider;
import androidx.activity.BackEventCompat;
import androidx.activity.ComponentActivity;
import androidx.activity.OnBackPressedCallback;
import androidx.annotation.Keep;
import java.util.ArrayList;

/**
 * Where an activity's native windows are (`renderer::platform::android`): each a view over its content, the newest in
 * front, which Back closes. Its activity attaches it as it's made, which makes the UI thread the native windows'
 * thread, and says when the system turns light or dark, animations go on or off, or the font size changes. Rust calls the rest, on the UI thread.
 * Kept whole (`@Keep`) as a release build shrinks it: Rust calls it by name.
 */
@Keep
public final class NativeHost {
    private static ComponentActivity activity;
    private static Runnable changed;
    private static final ArrayList<NativeView> open = new ArrayList<>();
    /** The view under every other, as a phone's launcher's is: Back doesn't close it. */
    private static NativeView root;
    private static OnBackPressedCallback back;
    /** What dims what's under an app's view as the rail's line pulls it home (`moved`), under every app's view. */
    private static View dim;

    private NativeHost() {}

    /** `activity` hosts native windows from now; `changed` is told as the first opens and the last closes. */
    public static void attach(ComponentActivity activity, Runnable changed) {
        NativeHost.activity = activity;
        NativeHost.changed = changed;
        nativeAttach(dark(activity), reduced(), activity.getResources().getConfiguration().fontScale);
    }

    public static void configurationChanged(Context context) {
        nativeAppearance(dark(context), reduced(), context.getResources().getConfiguration().fontScale);
    }

    /** Whether a native window is open, which Back closes: an app's, over the launcher's. */
    public static boolean isOpen() {
        return !open.isEmpty();
    }

    /** Remove animations, in Accessibility: animators run at no length, as Chrome reads `prefers-reduced-motion`. */
    private static boolean reduced() {
        return !android.animation.ValueAnimator.areAnimatorsEnabled();
    }

    private static boolean dark(Context context) {
        return (context.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
    }

    /**
     * A window's view, over the page, or with `root` under every other window's. Every SurfaceView's surface is
     * under the activity's window, in no order among those of a layer, so an app's is a media overlay's, the layer
     * above, over the root's.
     */
    static NativeView open(long id, String title, boolean root) {
        NativeView view = new NativeView(activity, id);
        view.setContentDescription(title);
        // Hidden until its page has what it first asked for (`show`): a hidden SurfaceView has no surface yet.
        view.setVisibility(android.view.View.INVISIBLE);
        ViewGroup content = activity.findViewById(android.R.id.content);
        ViewGroup.LayoutParams fill = new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT);
        if (root) {
            NativeHost.root = view;
            content.addView(view, open.isEmpty() ? content.getChildCount() : content.indexOfChild(open.get(0)), fill);
            listen();
            return view;
        }
        view.setZOrderMediaOverlay(true);
        // The rail's line, in the middle of its right edge, is pulled in, which the system's back gesture from that edge
        // would take: that stretch of the edge is the view's (DESIGN.md, The rail), a fingertip wide and taller than it.
        if (android.os.Build.VERSION.SDK_INT >= 29) {
            view.addOnLayoutChangeListener((changed, left, top, right, bottom, wasLeft, wasTop, wasRight, wasBottom) -> {
                float density = changed.getResources().getDisplayMetrics().density;
                int middle = (bottom - top) / 2;
                Rect line = new Rect(right - left - Math.round(24 * density), middle - Math.round(56 * density), right - left, middle + Math.round(56 * density));
                changed.setSystemGestureExclusionRects(java.util.Collections.singletonList(line));
            });
        }
        content.addView(view, fill);
        open.add(view);
        listen();
        changed();
        return view;
    }

    /**
     * Back goes to the newest window, else the root, from the first that opens: added after the activity's own, it's
     * asked first. What it doesn't take (the root with nothing open over its page) goes on to the activity's.
     *
     * An app's window that Back closes follows the back gesture as it's made, as Material's predictive back has a
     * screen do: smaller, toward the edge it's swiped from, its corners rounding, what's under it showing round it;
     * let go, it goes, and taken back, it springs back.
     */
    private static void listen() {
        if (back != null) return;
        back = new OnBackPressedCallback(true) {
            private NativeView going;

            @Override public void handleOnBackStarted(BackEventCompat event) {
                NativeView to = open.isEmpty() ? null : open.get(open.size() - 1);
                going = to != null && to.closes() ? to : null;
            }

            @Override public void handleOnBackProgressed(BackEventCompat event) {
                if (going == null) return;
                // Material's: 90% at the most, moved toward the edge by a twentieth of the width less 8 dp.
                float progress = event.getProgress();
                float width = going.getWidth() / going.getResources().getDisplayMetrics().density;
                float toward = event.getSwipeEdge() == BackEventCompat.EDGE_LEFT ? 1 : -1;
                moved(going, 1 - 0.1f * progress, toward * (width / 20 - 8) * progress, 0, 1, 32 * progress, 0);
            }

            @Override public void handleOnBackCancelled() {
                if (going == null) return;
                NativeView stays = going;
                going = null;
                stays.animate().scaleX(1).scaleY(1).translationX(0).setDuration(200).withEndAction(() -> moved(stays, 1, 0, 0, 1, 0, 0)).start();
            }

            @Override public void handleOnBackPressed() {
                NativeView to = open.isEmpty() ? root : open.get(open.size() - 1);
                if (to != null && to == going) {
                    // It goes as it is, fading, then closes.
                    going = null;
                    to.animate().alpha(0).scaleX(to.getScaleX() * 0.95f).scaleY(to.getScaleY() * 0.95f).setDuration(150).withEndAction(() -> {
                        if (!to.back()) moved(to, 1, 0, 0, 1, 0, 0);
                    }).start();
                    return;
                }
                going = null;
                if (to != null && to.back()) return;
                setEnabled(false);
                activity.getOnBackPressedDispatcher().onBackPressed();
                setEnabled(true);
            }
        };
        activity.getOnBackPressedDispatcher().addCallback(activity, back);
    }

    /** Shows it, once Rust has let go of the windows: its surface comes as it's shown, which Rust draws on. */
    static void show(NativeView view) {
        view.post(() -> view.setVisibility(android.view.View.VISIBLE));
    }

    /**
     * Draws `view` as the rail's pull has it: grown by `scale` about its middle, moved by `x`, `y` points, as opaque as
     * `opacity`, its corners `corner` points round, and what's under it dimmed by `dim` (the black over it, 0 to 1). An
     * app's view is a media overlay, over the activity's window, so the dim, a view in it, is under every app's.
     */
    static void moved(NativeView view, float scale, float x, float y, float opacity, float corner, float dim) {
        float density = view.getResources().getDisplayMetrics().density;
        view.setScaleX(scale);
        view.setScaleY(scale);
        view.setTranslationX(x * density);
        view.setTranslationY(y * density);
        view.setAlpha(opacity);
        view.setOutlineProvider(new ViewOutlineProvider() {
            @Override public void getOutline(View of, Outline outline) {
                outline.setRoundRect(0, 0, of.getWidth(), of.getHeight(), corner * density);
            }
        });
        view.setClipToOutline(corner > 0);
        if (NativeHost.dim == null) {
            NativeHost.dim = new View(activity);
            NativeHost.dim.setBackgroundColor(Color.BLACK);
            ViewGroup content = activity.findViewById(android.R.id.content);
            content.addView(NativeHost.dim, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        }
        NativeHost.dim.setAlpha(dim);
        NativeHost.dim.setVisibility(dim > 0 ? View.VISIBLE : View.GONE);
    }

    /** Hides it, as the root is while the page shows an app over it; `show` shows it again. */
    static void hide(NativeView view) {
        view.post(() -> view.setVisibility(android.view.View.INVISIBLE));
    }

    static void close(NativeView view) {
        ViewGroup parent = (ViewGroup) view.getParent();
        if (parent != null) parent.removeView(view);
        if (view == root) root = null;
        open.remove(view);
        changed();
    }

    static void activate(NativeView view) {
        // The root stays under every other.
        if (view == root) return;
        view.bringToFront();
        open.remove(view);
        open.add(view);
    }

    /** What's on Android's clipboard, as text, for a keyboard's Ctrl+V. */
    static String paste() {
        android.content.ClipboardManager clipboard = activity.getSystemService(android.content.ClipboardManager.class);
        android.content.ClipData clip = clipboard == null ? null : clipboard.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) return null;
        CharSequence text = clip.getItemAt(0).coerceToText(activity);
        return text == null ? null : text.toString();
    }

    /** Puts `text` on Android's clipboard, for a keyboard's Ctrl+C. */
    static void copy(String text) {
        android.content.ClipboardManager clipboard = activity.getSystemService(android.content.ClipboardManager.class);
        if (clipboard != null) clipboard.setPrimaryClip(android.content.ClipData.newPlainText(null, text));
    }

    /** Android's share sheet for `text`, as a page's `navigator.share` opens it. */
    static void share(String text) {
        android.content.Intent send = new android.content.Intent(android.content.Intent.ACTION_SEND).setType("text/plain").putExtra(android.content.Intent.EXTRA_TEXT, text);
        activity.startActivity(android.content.Intent.createChooser(send, null));
    }

    /**
     * A haptic, by iOS's name (`haptic` in the UI library), played on `view` as Android's own, which follows the
     * system's Touch feedback: Flutter's choices for iOS's impacts and selection.
     */
    public static void haptic(View view, String kind) {
        boolean modern = android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R;
        int played;
        switch (kind) {
            case "light": case "soft": played = HapticFeedbackConstants.VIRTUAL_KEY; break;
            case "medium": played = HapticFeedbackConstants.KEYBOARD_TAP; break;
            case "heavy": case "rigid": played = HapticFeedbackConstants.CONTEXT_CLICK; break;
            case "selection": played = HapticFeedbackConstants.CLOCK_TICK; break;
            case "success": played = modern ? HapticFeedbackConstants.CONFIRM : HapticFeedbackConstants.VIRTUAL_KEY; break;
            case "warning": played = HapticFeedbackConstants.LONG_PRESS; break;
            case "error": played = modern ? HapticFeedbackConstants.REJECT : HapticFeedbackConstants.LONG_PRESS; break;
            default: return;
        }
        view.performHapticFeedback(played);
    }

    static float density() {
        return activity.getResources().getDisplayMetrics().density;
    }

    /**
     * How pressed the phone is, as the Compute Pressure API's states, 0 nominal to 3 critical: its thermal status
     * (Android 10's), light or moderate fair, severe serious and worse critical, and Battery Saver serious, as Android
     * has apps do less then.
     */
    /** The display's refresh rate, in frames a second, or 0 before there's an activity to ask. */
    static float refreshRate() {
        return activity == null ? 0f : activity.getWindowManager().getDefaultDisplay().getRefreshRate();
    }

    static int pressure() {
        android.os.PowerManager power = activity == null ? null : activity.getSystemService(android.os.PowerManager.class);
        if (power == null) return 0;
        int state = 0;
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            int thermal = power.getCurrentThermalStatus();
            if (thermal >= android.os.PowerManager.THERMAL_STATUS_CRITICAL) state = 3;
            else if (thermal >= android.os.PowerManager.THERMAL_STATUS_SEVERE) state = 2;
            else if (thermal >= android.os.PowerManager.THERMAL_STATUS_LIGHT) state = 1;
        }
        return power.isPowerSaveMode() ? Math.max(state, 2) : state;
    }

    private static void changed() {
        if (back != null) back.setEnabled(!open.isEmpty() || root != null);
        if (changed != null) changed.run();
    }

    private static native void nativeAttach(boolean dark, boolean reduced, float fontScale);
    private static native void nativeAppearance(boolean dark, boolean reduced, float fontScale);
}
