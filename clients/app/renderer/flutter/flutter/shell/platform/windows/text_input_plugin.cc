// From Flutter's engine (BSD-3-Clause, LICENSE in clients/app/renderer/flutter), engine/src/flutter/shell/platform/windows/text_input_plugin.cc
// at flutter/flutter 4d19bda07ae859c5d3a205e3a61619ff485a1182.
// Changed: its channel is the native host's (text_input_channel.h): HandleMethodCall's branches are a method each,
// given their arguments decoded, and its updates go out through it; no delta model, which the host doesn't ask for.

// Copyright 2013 The Flutter Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "flutter/shell/platform/windows/text_input_plugin.h"

#include <windows.h>

#include <cstdint>

#include "flutter/fml/string_conversion.h"
#include "flutter/shell/platform/windows/flutter_windows_engine.h"
#include "flutter/shell/platform/windows/flutter_windows_view.h"

static constexpr char kMultilineInputType[] = "TextInputType.multiline";

static constexpr char kInputActionNewline[] = "TextInputAction.newline";

namespace flutter {

void TextInputPlugin::TextHook(const std::u16string& text) {
  if (active_model_ == nullptr) {
    return;
  }
  std::u16string text_before_change =
      fml::Utf8ToUtf16(active_model_->GetText());
  TextRange selection_before_change = active_model_->selection();
  active_model_->AddText(text);

  SendStateUpdate(*active_model_);
}

void TextInputPlugin::KeyboardHook(int key,
                                   int scancode,
                                   int action,
                                   char32_t character,
                                   bool extended,
                                   bool was_down) {
  if (active_model_ == nullptr) {
    return;
  }
  if (action == WM_KEYDOWN || action == WM_SYSKEYDOWN) {
    // Most editing keys (arrow keys, backspace, delete, etc.) are handled in
    // the framework, so don't need to be handled at this layer.
    switch (key) {
      case VK_RETURN:
        EnterPressed(active_model_.get());
        break;
      default:
        break;
    }
  }
}

TextInputPlugin::TextInputPlugin(TextInputChannel channel,
                                 FlutterWindowsEngine* engine)
    : channel_(channel), engine_(engine), active_model_(nullptr) {}

TextInputPlugin::~TextInputPlugin() = default;

void TextInputPlugin::ComposeBeginHook() {
  if (active_model_ == nullptr) {
    return;
  }
  active_model_->BeginComposing();
  SendStateUpdate(*active_model_);
}

void TextInputPlugin::ComposeCommitHook() {
  if (active_model_ == nullptr) {
    return;
  }
  std::string text_before_change = active_model_->GetText();
  TextRange selection_before_change = active_model_->selection();
  TextRange composing_before_change = active_model_->composing_range();
  std::string composing_text_before_change = text_before_change.substr(
      composing_before_change.start(), composing_before_change.length());
  active_model_->CommitComposing();

  // We do not trigger SendStateUpdate here.
  //
  // Until a WM_IME_ENDCOMPOSING event, the user is still composing from the OS
  // point of view. Commit events are always immediately followed by another
  // composing event or an end composing event. However, in the brief window
  // between the commit event and the following event, the composing region is
  // collapsed. Notifying the framework of this intermediate state will trigger
  // any framework code designed to execute at the end of composing, such as
  // input formatters, which may try to update the text and send a message back
  // to the engine with changes.
  //
  // This is a particular problem with Korean IMEs, which build up one
  // character at a time in their composing region until a keypress that makes
  // no sense for the in-progress character. At that point, the result
  // character is committed and a compose event is immedidately received with
  // the new composing region.
  //
  // In the case where this event is immediately followed by a composing event,
  // the state will be sent in ComposeChangeHook.
  //
  // In the case where this event is immediately followed by an end composing
  // event, the state will be sent in ComposeEndHook.
}

void TextInputPlugin::ComposeEndHook() {
  if (active_model_ == nullptr) {
    return;
  }
  std::string text_before_change = active_model_->GetText();
  TextRange selection_before_change = active_model_->selection();
  active_model_->CommitComposing();
  active_model_->EndComposing();
  SendStateUpdate(*active_model_);
}

void TextInputPlugin::ComposeChangeHook(const std::u16string& text,
                                        int cursor_pos) {
  if (active_model_ == nullptr) {
    return;
  }
  std::string text_before_change = active_model_->GetText();
  TextRange composing_before_change = active_model_->composing_range();
  active_model_->AddText(text);
  active_model_->UpdateComposingText(text, TextRange(cursor_pos, cursor_pos));
  std::string text_after_change = active_model_->GetText();
  SendStateUpdate(*active_model_);
}

void TextInputPlugin::ClearClient() {
  FlutterWindowsView* view = engine_->view(view_id_);
  if (view == nullptr) {
    return;
  }
  if (active_model_ != nullptr && active_model_->composing()) {
    active_model_->CommitComposing();
    active_model_->EndComposing();
    SendStateUpdate(*active_model_);
  }
  view->OnResetImeComposing();
  active_model_ = nullptr;
}

void TextInputPlugin::SetClient(int client_id,
                                FlutterViewId view_id,
                                const std::string& input_type,
                                const std::string& input_action) {
  client_id_ = client_id;
  view_id_ = view_id;
  input_action_ = input_action;
  input_type_ = input_type;
  active_model_ = std::make_unique<TextInputModel>();
}

void TextInputPlugin::SetEditingState(const std::string& text,
                                      int selection_base,
                                      int selection_extent,
                                      int composing_base,
                                      int composing_extent) {
  if (active_model_ == nullptr) {
    return;
  }
  // Flutter uses -1/-1 for invalid; translate that to 0/0 for the model.
  if (selection_base == -1 && selection_extent == -1) {
    selection_base = selection_extent = 0;
  }
  if (composing_base == -1 && composing_extent == -1) {
    composing_base = composing_extent = 0;
  }
  active_model_->SetText(text, TextRange(selection_base, selection_extent),
                         TextRange(composing_base, composing_extent));
}

void TextInputPlugin::SetMarkedTextRect(const Rect& rect) {
  FlutterWindowsView* view = engine_->view(view_id_);
  if (view == nullptr) {
    return;
  }
  composing_rect_ = rect;
  Rect transformed_rect = GetCursorRect();
  view->OnCursorRectUpdated(transformed_rect);
}

void TextInputPlugin::SetEditableSizeAndTransform(
    const std::array<double, 16>& transform) {
  FlutterWindowsView* view = engine_->view(view_id_);
  if (view == nullptr) {
    return;
  }
  size_t i = 0;
  for (double entry : transform) {
    editabletext_transform_[i / 4][i % 4] = entry;
    ++i;
  }
  Rect transformed_rect = GetCursorRect();
  view->OnCursorRectUpdated(transformed_rect);
}

Rect TextInputPlugin::GetCursorRect() const {
  Point transformed_point = {
      composing_rect_.left() * editabletext_transform_[0][0] +
          composing_rect_.top() * editabletext_transform_[1][0] +
          editabletext_transform_[3][0],
      composing_rect_.left() * editabletext_transform_[0][1] +
          composing_rect_.top() * editabletext_transform_[1][1] +
          editabletext_transform_[3][1]};
  return {transformed_point, composing_rect_.size()};
}

void TextInputPlugin::SendStateUpdate(const TextInputModel& model) {
  TextRange selection = model.selection();
  int composing_base = model.composing() ? model.composing_range().base() : -1;
  int composing_extent =
      model.composing() ? model.composing_range().extent() : -1;
  channel_.update_editing_state(channel_.context, client_id_,
                                model.GetText().c_str(), selection.base(),
                                selection.extent(), composing_base,
                                composing_extent);
}

void TextInputPlugin::EnterPressed(TextInputModel* model) {
  if (input_type_ == kMultilineInputType &&
      input_action_ == kInputActionNewline) {
    std::u16string text_before_change = fml::Utf8ToUtf16(model->GetText());
    TextRange selection_before_change = model->selection();
    model->AddText(u"\n");
    SendStateUpdate(*model);
  }
  channel_.perform_action(channel_.context, client_id_, input_action_.c_str());
}

void TextInputPlugin::OnViewRemoved(FlutterViewId view_id) {
  if (view_id == kImplicitViewId || view_id_ != view_id) {
    return;
  }

  // If composing, commit and end composing. Skip sending state updates and
  // IME reset since the view is being removed.
  if (active_model_ != nullptr && active_model_->composing()) {
    active_model_->CommitComposing();
    active_model_->EndComposing();
  }

  active_model_ = nullptr;
  view_id_ = 0;
}

}  // namespace flutter
