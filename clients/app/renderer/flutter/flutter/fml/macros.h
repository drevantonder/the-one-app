// Ours, in place of Flutter's fml/macros.h, for what the files beside it use of it.

#ifndef FLUTTER_FML_MACROS_H_
#define FLUTTER_FML_MACROS_H_

#define FML_DISALLOW_COPY_AND_ASSIGN(TypeName) \
  TypeName(const TypeName&) = delete;          \
  TypeName& operator=(const TypeName&) = delete

#endif  // FLUTTER_FML_MACROS_H_
