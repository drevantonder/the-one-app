// Ours, in place of Flutter's FlutterViewController.h: a strict window's view, whose keys and pointer
// FlutterTextInputPlugin gives back to it (`platform/mac/text_input.mm`).

#import <Cocoa/Cocoa.h>

// As Flutter's brings its channels' types, through its FlutterEngine.h.
#import "flutter/shell/platform/darwin/common/framework/Headers/FlutterCodecs.h"

typedef int64_t FlutterViewIdentifier;

@interface FlutterViewController : NSViewController
- (nonnull instancetype)initWithStrictView:(nonnull NSView*)view;
@end
