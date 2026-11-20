// From Flutter's Android embedder (BSD-3-Clause, LICENSE-FLUTTER beside this file),
// engine/src/flutter/shell/platform/android/io/flutter/plugin/editing/TextInputPlugin.java at flutter/flutter
// 4d19bda07ae859c5d3a205e3a61619ff485a1182.
// Changed: its framework is the renderer (`Framework`), not Flutter's TextInputChannel, and Flutter's
// TextInputConfiguration is read from its JSON as it comes; every field on the screen is in the autofill structure
// (as a browser's form is), each with its configuration, label and place (`Framework.fields`), not only those in the
// focused field's AutofillGroup; no platform views or Scribe.

// Copyright 2013 The Flutter Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package app.theone.renderer.editing;

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

    /** Every field, as JSON: its props as `field`, its text, and its rect in CSS pixels. */
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

  // Flutter's inputTypeFromTextInputType, reading its TextInputConfiguration.
  private static int inputTypeFrom(JSONObject config) {
    JSONObject inputType = config.optJSONObject("inputType");
    String type = inputType == null ? "TextInputType.text" : inputType.optString("name", "TextInputType.text");
    boolean obscureText = config.optBoolean("obscureText");
    boolean autocorrect = config.optBoolean("autocorrect", true);
    boolean enableSuggestions = config.optBoolean("enableSuggestions", true);
    String textCapitalization = config.optString("textCapitalization", "TextCapitalization.none");
    if (type.equals("TextInputType.datetime")) {
      return InputType.TYPE_CLASS_DATETIME;
    } else if (type.equals("TextInputType.number")) {
      int textType = InputType.TYPE_CLASS_NUMBER;
      if (inputType.optBoolean("signed")) {
        textType |= InputType.TYPE_NUMBER_FLAG_SIGNED;
      }
      if (inputType.optBoolean("decimal")) {
        textType |= InputType.TYPE_NUMBER_FLAG_DECIMAL;
      }
      return textType;
    } else if (type.equals("TextInputType.phone")) {
      return InputType.TYPE_CLASS_PHONE;
    } else if (type.equals("TextInputType.none")) {
      return InputType.TYPE_NULL;
    }

    int textType = InputType.TYPE_CLASS_TEXT;
    if (type.equals("TextInputType.multiline")) {
      textType |= InputType.TYPE_TEXT_FLAG_MULTI_LINE;
    } else if (type.equals("TextInputType.emailAddress")) {
      textType |= InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS;
    } else if (type.equals("TextInputType.url")) {
      textType |= InputType.TYPE_TEXT_VARIATION_URI;
    } else if (type.equals("TextInputType.visiblePassword")) {
      textType |= InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD;
    } else if (type.equals("TextInputType.name")) {
      textType |= InputType.TYPE_TEXT_VARIATION_PERSON_NAME;
    }

    if (obscureText) {
      // Note: both required. Some devices ignore TYPE_TEXT_FLAG_NO_SUGGESTIONS.
      textType |= InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
      textType |= InputType.TYPE_TEXT_VARIATION_PASSWORD;
    } else {
      if (autocorrect) textType |= InputType.TYPE_TEXT_FLAG_AUTO_CORRECT;
      if (!enableSuggestions) {
        // Note: both required. Some devices ignore TYPE_TEXT_FLAG_NO_SUGGESTIONS.
        textType |= InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
        textType |= InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD;
      }
    }

    if (textCapitalization.equals("TextCapitalization.characters")) {
      textType |= InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS;
    } else if (textCapitalization.equals("TextCapitalization.words")) {
      textType |= InputType.TYPE_TEXT_FLAG_CAP_WORDS;
    } else if (textCapitalization.equals("TextCapitalization.sentences")) {
      textType |= InputType.TYPE_TEXT_FLAG_CAP_SENTENCES;
    }

    return textType;
  }

  // Flutter's inputActionFromTextInputAction.
  private static int actionFrom(JSONObject config) {
    switch (config.optString("inputAction", "TextInputAction.done")) {
      case "TextInputAction.newline": return EditorInfo.IME_ACTION_NONE;
      case "TextInputAction.none": return EditorInfo.IME_ACTION_NONE;
      case "TextInputAction.unspecified": return EditorInfo.IME_ACTION_UNSPECIFIED;
      case "TextInputAction.go": return EditorInfo.IME_ACTION_GO;
      case "TextInputAction.search": return EditorInfo.IME_ACTION_SEARCH;
      case "TextInputAction.send": return EditorInfo.IME_ACTION_SEND;
      case "TextInputAction.next": return EditorInfo.IME_ACTION_NEXT;
      case "TextInputAction.previous": return EditorInfo.IME_ACTION_PREVIOUS;
      default: return EditorInfo.IME_ACTION_DONE;
    }
  }

  /** A configuration's autofill hints, as TextInputChannel's translateAutofillHint has Flutter's as Android's. */
  private static String[] hintsFrom(JSONObject config) {
    JSONObject autofill = config.optJSONObject("autofill");
    JSONArray hints = autofill == null ? null : autofill.optJSONArray("hints");
    if (hints == null) return new String[0];
    String[] android = new String[hints.length()];
    for (int i = 0; i < hints.length(); i++) {
      String hint = hints.optString(i);
      switch (hint) {
        case "username": android[i] = View.AUTOFILL_HINT_USERNAME; break;
        case "password": android[i] = View.AUTOFILL_HINT_PASSWORD; break;
        case "email": android[i] = View.AUTOFILL_HINT_EMAIL_ADDRESS; break;
        case "name": android[i] = "personName"; break;
        case "telephoneNumber": android[i] = View.AUTOFILL_HINT_PHONE; break;
        case "postalCode": android[i] = View.AUTOFILL_HINT_POSTAL_CODE; break;
        case "oneTimeCode": android[i] = "smsOTPCode"; break;
        default: android[i] = hint;
      }
    }
    return android;
  }

  /** A field's place in the view, in its pixels, from its `rect` in CSS pixels. */
  public Rect rectOf(JSONObject field) {
    JSONObject rect = field == null ? null : field.optJSONObject("rect");
    if (rect == null) return null;
    float density = mView.getResources().getDisplayMetrics().density;
    int x = Math.round((float) rect.optDouble("x") * density), y = Math.round((float) rect.optDouble("y") * density);
    return new Rect(x, y, x + Math.round((float) rect.optDouble("width") * density), y + Math.round((float) rect.optDouble("height") * density));
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

  /** Flutter's setTextInputClient: its editing state comes next (`setTextInputEditingState`). */
  public void setTextInputClient(int id, JSONObject configuration) {
    notifyViewExited();
    this.configuration = configuration;
    client = id;

    mEditable.removeEditingStateListener(this);
    mEditable = new ListenableEditingState(null, mView);
    mLastKnownFrameworkTextEditingState = null;
    lastClientRect = null;

    mRestartInputPending = true;
    mEditable.addEditingStateListener(this);
  }

  /** Flutter's saveEditableSizeAndTransform, for an affine `matrix`: where the field is, which autofill shows at. */
  public void setEditableSizeAndTransform(double width, double height, double[] matrix) {
    double minX = Double.MAX_VALUE, minY = Double.MAX_VALUE, maxX = -Double.MAX_VALUE, maxY = -Double.MAX_VALUE;
    for (double[] corner : new double[][] {{0, 0}, {width, 0}, {width, height}, {0, height}}) {
      double x = matrix[0] * corner[0] + matrix[4] * corner[1] + matrix[12];
      double y = matrix[1] * corner[0] + matrix[5] * corner[1] + matrix[13];
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
    }
    float density = mView.getResources().getDisplayMetrics().density;
    boolean first = lastClientRect == null;
    lastClientRect = new Rect((int) (minX * density), (int) (minY * density), (int) Math.ceil(maxX * density), (int) Math.ceil(maxY * density));
    if (first) notifyViewEntered();
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
      case EditorInfo.IME_ACTION_NEXT: action = "TextInputAction.next"; break;
      case EditorInfo.IME_ACTION_GO: action = "TextInputAction.go"; break;
      case EditorInfo.IME_ACTION_SEARCH: action = "TextInputAction.search"; break;
      case EditorInfo.IME_ACTION_SEND: action = "TextInputAction.send"; break;
      case EditorInfo.IME_ACTION_PREVIOUS: action = "TextInputAction.previous"; break;
      case EditorInfo.IME_ACTION_NONE: action = "TextInputAction.newline"; break;
      default: action = "TextInputAction.done";
    }
    if (afm != null && (actionCode == EditorInfo.IME_ACTION_GO || actionCode == EditorInfo.IME_ACTION_DONE || actionCode == EditorInfo.IME_ACTION_SEND)) {
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
      JSONObject config = field.optJSONObject("configuration");
      final int autofillId = field.optInt("id");
      String[] hints = hintsFrom(config);

      structure.addChildCount(1);
      final ViewStructure child = structure.newChild(i);
      child.setAutofillId(parentId, autofillId);
      if (hints.length > 0) {
        child.setAutofillHints(hints);
      }
      child.setAutofillType(View.AUTOFILL_TYPE_TEXT);
      child.setVisibility(View.VISIBLE);
      child.setClassName("android.widget.EditText");
      child.setInputType(inputTypeFrom(config));
      String label = field.optString("label", null);
      if (label != null) {
        child.setHint(label);
      }
      Rect rect = rectOf(field);
      if (rect != null) {
        child.setDimens(rect.left, rect.top, 0, 0, Math.max(1, rect.width()), Math.max(1, rect.height()));
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
