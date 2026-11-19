// From Flutter's engine (BSD-3-Clause, LICENSE in clients/app/strict/flutter), engine/src/flutter/shell/platform/windows/text_input_plugin.h
// at flutter/flutter 4d19bda07ae859c5d3a205e3a61619ff485a1182.
// Changed: its channel is the strict host's (text_input_channel.h): TextInput's method calls come in decoded, one
// method each, and TextInputClient's go out through it; no delta model, which the host doesn't ask for.

// Copyright 2013 The Flutter Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef FLUTTER_SHELL_PLATFORM_WINDOWS_TEXT_INPUT_PLUGIN_H_
#define FLUTTER_SHELL_PLATFORM_WINDOWS_TEXT_INPUT_PLUGIN_H_

#include <array>
#include <map>
#include <memory>

#include "flutter/fml/macros.h"
#include "flutter/shell/geometry/geometry.h"
#include <string>

#include "flutter/shell/platform/common/text_input_model.h"
#include "flutter/shell/platform/embedder/embedder.h"
#include "flutter/shell/platform/windows/text_input_channel.h"

namespace flutter {

class FlutterWindowsEngine;

// Implements a text input plugin.
//
// Specifically handles window events within windows.
class TextInputPlugin {
 public:
  TextInputPlugin(TextInputChannel channel, FlutterWindowsEngine* engine);

  virtual ~TextInputPlugin();

  // Called when the Flutter engine receives a raw keyboard message.
  virtual void KeyboardHook(int key,
                            int scancode,
                            int action,
                            char32_t character,
                            bool extended,
                            bool was_down);

  // Called when the Flutter engine receives a keyboard character.
  virtual void TextHook(const std::u16string& text);

  // Called on an IME compose begin event.
  //
  // Triggered when the user begins editing composing text using a multi-step
  // input method such as in CJK text input.
  virtual void ComposeBeginHook();

  // Called on an IME compose commit event.
  //
  // Triggered when the user triggers a commit of the current composing text
  // while using a multi-step input method such as in CJK text input. Composing
  // continues with the next keypress.
  virtual void ComposeCommitHook();

  // Called on an IME compose end event.
  //
  // Triggered when the composing ends, for example when the user presses
  // ESC or when the user triggers a commit of the composing text while using a
  // multi-step input method such as in CJK text input.
  virtual void ComposeEndHook();

  // Called on an IME composing region change event.
  //
  // Triggered when the user edits the composing text while using a multi-step
  // input method such as in CJK text input.
  virtual void ComposeChangeHook(const std::u16string& text, int cursor_pos);

  // TextInput's method calls, as the framework makes them.
  void SetClient(int client_id,
                 FlutterViewId view_id,
                 const std::string& input_type,
                 const std::string& input_action);
  void ClearClient();
  void SetEditingState(const std::string& text,
                       int selection_base,
                       int selection_extent,
                       int composing_base,
                       int composing_extent);
  void SetMarkedTextRect(const Rect& rect);
  void SetEditableSizeAndTransform(const std::array<double, 16>& transform);

  // Whether a client is set.
  bool HasClient() const { return active_model_ != nullptr; }

  // Called when a view is removed from the engine.
  //
  // If the removed view is the currently active view for text input, resets
  // the active model and view id to prevent stale references. The implicit
  // view is excluded from this reset.
  void OnViewRemoved(FlutterViewId view_id);

 private:
  // Allows modifying the TextInputPlugin in tests.
  friend class TextInputPluginModifier;

  // Sends the current state of the given model to the Flutter engine.
  void SendStateUpdate(const TextInputModel& model);

  // Sends an action triggered by the Enter key to the Flutter engine.
  void EnterPressed(TextInputModel* model);

  // Returns the composing rect, or if IME composing mode is not active, the
  // cursor rect in the PipelineOwner root coordinate system.
  Rect GetCursorRect() const;

  // The channel used for communication with the strict host.
  TextInputChannel channel_;

  // The associated |FlutterWindowsEngine|.
  FlutterWindowsEngine* engine_;

  // The active client id.
  int client_id_;

  // The active view id.
  FlutterViewId view_id_ = 0;

  // The active model. nullptr if not set.
  std::unique_ptr<TextInputModel> active_model_;

  // Keyboard type of the client. See available options:
  // https://api.flutter.dev/flutter/services/TextInputType-class.html
  std::string input_type_;

  // An action requested by the user on the input client. See available options:
  // https://api.flutter.dev/flutter/services/TextInputAction-class.html
  std::string input_action_;

  // The smallest rect, in local coordinates, of the text in the composing
  // range, or of the caret in the case where there is no current composing
  // range. This value is updated via `TextInput.setMarkedTextRect` messages
  // over the text input channel.
  Rect composing_rect_;

  // A 4x4 matrix that maps from `EditableText` local coordinates to the
  // coordinate system of `PipelineOwner.rootNode`.
  std::array<std::array<double, 4>, 4> editabletext_transform_ = {
      0.0, 0.0, 0.0, 0.0,  //
      0.0, 0.0, 0.0, 0.0,  //
      0.0, 0.0, 0.0, 0.0,  //
      0.0, 0.0, 0.0, 0.0};

  FML_DISALLOW_COPY_AND_ASSIGN(TextInputPlugin);
};

}  // namespace flutter

#endif  // FLUTTER_SHELL_PLATFORM_WINDOWS_TEXT_INPUT_PLUGIN_H_
