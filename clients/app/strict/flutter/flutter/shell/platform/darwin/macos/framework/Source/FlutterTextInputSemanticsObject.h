// Ours, in place of Flutter's FlutterTextInputSemanticsObject.h, for what FlutterTextInputPlugin uses of it: the
// text field VoiceOver edits, which the strict host doesn't make (its `client` is nil).

#import <Cocoa/Cocoa.h>

@interface FlutterTextField : NSTextField
- (void)startEditing;
- (void)updateString:(nonnull NSString*)string withSelection:(NSRange)selection;
@end
