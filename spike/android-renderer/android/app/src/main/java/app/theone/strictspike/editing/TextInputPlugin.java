// From Flutter's Android embedder (BSD-3-Clause, LICENSE-FLUTTER beside this file),
// engine/src/flutter/shell/platform/android/io/flutter/plugin/editing/TextInputPlugin.java at flutter/flutter
// 4d19bda07ae859c5d3a205e3a61619ff485a1182.
// Changed: its framework is the renderer (`Framework`), not Flutter's TextInputChannel; a field's configuration is
// the strict protocol's input props (type, autocomplete, enterkeyhint, autocapitalize, spellcheck) as Chrome reads
// them, not Flutter's TextInputConfiguration; every field on the screen is in the autofill structure (as a browser's
// form is), not only those in the focused field's AutofillGroup; no platform views or Scribe.

// Copyright 2013 The Flutter Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package app.theone.strictspike.editing;

import android.content.Context;
import android.graphics.Rect;
import android.text.InputType;
import android.util.Log;
import android.util.SparseArray;
import android.view.View;
import android.view.ViewStructure;
import android.view.autofill.AutofillId;
import android.view.autofill.AutofillManager;
import android.view.autofill.AutofillValue;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import org.json.JSONArray;
import org.json.JSONObject;

public class TextInputPlugin implements ListenableEditingState.EditingStateWatcher, InputConnectionAdaptor.Client {
  private static final String TAG = "TextInputPlugin";

  /** What the plugin tells the renderer: Flutter's TextInputChannel calls. */
  public interface Framework {
    void updateEditingState(int client, String text, int selectionStart, int selectionEnd, int composingStart, int composingEnd);

    void performAction(int client, String action);

    void autofill(int client, String value);

    /** Every field, as JSON: id, type, autocomplete, label, text, bounds. */
    String fields();
  }

  private final View mView;
  private final InputMethodManager mImm;
  private final AutofillManager afm;
  private final Framework framework;
  private JSONObject configuration;
  private int client = -1;
  private ListenableEditingState mEditable;
  private TextEditState mLastKnownFrameworkTextEditingState;
  private boolean mRestartInputPending;
  private InputConnection lastInputConnection;
  private Rect lastClientRect;

  public TextInputPlugin(View view, Framework framework) {
    mView = view;
    this.framework = framework;
    mEditable = new ListenableEditingState(null, mView);
    mImm = (InputMethodManager) view.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
    afm = view.getContext().getSystemService(AutofillManager.class);
  }

  // The strict protocol's input props as Android's input type, as Chrome reads an <input>'s, in the shape of Flutter's
  // inputTypeFromTextInputType.
  private static int inputTypeFrom(JSONObject config) {
    String type = config.optString("type", "text");
    if (type.equals("number")) return InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_FLAG_DECIMAL;
    if (type.equals("tel")) return InputType.TYPE_CLASS_PHONE;
    boolean obscureText = type.equals("password");
    boolean spellcheck = config.optBoolean("spellcheck", true);
    int textType = InputType.TYPE_CLASS_TEXT;
    boolean autocorrect = spellcheck;
    if (type.equals("email")) {
      textType |= InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS;
      autocorrect = false;
    } else if (type.equals("url")) {
      textType |= InputType.TYPE_TEXT_VARIATION_URI;
      autocorrect = false;
    } else if ("name".equals(config.optString("autocomplete"))) {
      textType |= InputType.TYPE_TEXT_VARIATION_PERSON_NAME;
    }

    if (obscureText) {
      textType |= InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
      textType |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
    } else {
      if (autocorrect) textType |= InputType.TYPE_TEXT_FLAG_AUTO_CORRECT;
      if (!spellcheck) {
        textType |= InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
      }
    }

    String capitalize = config.optString("autocapitalize", obscureText || type.equals("email") || type.equals("url") ? "none" : "sentences");
    if (capitalize.equals("characters")) {
      textType |= InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS;
    } else if (capitalize.equals("words")) {
      textType |= InputType.TYPE_TEXT_FLAG_CAP_WORDS;
    } else if (capitalize.equals("sentences") || capitalize.equals("on")) {
      textType |= InputType.TYPE_TEXT_FLAG_CAP_SENTENCES;
    }

    return textType;
  }

  private static int actionFrom(JSONObject config) {
    switch (config.optString("enterkeyhint", "done")) {
      case "next": return EditorInfo.IME_ACTION_NEXT;
      case "go": return EditorInfo.IME_ACTION_GO;
      case "search": return EditorInfo.IME_ACTION_SEARCH;
      case "send": return EditorInfo.IME_ACTION_SEND;
      case "previous": return EditorInfo.IME_ACTION_PREVIOUS;
      case "enter": return EditorInfo.IME_ACTION_NONE;
      default: return EditorInfo.IME_ACTION_DONE;
    }
  }

