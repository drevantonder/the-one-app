// Ours, in place of Flutter's flutter_windows_engine.h, for what TextInputPlugin uses of it: a native window has one
// view (src/platform/windows/text_input.cc).

#ifndef FLUTTER_SHELL_PLATFORM_WINDOWS_FLUTTER_WINDOWS_ENGINE_H_
#define FLUTTER_SHELL_PLATFORM_WINDOWS_FLUTTER_WINDOWS_ENGINE_H_

#include "flutter/shell/platform/embedder/embedder.h"
#include "flutter/shell/platform/windows/flutter_windows_view.h"

namespace flutter {

class FlutterWindowsEngine {
 public:
  virtual ~FlutterWindowsEngine() = default;
  virtual FlutterWindowsView* view(FlutterViewId view_id) const = 0;
};

}  // namespace flutter

#endif  // FLUTTER_SHELL_PLATFORM_WINDOWS_FLUTTER_WINDOWS_ENGINE_H_
