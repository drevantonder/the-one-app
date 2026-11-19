// A strict window's text input on Windows: Flutter's TextInputPlugin and TextInputManager (clients/app/strict/flutter),
// with what they have of Flutter's engine (its view, its engine) standing in for the strict window, and a C interface
// to them for the window's Rust (`window.rs`). The input method's messages are handled as Flutter's FlutterWindow
// handles them (engine/src/flutter/shell/platform/windows/flutter_window.cc at flutter/flutter
// 4d19bda07ae859c5d3a205e3a61619ff485a1182, BSD-3-Clause, LICENSE in clients/app/strict/flutter): `text_input_ime`.

#include <windows.h>

#include <imm.h>

#include <array>
#include <string>

#include "flutter/shell/platform/windows/flutter_windows_engine.h"
#include "flutter/shell/platform/windows/text_input_manager.h"
#include "flutter/shell/platform/windows/text_input_plugin.h"

namespace {

using flutter::Point;
using flutter::Rect;
using flutter::Size;

// A window's plugin and input method, and the window as Flutter's engine and view are to them.
class TextInput : public flutter::FlutterWindowsEngine, public flutter::FlutterWindowsView {
 public:
  TextInput(HWND window, TextInputChannel channel) : window_(window), plugin(channel, this) {
    manager.SetWindowHandle(window);
  }

  flutter::FlutterWindowsView* view(FlutterViewId view_id) const override {
    return const_cast<TextInput*>(this);
  }

  // As Flutter's FlutterWindow::OnCursorRectUpdated: from logical pixels to the window's.
  void OnCursorRectUpdated(const Rect& rect) override {
    double scale = GetDpiForWindow(window_) / 96.0;
    manager.UpdateCaretRect(Rect(Point(rect.left() * scale, rect.top() * scale),
                                 Size(rect.width() * scale, rect.height() * scale)));
  }

  void OnResetImeComposing() override { manager.AbortComposing(); }

  flutter::TextInputManager manager;
  flutter::TextInputPlugin plugin;

 private:
  HWND window_;
};

// As Flutter's GetCursorPositionForComposition.
int CursorPositionForComposition(const flutter::TextInputManager& manager,
                                 LPARAM lparam,
                                 size_t text_length) {
  if (!(lparam & GCS_CURSORPOS)) {
    return static_cast<int>(text_length);
  }
  int position = static_cast<int>(manager.GetComposingCursorPosition());
  if (position < 0 || static_cast<size_t>(position) > text_length) {
    return static_cast<int>(text_length);
  }
  return position;
}

}  // namespace

extern "C" {

void* text_input_new(HWND window, TextInputChannel channel) {
  return new TextInput(window, channel);
}

void text_input_free(void* input) {
  delete static_cast<TextInput*>(input);
}

bool text_input_has_client(void* input) {
  return input != nullptr && static_cast<TextInput*>(input)->plugin.HasClient();
}

// TextInput's method calls, decoded.
void text_input_set_client(void* input,
                           int client,
                           const char* input_type,
                           const char* input_action) {
  static_cast<TextInput*>(input)->plugin.SetClient(client, flutter::kImplicitViewId, input_type,
                                                   input_action);
}

void text_input_clear_client(void* input) {
  static_cast<TextInput*>(input)->plugin.ClearClient();
}

void text_input_set_editing_state(void* input,
                                  const char* text,
                                  int selection_base,
                                  int selection_extent,
                                  int composing_base,
                                  int composing_extent) {
  static_cast<TextInput*>(input)->plugin.SetEditingState(text, selection_base, selection_extent,
                                                         composing_base, composing_extent);
}

void text_input_set_marked_text_rect(void* input, double x, double y, double width, double height) {
  static_cast<TextInput*>(input)->plugin.SetMarkedTextRect(Rect(Point(x, y), Size(width, height)));
}

void text_input_set_editable_transform(void* input, const double* transform) {
  std::array<double, 16> matrix;
  std::copy(transform, transform + 16, matrix.begin());
  static_cast<TextInput*>(input)->plugin.SetEditableSizeAndTransform(matrix);
}

// What a key typed (WM_CHAR's, a surrogate pair whole), as Flutter's FlutterWindow::OnText gives it.
void text_input_text(void* input, const char16_t* text, size_t length) {
  static_cast<TextInput*>(input)->plugin.TextHook(std::u16string(text, length));
}

// The input method's message `message`, as Flutter's FlutterWindow handles it: → whether it's handled here, so
// Windows' own handling (DefWindowProc) doesn't have it, with `lparam` as DefWindowProc is to have it.
bool text_input_ime(void* input, UINT message, WPARAM wparam, LPARAM* lparam) {
  TextInput* text_input = static_cast<TextInput*>(input);
  flutter::TextInputManager& manager = text_input->manager;
  flutter::TextInputPlugin& plugin = text_input->plugin;
  switch (message) {
    case WM_IME_SETCONTEXT:
      if (wparam != 0) {
        manager.CreateImeWindow();
      }
      // The composition window is hidden, since the page draws what's composed itself.
      *lparam &= ~ISC_SHOWUICOMPOSITIONWINDOW;
      return false;
    case WM_IME_STARTCOMPOSITION:
      manager.CreateImeWindow();
      plugin.ComposeBeginHook();
      return true;
    case WM_IME_COMPOSITION: {
      manager.UpdateImeWindow();
      if (*lparam == 0) {
        plugin.ComposeChangeHook(u"", 0);
        plugin.ComposeCommitHook();
      }
      if (*lparam & GCS_RESULTSTR) {
        std::optional<std::u16string> text = manager.GetResultString();
        if (text) {
          int position = CursorPositionForComposition(manager, *lparam, text->length());
          plugin.ComposeChangeHook(text.value(), position);
          plugin.ComposeCommitHook();
        }
      }
      if (*lparam & GCS_COMPSTR) {
        std::optional<std::u16string> text = manager.GetComposingString();
        if (text) {
          int position = CursorPositionForComposition(manager, *lparam, text->length());
          plugin.ComposeChangeHook(text.value(), position);
        }
      }
      return (*lparam & GCS_RESULTSTR) || (*lparam & GCS_COMPSTR);
    }
    case WM_IME_ENDCOMPOSITION:
      manager.DestroyImeWindow();
      plugin.ComposeEndHook();
      return true;
  }
  return false;
}

}  // extern "C"
