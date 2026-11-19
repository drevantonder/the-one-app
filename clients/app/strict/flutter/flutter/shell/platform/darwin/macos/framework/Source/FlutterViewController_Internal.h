// Ours, in place of Flutter's FlutterViewController_Internal.h, for what FlutterTextInputPlugin uses of it.

#import "flutter/shell/platform/darwin/macos/framework/Headers/FlutterViewController.h"

@interface FlutterViewController (Internal)
// What the plugin's rects are in: the strict view's own coordinates, from its top left, as a FlutterView's are.
@property(nonatomic, readonly, nonnull) NSView* flutterView;
- (BOOL)isDispatchingKeyEvent:(nonnull NSEvent*)event;
@end
