// After Flutter's Android embedder (BSD-3-Clause, LICENSE-FLUTTER beside this file): TextInputChannel.TextEditState,
// engine/src/flutter/shell/platform/android/io/flutter/embedding/engine/systemchannels/TextInputChannel.java at
// flutter/flutter 4d19bda07ae859c5d3a205e3a61619ff485a1182, its fields and checks, written again here (that file
// wasn't copied); read from the renderer's `Editing` (JSON) rather than Flutter's method channel.

// Copyright 2013 The Flutter Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package app.theone.strict.editing;

import org.json.JSONObject;

/** State of an on-going text editing session. */
public class TextEditState {
  public final String text;
  public final int selectionStart;
  public final int selectionEnd;
  public final int composingStart;
  public final int composingEnd;

  public TextEditState(String text, int selectionStart, int selectionEnd, int composingStart, int composingEnd) {
    if ((selectionStart != -1 || selectionEnd != -1)
        && (selectionStart < 0 || selectionEnd < 0)) {
      throw new IndexOutOfBoundsException(
          "invalid selection: (" + selectionStart + ", " + selectionEnd + ")");
    }
    if ((composingStart != -1 || composingEnd != -1)
        && (composingStart < 0 || composingStart > composingEnd)) {
      throw new IndexOutOfBoundsException(
          "invalid composing range: (" + composingStart + ", " + composingEnd + ")");
    }
    if (composingEnd > text.length()) {
      throw new IndexOutOfBoundsException("invalid composing start: " + composingStart);
    }
    if (selectionStart > text.length()) {
      throw new IndexOutOfBoundsException("invalid selection start: " + selectionStart);
    }
    if (selectionEnd > text.length()) {
      throw new IndexOutOfBoundsException("invalid selection end: " + selectionEnd);
    }
    this.text = text;
    this.selectionStart = selectionStart;
    this.selectionEnd = selectionEnd;
    this.composingStart = composingStart;
    this.composingEnd = composingEnd;
  }

  /** Flutter's TextEditState.fromJson: its TextEditingValue's JSON, -1 where there's no composing range. */
  public static TextEditState fromJson(JSONObject state) {
    return new TextEditState(
        state.optString("text"),
        state.optInt("selectionBase", -1),
        state.optInt("selectionExtent", -1),
        state.optInt("composingBase", -1),
        state.optInt("composingExtent", -1));
  }

  public boolean hasSelection() {
    // When selectionStart == -1, it's guaranteed that selectionEnd will also
    // be -1.
    return selectionStart >= 0;
  }

  public boolean hasComposing() {
    return composingStart >= 0 && composingEnd > composingStart;
  }
}
