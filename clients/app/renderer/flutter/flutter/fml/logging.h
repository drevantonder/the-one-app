// Ours, in place of Flutter's fml/logging.h, for what the files beside it use of it: FML_DCHECK, which checks
// nothing, as in Flutter's release builds, and takes what's streamed to it as its message.

#ifndef FLUTTER_FML_LOGGING_H_
#define FLUTTER_FML_LOGGING_H_

namespace fml {
struct NullStream {
  template <typename T>
  NullStream& operator<<(const T&) {
    return *this;
  }
};
}  // namespace fml

#define FML_DCHECK(condition) \
  while (false)               \
  ::fml::NullStream()

#endif  // FLUTTER_FML_LOGGING_H_
