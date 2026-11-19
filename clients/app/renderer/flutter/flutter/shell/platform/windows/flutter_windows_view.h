// Ours, in place of Flutter's flutter_windows_view.h, for what TextInputPlugin uses of it: a native window, whose
// input method's window goes where the caret is (src/platform/windows/text_input.cc).

#ifndef FLUTTER_SHELL_PLATFORM_WINDOWS_FLUTTER_WINDOWS_VIEW_H_
#define FLUTTER_SHELL_PLATFORM_WINDOWS_FLUTTER_WINDOWS_VIEW_H_

#include "flutter/shell/geometry/geometry.h"
#include "flutter/shell/platform/embedder/embedder.h"

namespace flutter {

// The view a window that has one has.
constexpr FlutterViewId kImplicitViewId = 0;

class FlutterWindowsView {
 public:
  virtual ~FlutterWindowsView() = default;
  // Where the caret is, in the window's logical pixels.
  virtual void OnCursorRectUpdated(const Rect& rect) = 0;
  virtual void OnResetImeComposing() = 0;
};

}  // namespace flutter

#endif  // FLUTTER_SHELL_PLATFORM_WINDOWS_FLUTTER_WINDOWS_VIEW_H_
