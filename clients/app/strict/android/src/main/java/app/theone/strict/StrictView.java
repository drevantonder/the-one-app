package app.theone.strict;

import android.content.Context;
import android.graphics.Rect;
import android.util.Log;
import android.util.SparseArray;
import android.view.ActionMode;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewStructure;
import android.view.autofill.AutofillValue;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import app.theone.strict.editing.InputConnectionAdaptor;
import app.theone.strict.editing.TextEditState;
import app.theone.strict.editing.TextInputPlugin;
import org.json.JSONArray;
import org.json.JSONObject;

/**
 * A strict app's window: a surface the strict crate draws on (`strict::platform::android`), by its window's id. It
 * passes on what a finger does, bridges the keyboard (TextInputPlugin, from Flutter's embedder) and autofill, and does
 * what Rust asks of it (`command`): Flutter's text input method calls (`TextInput.setClient`, `setEditingState`,
 * `show`…), or showing the Cut, Copy and Paste menu.
 */
public final class StrictView extends SurfaceView implements SurfaceHolder.Callback, TextInputPlugin.Framework {
    private static final String TAG = "strict";
    private final long id;
    private final TextInputPlugin input;
    private float downX, downY;
    private final Runnable longPress = this::longPress;
    private ActionMode menu;

    StrictView(Context context, long id) {
        super(context);
        this.id = id;
        input = new TextInputPlugin(this, this);
        getHolder().addCallback(this);
        setFocusable(true);
        setFocusableInTouchMode(true);
        setImportantForAutofill(IMPORTANT_FOR_AUTOFILL_YES);
        // Once Rust has let go of the window it's opening.
        post(() -> nativeAccessibility(id));
    }

    /** Back: the window's app closes. */
    void back() {
        nativeBack(id);
    }

    /** What Rust asks, done on the UI thread once Rust has returned. */
    public void command(String command) {
        post(() -> apply(command));
    }

    private void apply(String command) {
        try {
            JSONObject c = new JSONObject(command);
            if (c.optBoolean("menu")) showMenu();
            textInput(c.optString("method"), c.opt("args"));
        } catch (Exception e) {
            Log.e(TAG, "command " + command, e);
        }
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
        if (action == MotionEvent.ACTION_DOWN) {
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
        nativeTouch(id, action, e.getX(), e.getY(), slop);
        return true;
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
    private native void nativeBack(long id);
    private native void nativeActive(long id, boolean active);
    private native void nativeTouch(long id, int action, float x, float y, float slop);
    private native void nativeLongPress(long id, float x, float y);
    private native void nativeEditingState(long id, int field, String text, int base, int extent, int composingStart, int composingEnd);
    private native void nativeAction(long id, int field, String action);
    private native String nativeFields(long id);
    private native void nativeAutofill(long id, int field, String value);
    private native void nativeAccessibility(long id);
    private native void nativeAccessibilityDrain(long id);
}
