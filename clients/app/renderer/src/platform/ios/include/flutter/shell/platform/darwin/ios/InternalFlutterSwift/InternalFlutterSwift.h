// What Flutter's engine makes in Swift that its iOS text input plugin names (clients/app/renderer/flutter): a hardware
// key's press, which the plugin's `handlePress:` takes and doesn't handle.

#import <UIKit/UIKit.h>

@interface FlutterUIPressProxy : NSObject
@end
