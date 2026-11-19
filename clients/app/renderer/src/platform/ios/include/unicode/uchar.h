// The one ICU call Flutter's iOS text input plugin makes, whether a character is an emoji, from the system's ICU
// (libicucore, which `build.rs` links), whose headers the iOS SDK doesn't ship. ICU's own names and values
// (unicode/uchar.h, UCHAR_EMOJI = 57).

#include <stdint.h>

typedef int32_t UChar32;
typedef int8_t UBool;
typedef enum UProperty { UCHAR_EMOJI = 57 } UProperty;

#ifdef __cplusplus
extern "C"
#endif
    UBool
    u_hasBinaryProperty(UChar32 c, UProperty which);