  /** An HTML autocomplete token as Android's autofill hints. */
  private static String[] hintsFrom(String autocomplete) {
    switch (autocomplete == null ? "" : autocomplete) {
      case "username": return new String[] {View.AUTOFILL_HINT_USERNAME};
      case "current-password": return new String[] {View.AUTOFILL_HINT_PASSWORD};
      case "new-password": return new String[] {"newPassword"};
      case "email": return new String[] {View.AUTOFILL_HINT_EMAIL_ADDRESS};
      case "name": return new String[] {View.AUTOFILL_HINT_NAME};
      case "tel": return new String[] {View.AUTOFILL_HINT_PHONE};
      case "postal-code": return new String[] {View.AUTOFILL_HINT_POSTAL_CODE};
      case "one-time-code": return new String[] {"smsOTPCode"};
      default: return new String[0];
    }
  }

  public InputConnection createInputConnection(View view, EditorInfo outAttrs) {
    if (client < 0) {
      lastInputConnection = null;
      return null;
    }

    outAttrs.inputType = inputTypeFrom(configuration);
    outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN;
    int enterAction = actionFrom(configuration);
    outAttrs.imeOptions |= enterAction;
    outAttrs.hintText = configuration.optString("label", null);
    InputConnectionAdaptor connection = new InputConnectionAdaptor(view, client, this, mEditable, outAttrs);
    outAttrs.initialSelStart = mEditable.getSelectionStart();
    outAttrs.initialSelEnd = mEditable.getSelectionEnd();

    lastInputConnection = connection;
    return lastInputConnection;
  }

  public InputConnection getLastInputConnection() {
    return lastInputConnection;
  }

  public boolean isTextEditor() {
    return client >= 0;
  }

  public void showTextInput(View view) {
    view.requestFocus();
    mImm.showSoftInput(view, 0);
  }

  public void hideTextInput(View view) {
    notifyViewExited();
    mImm.hideSoftInputFromWindow(view.getApplicationWindowToken(), 0);
  }

  /** Flutter's setTextInputClient, then setTextInputEditingState. */
  public void setTextInputClient(int id, JSONObject configuration, TextEditState state) {
    notifyViewExited();
    this.configuration = configuration;
    client = id;

    mEditable.removeEditingStateListener(this);
    mEditable = new ListenableEditingState(state, mView);
    mLastKnownFrameworkTextEditingState = state;
    JSONArray bounds = configuration.optJSONArray("bounds");
    lastClientRect = bounds == null ? null : new Rect(bounds.optInt(0), bounds.optInt(1), bounds.optInt(0) + bounds.optInt(2), bounds.optInt(1) + bounds.optInt(3));

    mRestartInputPending = true;
    mEditable.addEditingStateListener(this);
    mImm.restartInput(mView);
    mRestartInputPending = false;
    notifyViewEntered();
  }

  private static boolean composingChanged(TextEditState before, TextEditState after) {
    final int composingRegionLength = before.composingEnd - before.composingStart;
    if (composingRegionLength != after.composingEnd - after.composingStart) {
      return true;
    }
    for (int index = 0; index < composingRegionLength; index++) {
      if (before.text.charAt(index + before.composingStart)
          != after.text.charAt(index + after.composingStart)) {
        return true;
      }
    }
    return false;
  }

  /** Flutter's setTextInputEditingState: the framework changed the field (a tap moved the caret). */
  public void setTextInputEditingState(View view, TextEditState state) {
    if (!mRestartInputPending
        && mLastKnownFrameworkTextEditingState != null
        && mLastKnownFrameworkTextEditingState.hasComposing()) {
      mRestartInputPending = composingChanged(mLastKnownFrameworkTextEditingState, state);
      if (mRestartInputPending) {
        Log.i(TAG, "Composing region changed by the framework. Restarting the input method.");
      }
    }

    mLastKnownFrameworkTextEditingState = state;
    mEditable.setEditingState(state);

    if (mRestartInputPending) {
      mImm.restartInput(view);
      mRestartInputPending = false;
    }
  }

  public void clearTextInputClient() {
    mEditable.removeEditingStateListener(this);
    notifyViewExited();
    configuration = null;
    client = -1;
    lastClientRect = null;
    mImm.restartInput(mView);
  }

