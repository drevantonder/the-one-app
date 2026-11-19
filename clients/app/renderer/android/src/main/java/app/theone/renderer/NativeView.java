package app.theone.renderer;

import android.content.Context;
import android.graphics.Rect;
import android.util.Log;
import android.util.SparseArray;
import android.view.ActionMode;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.VelocityTracker;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewStructure;
import android.view.autofill.AutofillValue;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.widget.OverScroller;
import androidx.annotation.Keep;
import app.theone.renderer.editing.InputConnectionAdaptor;
import app.theone.renderer.editing.TextEditState;
import app.theone.renderer.editing.TextInputPlugin;
import org.json.JSONArray;
import org.json.JSONObject;

/**
 * An app's native window: a surface the renderer crate draws on (`renderer::platform::android`), by its window's id. It
 * passes on what a finger does, bridges the keyboard (TextInputPlugin, from Flutter's embedder) and autofill, and does
 * what Rust asks of it (`command`): Flutter's text input method calls (`TextInput.setClient`, `setEditingState`,
 * `show`…), or showing the Cut, Copy and Paste menu.
 * Kept whole (`@Keep`) as a release build shrinks it: Rust calls it by name.
 */
@Keep
public final class NativeView extends SurfaceView implements SurfaceHolder.Callback, TextInputPlugin.Framework {
    private static final String TAG = "renderer";
    private final long id;
    private final TextInputPlugin input;
    private float downX, downY;
    private final Runnable longPress = this::longPress;
    /**
     * A finger's speed, and the fling it leaves what it panned with as it lifts: Android's own (`OverScroller`, as a
     * ScrollView's and a RecyclerView's), stepped a display frame at a time, each step what's under where the finger
     * went down panned by (`nativePan`).
     */
    private VelocityTracker speed;
    private final OverScroller fling;
    private boolean panning;
    private int flungX, flungY;
    private final Runnable flinging = this::flinging;
    private ActionMode menu;

    NativeView(Context context, long id) {
        super(context);
        this.id = id;
        fling = new OverScroller(context);
        input = new TextInputPlugin(this, this);
        getHolder().addCallback(this);
        // It draws under the system's bars, the display's cutout and the keyboard, as an edge-to-edge app does, told
        // where they are.
        androidx.core.view.ViewCompat.setOnApplyWindowInsetsListener(this, (view, insets) -> {
            int kept = androidx.core.view.WindowInsetsCompat.Type.systemBars() | androidx.core.view.WindowInsetsCompat.Type.displayCutout() | androidx.core.view.WindowInsetsCompat.Type.ime();
            androidx.core.graphics.Insets covered = insets.getInsets(kept);
            nativeInsets(id, covered.left, covered.top, covered.right, covered.bottom);
            return insets;
        });
        setFocusable(true);
        setFocusableInTouchMode(true);
        setImportantForAutofill(IMPORTANT_FOR_AUTOFILL_YES);
        // Once Rust has let go of the window it's opening.
        post(() -> nativeAccessibility(id));
    }

    /** Back: what's open over its page shuts, or its app's window closes; the root's takes Back only for the first. → whether it took it. */
    boolean back() {
        return nativeBack(id);
    }

    /** What Rust asks, done on the UI thread once Rust has returned. */
    public void command(String command) {
        post(() -> apply(command));
    }

    private void apply(String command) {
        try {
            JSONObject c = new JSONObject(command);
            if (c.optBoolean("menu")) showMenu();
            if (c.has("pick")) pick(c.getJSONObject("pick"));
            if (c.has("share")) NativeHost.share(c.getString("share"));
            if (c.has("haptic")) NativeHost.haptic(this, c.getString("haptic"));
            textInput(c.optString("method"), c.opt("args"));
        } catch (Exception e) {
            Log.e(TAG, "command " + command, e);
        }
    }

    /**
     * Android's picker for picker node `node`: Material 3's dialog of its options that can be chosen, the one chosen
     * ticked, as Chrome's `<select>` opens. What's chosen goes back to Rust (`nativePicked`).
     */
    private void pick(JSONObject pick) throws org.json.JSONException {
        JSONArray options = pick.getJSONArray("options");
        String chosen = pick.isNull("chosen") ? null : pick.getString("chosen");
        java.util.List<String> labels = new java.util.ArrayList<>(), values = new java.util.ArrayList<>();
        int checked = -1;
        for (int i = 0; i < options.length(); i++) {
            JSONObject option = options.getJSONObject(i);
            if (option.optBoolean("disabled")) continue;
            String value = option.getString("value");
            if (value.equals(chosen)) checked = values.size();
            labels.add(option.optString("label", value));
            values.add(value);
        }
        long node = pick.getLong("node");
        new com.google.android.material.dialog.MaterialAlertDialogBuilder(getContext())
            .setSingleChoiceItems(labels.toArray(new String[0]), checked, (dialog, which) -> {
                dialog.dismiss();
                nativePicked(id, node, values.get(which));
            })
            .show();
    }

