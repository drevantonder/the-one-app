package app.theone.renderer;

import android.content.Context;
import android.content.res.Configuration;
import android.view.ViewGroup;
import androidx.activity.ComponentActivity;
import androidx.activity.OnBackPressedCallback;
import androidx.annotation.Keep;
import java.util.ArrayList;

/**
 * Where an activity's native windows are (`renderer::platform::android`): each a view over its content, the newest in
 * front, which Back closes. Its activity attaches it as it's made, which makes the UI thread the native windows'
 * thread, and says when the system turns light or dark, or animations go on or off. Rust calls the rest, on the UI thread.
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

    private NativeHost() {}

    /** `activity` hosts native windows from now; `changed` is told as the first opens and the last closes. */
    public static void attach(ComponentActivity activity, Runnable changed) {
        NativeHost.activity = activity;
        NativeHost.changed = changed;
        nativeAttach(dark(activity), reduced());
    }

    public static void configurationChanged(Context context) {
        nativeAppearance(dark(context), reduced());
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
        content.addView(view, fill);
        open.add(view);
        listen();
        changed();
        return view;
    }

    /**
     * Back goes to the newest window, else the root, from the first that opens: added after the activity's own, it's
     * asked first. What it doesn't take (the root with nothing open over its page) goes on to the activity's.
     */
    private static void listen() {
        if (back != null) return;
        back = new OnBackPressedCallback(true) {
            @Override public void handleOnBackPressed() {
                NativeView to = open.isEmpty() ? root : open.get(open.size() - 1);
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

    static float density() {
        return activity.getResources().getDisplayMetrics().density;
    }

    private static void changed() {
        if (back != null) back.setEnabled(!open.isEmpty() || root != null);
        if (changed != null) changed.run();
    }

    private static native void nativeAttach(boolean dark, boolean reduced);
    private static native void nativeAppearance(boolean dark, boolean reduced);
}