  @Override
  public void didChangeEditingState(
      boolean textChanged, boolean selectionChanged, boolean composingRegionChanged) {
    if (textChanged) {
      notifyValueChanged(mEditable.toString());
    }

    final int selectionStart = mEditable.getSelectionStart();
    final int selectionEnd = mEditable.getSelectionEnd();
    final int composingStart = mEditable.getComposingStart();
    final int composingEnd = mEditable.getComposingEnd();

    final boolean skipFrameworkUpdate =
        mLastKnownFrameworkTextEditingState == null
            || (mEditable.toString().equals(mLastKnownFrameworkTextEditingState.text)
                && selectionStart == mLastKnownFrameworkTextEditingState.selectionStart
                && selectionEnd == mLastKnownFrameworkTextEditingState.selectionEnd
                && composingStart == mLastKnownFrameworkTextEditingState.composingStart
                && composingEnd == mLastKnownFrameworkTextEditingState.composingEnd);
    if (!skipFrameworkUpdate) {
      framework.updateEditingState(client, mEditable.toString(), selectionStart, selectionEnd, composingStart, composingEnd);
      mLastKnownFrameworkTextEditingState =
          new TextEditState(mEditable.toString(), selectionStart, selectionEnd, composingStart, composingEnd);
    }
  }

  @Override
  public void performEditorAction(int clientId, int actionCode) {
    String action;
    switch (actionCode) {
      case EditorInfo.IME_ACTION_NEXT: action = "next"; break;
      case EditorInfo.IME_ACTION_GO: action = "go"; break;
      case EditorInfo.IME_ACTION_SEARCH: action = "search"; break;
      case EditorInfo.IME_ACTION_SEND: action = "send"; break;
      case EditorInfo.IME_ACTION_PREVIOUS: action = "previous"; break;
      case EditorInfo.IME_ACTION_NONE: action = "enter"; break;
      default: action = "done";
    }
    if (afm != null && (action.equals("go") || action.equals("done") || action.equals("send"))) {
      // Flutter's TextInput.finishAutofillContext: the form is done, so a password manager may offer to save.
      afm.commit();
    }
    framework.performAction(clientId, action);
  }

  // -------- Autofill --------

  private void notifyViewEntered() {
    if (afm == null || client < 0 || lastClientRect == null) {
      return;
    }
    final int[] offset = new int[2];
    mView.getLocationOnScreen(offset);
    Rect rect = new Rect(lastClientRect);
    rect.offset(offset[0], offset[1]);
    afm.notifyViewEntered(mView, client, rect);
  }

  private void notifyViewExited() {
    if (afm == null || client < 0) {
      return;
    }
    afm.notifyViewExited(mView, client);
  }

  private void notifyValueChanged(String newValue) {
    if (afm == null || client < 0) {
      return;
    }
    afm.notifyValueChanged(mView, client, AutofillValue.forText(newValue));
  }

  public void onProvideAutofillVirtualStructure(ViewStructure structure, int flags) {
    JSONArray fields;
    try {
      fields = new JSONArray(framework.fields());
    } catch (Exception e) {
      return;
    }
    final AutofillId parentId = structure.getAutofillId();
    for (int i = 0; i < fields.length(); i++) {
      JSONObject field = fields.optJSONObject(i);
      final int autofillId = field.optInt("id");
      String[] hints = hintsFrom(field.optString("autocomplete", null));

      structure.addChildCount(1);
      final ViewStructure child = structure.newChild(i);
      child.setAutofillId(parentId, autofillId);
      if (hints.length > 0) {
        child.setAutofillHints(hints);
      }
      child.setAutofillType(View.AUTOFILL_TYPE_TEXT);
      child.setVisibility(View.VISIBLE);
      child.setClassName("android.widget.EditText");
      child.setInputType(inputTypeFrom(field));
      String label = field.optString("label", null);
      if (label != null) {
        child.setHint(label);
      }
      JSONArray b = field.optJSONArray("bounds");
      if (b != null) {
        child.setDimens(b.optInt(0), b.optInt(1), 0, 0, Math.max(1, b.optInt(2)), Math.max(1, b.optInt(3)));
      }
      if (autofillId == client) {
        child.setAutofillValue(AutofillValue.forText(mEditable));
        child.setFocused(true);
      } else {
        child.setAutofillValue(AutofillValue.forText(field.optString("text")));
      }
    }
  }

  public void autofill(SparseArray<AutofillValue> values) {
    for (int i = 0; i < values.size(); i++) {
      int virtualId = values.keyAt(i);
      final String value = values.valueAt(i).getTextValue().toString();
      if (virtualId == client) {
        mEditable.setEditingState(new TextEditState(value, value.length(), value.length(), -1, -1));
      } else {
        framework.autofill(virtualId, value);
      }
    }
  }
}
