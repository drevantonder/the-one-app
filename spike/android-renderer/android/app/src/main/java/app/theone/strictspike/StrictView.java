package app.theone.strictspike;

import android.content.Context;
import android.content.res.Configuration;
import android.graphics.Rect;
import android.os.SystemClock;
import android.util.Log;
import android.util.SparseArray;
import android.view.ActionMode;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewStructure;
import android.view.WindowInsets;
import android.view.autofill.AutofillValue;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import app.theone.strictspike.editing.TextEditState;
import app.theone.strictspike.editing.TextInputPlugin;
import org.json.JSONArray;
import org.json.JSONObject;

/** A strict screen, drawn by the renderer (libstrict_android) on this view's surface. The view passes the renderer
 * what a finger does, bridges the keyboard (TextInputPlugin, from Flutter's embedder) and autofill to it, and does
 * what it answers: focus a field, move its caret, blur, or show the Cut/Copy/Paste menu. */
public class StrictView extends SurfaceView implements SurfaceHolder.Callback, TextInputPlugin.Framework {
    static {
        System.loadLibrary("strict_android");
    }

    private static final String TAG = "StrictSpike";
    private final long handle;
    private final TextInputPlugin input;
    private final Runnable onDrawn;
    private boolean drawn;
    private float downX, downY;
    private final Runnable longPress = this::longPress;
    private ActionMode menu;
    private Rect fieldRect = new Rect();

    public StrictView(Context context, String page, Runnable onDrawn) {
        super(context);
        this.onDrawn = onDrawn;
        boolean dark = (context.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
        handle = nativeCreate(context.getAssets(), page, context.getCodeCacheDir().getAbsolutePath(), context.getResources().getDisplayMetrics().density, dark);
        input = new TextInputPlugin(this, this);
        getHolder().addCallback(this);
        setFocusable(true);
        setFocusableInTouchMode(true);
        setImportantForAutofill(IMPORTANT_FOR_AUTOFILL_YES);
        nativeAccessibility(handle, this);
    }

    /** For checks: does to the focused field what an input method would, through the same InputConnection
     * (`adb shell am broadcast -a app.theone.strictspike.IME --es op compose --es text 日本`). */
    public void ime(android.content.Intent intent) {
        InputConnection c = input.getLastInputConnection();
        if (c == null) return;
        String text = intent.getStringExtra("text");
        int a = intent.getIntExtra("a", 0), b = intent.getIntExtra("b", 0);
        switch (String.valueOf(intent.getStringExtra("op"))) {
            case "compose": c.setComposingText(text, 1); break;
            case "commit": c.commitText(text, 1); break;
            case "region": c.setComposingRegion(a, b); break;
            case "finish": c.finishComposingText(); break;
            case "delete": c.deleteSurroundingText(a, b); break;
            case "select": c.setSelection(a, b); break;
            case "paste": c.performContextMenuAction(android.R.id.paste); break;
        }
    }

    public void destroy() {
        nativeDestroy(handle);
    }

    @Override public void surfaceCreated(SurfaceHolder holder) {}

    @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        nativeSurface(handle, holder.getSurface(), width, height);
        if (!drawn) {
            drawn = true;
            onDrawn.run();
        }
    }

    @Override public void surfaceDestroyed(SurfaceHolder holder) {
        nativeSurfaceDestroyed(handle);
    }

    @Override public WindowInsets onApplyWindowInsets(WindowInsets insets) {
        int top = insets.getInsets(WindowInsets.Type.systemBars()).top;
        int bottom = Math.max(insets.getInsets(WindowInsets.Type.systemBars()).bottom, insets.getInsets(WindowInsets.Type.ime()).bottom);
        nativeInsets(handle, top, bottom);
        return insets;
    }

    @Override public boolean onTouchEvent(MotionEvent e) {
        int action = e.getActionMasked();
        if (action == MotionEvent.ACTION_DOWN) {
            downX = e.getX();
            downY = e.getY();
            postDelayed(longPress, ViewConfiguration.getLongPressTimeout());
        } else if (action == MotionEvent.ACTION_MOVE) {
            float slop = ViewConfiguration.get(getContext()).getScaledTouchSlop();
            if (Math.abs(e.getX() - downX) > slop || Math.abs(e.getY() - downY) > slop) removeCallbacks(longPress);
        } else {
            removeCallbacks(longPress);
        }
        if (action > 3) return true;
        String command = nativeTouch(handle, action, e.getX(), e.getY());
        if (action == MotionEvent.ACTION_UP) {
            Log.i(TAG, "strict tap to painted " + (SystemClock.uptimeMillis() - e.getEventTime()) + " ms");
        }
        apply(command);
        return true;
    }

    private void longPress() {
        apply(nativeLongPress(handle, downX, downY));
    }

    /** Does what the renderer answered. */
    private void apply(String command) {
        if (command == null) return;
        try {
            JSONObject c = new JSONObject(command);
            if (c.has("focus")) {
                JSONObject focus = c.getJSONObject("focus");
                JSONArray b = focus.optJSONArray("bounds");
                if (b != null) fieldRect = new Rect(b.optInt(0), b.optInt(1), b.optInt(0) + b.optInt(2), b.optInt(1) + b.optInt(3));
                input.setTextInputClient(focus.getInt("id"), focus, TextEditState.fromJson(c.getJSONObject("state")));
                input.showTextInput(this);
            } else if (c.has("state")) {
                input.setTextInputEditingState(this, TextEditState.fromJson(c.getJSONObject("state")));
            }
            if (c.optBoolean("blur")) {
                if (menu != null) menu.finish();
                input.hideTextInput(this);
                input.clearTextInputClient();
            }
            if (c.optBoolean("menu")) showMenu();
        } catch (Exception e) {
            Log.e(TAG, "command " + command, e);
        }
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
            @Override public void onGetContentRect(ActionMode mode, View view, Rect outRect) { outRect.set(fieldRect); }
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

    /** Called by the renderer's AccessKit adapter, on any thread: do what the screen reader asked, on this one. */
    public void requestAccessibilityDrain() {
        post(() -> nativeAccessibilityDrain(handle));
    }

    // TextInputPlugin.Framework
    @Override public void updateEditingState(int client, String text, int selectionStart, int selectionEnd, int composingStart, int composingEnd) {
        nativeEditingState(handle, client, text, selectionStart, selectionEnd, composingStart, composingEnd);
    }

    @Override public void performAction(int client, String action) {
        apply(nativeAction(handle, client, action));
    }

    @Override public void autofill(int client, String value) {
        nativeAutofill(handle, client, value);
    }

    @Override public String fields() {
        return nativeFields(handle);
    }

    private static native long nativeCreate(android.content.res.AssetManager assets, String page, String cacheDir, float density, boolean dark);
    private static native void nativeSurface(long handle, android.view.Surface surface, int width, int height);
    private static native void nativeSurfaceDestroyed(long handle);
    private static native void nativeDestroy(long handle);
    private static native void nativeInsets(long handle, int top, int bottom);
    private static native String nativeTouch(long handle, int action, float x, float y);
    private static native String nativeLongPress(long handle, float x, float y);
    private static native void nativeEditingState(long handle, int id, String text, int base, int extent, int composingStart, int composingEnd);
    private static native String nativeAction(long handle, int id, String action);
    private static native String nativeFields(long handle);
    private static native void nativeAutofill(long handle, int id, String value);
    private static native void nativeAccessibility(long handle, View view);
    private static native void nativeAccessibilityDrain(long handle);
}
