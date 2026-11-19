// Ours, in place of Flutter's fml/logging.h, for what the files beside it use of it: FML_DCHECK, which checks
// nothing, as in Flutter's release builds.

#ifndef FLUTTER_FML_LOGGING_H_
#define FLUTTER_FML_LOGGING_H_

#define FML_DCHECK(condition) ((void)0)

#endif  // FLUTTER_FML_LOGGING_H_