    /** Flutter's text input channel's method calls, as its TextInputPlugin takes them from its TextInputChannel. */
    private void textInput(String method, Object args) throws org.json.JSONException {
        switch (method) {
            case "TextInput.setClient": {
                JSONArray client = (JSONArray) args;
                input.setTextInputClient(client.getInt(0), client.getJSONObject(1));
                break;
            }
            case "TextInput.setEditingState":
                input.setTextInputEditingState(this, TextEditState.fromJson((JSONObject) args));
                break;
            case "TextInput.setEditableSizeAndTransform": {
                JSONObject size = (JSONObject) args;
                JSONArray transform = size.getJSONArray("transform");
                double[] matrix = new double[16];
                for (int i = 0; i < 16; i++) matrix[i] = transform.getDouble(i);
                input.setEditableSizeAndTransform(size.getDouble("width"), size.getDouble("height"), matrix);
                break;
            }
            case "TextInput.show":
                if (input.isTextEditor()) input.showTextInput(this);
                break;
            case "TextInput.hide":
                input.hideTextInput(this);
                break;
            case "TextInput.clearClient":
                if (menu != null) menu.finish();
                input.clearTextInputClient();
                break;
            default:
                break;
        }
    }

    /** Whether its window has the keyboard as it's put in it, which no focus change says. */
    @Override protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        // What covers it, which a view put in after the window's insets went out isn't told by itself.
        androidx.core.view.ViewCompat.requestApplyInsets(this);
        post(() -> nativeActive(id, hasWindowFocus()));
    }

    /** The window got or lost the keyboard: the caret shows only while it has it. */
    @Override public void onWindowFocusChanged(boolean hasWindowFocus) {
        super.onWindowFocusChanged(hasWindowFocus);
        nativeActive(id, hasWindowFocus);
    }

    @Override public void surfaceCreated(SurfaceHolder holder) {}

    @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        nativeSurface(id, holder.getSurface(), width, height);
    }

    @Override public void surfaceDestroyed(SurfaceHolder holder) {
        nativeSurfaceDestroyed(id);
    }

    @Override public boolean onTouchEvent(MotionEvent e) {
        int action = e.getActionMasked();
        float slop = ViewConfiguration.get(getContext()).getScaledTouchSlop();
        // A finger landing stops a fling, as it does a ScrollView's, and only that: it presses nothing.
        boolean stopped = action == MotionEvent.ACTION_DOWN && !fling.isFinished();
        if (action == MotionEvent.ACTION_DOWN) {
            fling.forceFinished(true);
            if (speed == null) speed = VelocityTracker.obtain();
            speed.clear();
            downX = e.getX();
            downY = e.getY();
            postDelayed(longPress, ViewConfiguration.getLongPressTimeout());
        } else if (action == MotionEvent.ACTION_MOVE) {
            if (Math.abs(e.getX() - downX) > slop || Math.abs(e.getY() - downY) > slop) removeCallbacks(longPress);
        } else {
            removeCallbacks(longPress);
        }
        // A second finger isn't followed.
        if (action > MotionEvent.ACTION_CANCEL) return true;
        if (speed != null) speed.addMovement(e);
        boolean pans = nativeTouch(id, action, e.getX(), e.getY(), slop, stopped);
        if (action == MotionEvent.ACTION_MOVE) panning = pans;
        if (action == MotionEvent.ACTION_UP && panning && speed != null) {
            ViewConfiguration configuration = ViewConfiguration.get(getContext());
            speed.computeCurrentVelocity(1000, configuration.getScaledMaximumFlingVelocity());
            float vx = speed.getXVelocity(), vy = speed.getYVelocity();
            if (Math.hypot(vx, vy) > configuration.getScaledMinimumFlingVelocity()) {
                flungX = flungY = 0;
                fling.fling(0, 0, (int) -vx, (int) -vy, Integer.MIN_VALUE, Integer.MAX_VALUE, Integer.MIN_VALUE, Integer.MAX_VALUE);
                postOnAnimation(flinging);
            }
        }
        if (action != MotionEvent.ACTION_MOVE) panning = false;
        return true;
    }

    /** A display frame's step of the fling: what it panned stops it at its end. */
    private void flinging() {
        if (!fling.computeScrollOffset()) return;
        int x = fling.getCurrX(), y = fling.getCurrY();
        boolean moved = nativePan(id, downX, downY, x - flungX, y - flungY);
        flungX = x;
        flungY = y;
        if (moved) postOnAnimation(flinging);
        else fling.forceFinished(true);
    }

    private void longPress() {
        nativeLongPress(id, downX, downY);
    }

    /** A key from a keyboard that isn't on screen (a physical one, or `adb shell input`), to the field, as the input method's own. */
    @Override public boolean dispatchKeyEvent(KeyEvent event) {
        InputConnection connection = input.getLastInputConnection();
        if (connection instanceof InputConnectionAdaptor && ((InputConnectionAdaptor) connection).handleKeyEvent(event)) return true;
        return super.dispatchKeyEvent(event);
    }

    /** Cut, Copy, Paste and Select All for the field, as Android's floating text toolbar. */
    private void showMenu() {
        if (menu != null) menu.finish();
        menu = startActionMode(new ActionMode.Callback2() {
            @Override public boolean onCreateActionMode(ActionMode mode, Menu m) {
                m.add(Menu.NONE, android.R.id.cut, 0, android.R.string.cut);
                m.add(Menu.NONE, android.R.id.copy, 1, android.R.string.copy);
                m.add(Menu.NONE, android.R.id.paste, 2, android.R.string.paste);
                m.add(Menu.NONE, android.R.id.selectAll, 3, android.R.string.selectAll);
                return true;
            }
            @Override public boolean onPrepareActionMode(ActionMode mode, Menu m) { return false; }
            @Override public boolean onActionItemClicked(ActionMode mode, MenuItem item) {
                InputConnection connection = input.getLastInputConnection();
                if (connection != null) connection.performContextMenuAction(item.getItemId());
                if (item.getItemId() != android.R.id.selectAll) mode.finish();
                return true;
            }
            @Override public void onDestroyActionMode(ActionMode mode) { menu = null; }
            @Override public void onGetContentRect(ActionMode mode, View view, Rect outRect) {
                Rect rect = input.clientRect();
                if (rect != null) outRect.set(rect);
            }
        }, ActionMode.TYPE_FLOATING);
    }

    @Override public boolean onCheckIsTextEditor() {
        return input.isTextEditor();
    }

    @Override public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        return input.createInputConnection(this, outAttrs);
    }

    @Override public void onProvideAutofillVirtualStructure(ViewStructure structure, int flags) {
        input.onProvideAutofillVirtualStructure(structure, flags);
    }

    @Override public void autofill(SparseArray<AutofillValue> values) {
        input.autofill(values);
    }

    /** Called by AccessKit's adapter, on any thread: do what TalkBack asked, on this one. */
    public void requestAccessibilityDrain() {
        post(() -> nativeAccessibilityDrain(id));
    }

    // TextInputPlugin.Framework
    @Override public void updateEditingState(int client, String text, int selectionStart, int selectionEnd, int composingStart, int composingEnd) {
        nativeEditingState(id, client, text, selectionStart, selectionEnd, composingStart, composingEnd);
    }

    @Override public void performAction(int client, String action) {
        nativeAction(id, client, action);
    }

    @Override public void autofill(int client, String value) {
        nativeAutofill(id, client, value);
    }

    @Override public String fields() {
        return nativeFields(id);
    }

    private native void nativeSurface(long id, android.view.Surface surface, int width, int height);
    private native void nativeSurfaceDestroyed(long id);
    private native boolean nativeBack(long id);
    private native void nativeActive(long id, boolean active);
    private native boolean nativeTouch(long id, int action, float x, float y, float slop, boolean stopped);
    private native boolean nativePan(long id, float x, float y, float dx, float dy);
    private native void nativeLongPress(long id, float x, float y);

    private native void nativePicked(long id, long node, String value);

    private native void nativeInsets(long id, int left, int top, int right, int bottom);
    private native void nativeEditingState(long id, int field, String text, int base, int extent, int composingStart, int composingEnd);
    private native void nativeAction(long id, int field, String action);
    private native String nativeFields(long id);
    private native void nativeAutofill(long id, int field, String value);
    private native void nativeAccessibility(long id);
    private native void nativeAccessibilityDrain(long id);
}
