package app.theone.strict;

import android.content.Context;
import android.content.res.Configuration;
import android.view.ViewGroup;
import androidx.activity.ComponentActivity;
import androidx.activity.OnBackPressedCallback;
import java.util.ArrayList;

/**
 * Where an activity's strict windows are (`strict::platform::android`): each a view over its content, the newest in
 * front, which Back closes. Its activity attaches it as it's made, which makes the UI thread the strict windows'
 * thread, and says when the system turns light or dark, or animations go on or off. Rust calls the rest, on the UI thread.
 */
public final class StrictHost {
    private static ComponentActivity activity;
    private static Runnable changed;
    private static final ArrayList<StrictView> open = new ArrayList<>();
    private static OnBackPressedCallback back;

    private StrictHost() {}

    /** `activity` hosts strict windows from now; `changed` is told as the first opens and the last closes. */
    public static void attach(ComponentActivity activity, Runnable changed) {
        StrictHost.activity = activity;
        StrictHost.changed = changed;
        nativeAttach(dark(activity), reduced());
    }

    public static void configurationChanged(Context context) {
        nativeAppearance(dark(context), reduced());
    }

    /** Whether a strict window is open, which Back closes. */
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

    static StrictView open(long id, String title) {
        StrictView view = new StrictView(activity, id);
        view.setContentDescription(title);
        // Hidden until its page has what it first asked for (`show`): a hidden SurfaceView has no surface yet.
        view.setVisibility(android.view.View.INVISIBLE);
        ViewGroup content = activity.findViewById(android.R.id.content);
        content.addView(view, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        open.add(view);
        // Added as the first window opens, after the activity's own, so it's asked first.
        if (back == null) {
            back = new OnBackPressedCallback(false) {
                @Override public void handleOnBackPressed() {
                    if (!open.isEmpty()) open.get(open.size() - 1).back();
                }
            };
            activity.getOnBackPressedDispatcher().addCallback(activity, back);
        }
        changed();
        return view;
    }

    /** Shows it, once Rust has let go of the windows: its surface comes as it's shown, which Rust draws on. */
    static void show(StrictView view) {
        view.post(() -> view.setVisibility(android.view.View.VISIBLE));
    }

    static void close(StrictView view) {
        ViewGroup parent = (ViewGroup) view.getParent();
        if (parent != null) parent.removeView(view);
        open.remove(view);
        changed();
    }

    static void activate(StrictView view) {
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
        if (back != null) back.setEnabled(!open.isEmpty());
        if (changed != null) changed.run();
    }

    private static native void nativeAttach(boolean dark, boolean reduced);
    private static native void nativeAppearance(boolean dark, boolean reduced);
}
