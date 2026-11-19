// Ours, in place of Flutter's string_attribute.h: its plain structs, as Flutter has them, without the Dart wrapper.
#pragma once
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace flutter {

struct StringAttribute;

using StringAttributePtr = std::shared_ptr<flutter::StringAttribute>;
using StringAttributes = std::vector<StringAttributePtr>;

enum class StringAttributeType : int32_t {
  kSpellOut,
  kLocale,
};

struct StringAttribute {
  virtual ~StringAttribute() = default;
  int32_t start = -1;
  int32_t end = -1;
  StringAttributeType type = StringAttributeType::kSpellOut;
};

struct SpellOutStringAttribute : StringAttribute {};

struct LocaleStringAttribute : StringAttribute {
  std::string locale;
};

}  // namespace flutter
