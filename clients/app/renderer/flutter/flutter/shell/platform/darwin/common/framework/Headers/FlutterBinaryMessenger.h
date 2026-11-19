// Ours, in place of Flutter's FlutterBinaryMessenger.h: what carries the text input channel's method calls between
// FlutterTextInputPlugin and the native host's Rust (`platform/mac/text_input.mm`), which isn't Flutter's engine.

#import <Foundation/Foundation.h>

@protocol FlutterBinaryMessenger <NSObject>
@end
