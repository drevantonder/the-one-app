// Ours, in place of Flutter's custom_accessibility_action.h: its plain struct, without the Dart wrapper.
#pragma once
#include <cstdint>
#include <string>
#include <unordered_map>

namespace flutter {

struct CustomAccessibilityAction {
  int32_t id = 0;
  int32_t overrideId = -1;
  std::string label;
  std::string hint;
};

using CustomAccessibilityActionUpdates = std::unordered_map<int32_t, CustomAccessibilityAction>;

}  // namespace flutter
