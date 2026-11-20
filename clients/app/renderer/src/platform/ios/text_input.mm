// A native window's text input on an iPhone: Flutter's FlutterTextInputPlugin (clients/app/renderer/flutter), with
// what it has of Flutter's engine standing in for the native host: its text input delegate, which says what the
// plugin's view did as Flutter's text input channel's method calls, as FlutterEngine.mm's does, and a C interface to it
// for the window's Rust (`mod.rs`). The calls are JSON: the screen's `TextInput` calls go in (`text_input_call`), and
// the plugin's `TextInputClient` calls come out (the `send` it's made with). With its field's edit menu
// (UIEditMenuInteraction's, `text_input_menu`) and iOS's magnifier (`UITextLoupeSession`, `text_input_magnifier`) for
// what a finger selects, whose handles the renderer draws.

#import "flutter/shell/platform/darwin/common/InternalFlutterSwiftCommon/InternalFlutterSwiftCommon.h"
#import "flutter/shell/platform/darwin/common/framework/Headers/FlutterCodecs.h"
#import "flutter/shell/platform/darwin/ios/InternalFlutterSwift/InternalFlutterSwift.h"
#import "flutter/shell/platform/darwin/ios/framework/Source/FlutterSharedApplication.h"
#import "flutter/shell/platform/darwin/ios/framework/Source/FlutterTextInputPlugin.h"
#import "flutter/shell/platform/darwin/ios/framework/Source/UIViewController+FlutterScreenAndSceneIfLoaded.h"

typedef void (*TextInputSend)(void* context, const char* method, const char* arguments);

NSObject const* FlutterMethodNotImplemented = [[NSObject alloc] init];

@implementation FlutterMethodCall
+ (instancetype)methodCallWithMethodName:(NSString*)method arguments:(id)arguments {
  FlutterMethodCall* call = [[FlutterMethodCall alloc] init];
  call->_method = method;
  call->_arguments = arguments;
  return call;
}
@end

@implementation FlutterError
+ (instancetype)errorWithCode:(NSString*)code message:(NSString*)message details:(id)details {
  NSLog(@"text input: %@: %@", code, message);
  return [[FlutterError alloc] init];
}
@end

@implementation FlutterUIPressProxy
@end

@implementation FlutterLogger
+ (void)logError:(NSString*)message {
  NSLog(@"text input: %@", message);
}
@end

// The app the plugin is in: an app, not an extension, with its scenes.
@implementation FlutterSharedApplication
+ (BOOL)isAppExtension {
  return NO;
}

+ (BOOL)isAvailable {
  return YES;
}

+ (UIApplication*)application {
  return UIApplication.sharedApplication;
}

+ (BOOL)hasSceneDelegate {
  return UIApplication.sharedApplication.connectedScenes.count > 0;
}

+ (int64_t)lastAppModificationTime {
  return 0;
}

+ (BOOL)isFlutterDeepLinkingEnabled {
  return NO;
}
@end

// As Flutter's UIViewController+FlutterScreenAndSceneIfLoaded.mm has it.
@implementation UIViewController (FlutterScreenAndSceneIfLoaded)
- (UIWindowScene*)flutterWindowSceneIfViewLoaded {
  return self.viewIfLoaded.window.windowScene;
}

- (UIScreen*)flutterScreenIfViewLoaded {
  return self.viewIfLoaded.window.windowScene.screen;
}
@end

// Flutter's engine, to the plugin: what its view did, as the text input channel's method calls (FlutterEngine.mm's
// "Text input delegate"), out to the window's Rust as JSON.
@interface TextInput : NSObject <FlutterTextInputDelegate>
@property(nonatomic, readonly) FlutterTextInputPlugin* plugin;
@property(nonatomic) void* context;
@property(nonatomic) TextInputSend send;
// Whether the field's edit menu is shown, as the screen asked, and what was selected as it showed; and the magnifier
// showing.
@property(nonatomic) BOOL menuShown;
@property(nonatomic) NSRange menuSelection;
@property(nonatomic, strong) id loupe;
@end

// What the field's view keeps (FlutterTextInputPlugin.mm's): where its edit menu is, which moved there moves the menu
// showing rather than showing it again, which would close it and say so; and its delegate and client, which its Select
// tells.
@interface FlutterTextInputView (EditMenu)
@property(nonatomic, assign) CGRect editMenuTargetRect;
@property(nonatomic, assign) int textInputClient;
- (id<FlutterTextInputDelegate>)textInputDelegate;
@end

// The edit menu's Select, which UIKit offers at a caret, as its own fields do: the screen selects the word there, as
// Flutter's engine tells its framework of a custom item (`ContextMenu.onPerformCustomAction`). A selection offers none.
@implementation FlutterTextInputView (Select)
- (void)select:(id)sender {
  [self.textInputDelegate flutterTextInputView:self
      performContextMenuCustomActionWithActionID:@"select"
                                 textInputClient:self.textInputClient];
}

- (void)validateCommand:(UICommand*)command {
  [super validateCommand:command];
  if (command.action == @selector(select:) && !self.selectedTextRange.isEmpty) {
    command.attributes |= UIMenuElementAttributesHidden;
  }
}
@end

@implementation TextInput
- (instancetype)initWithController:(UIViewController*)controller context:(void*)context send:(TextInputSend)send {
  self = [super init];
  _context = context;
  _send = send;
  _plugin = [[FlutterTextInputPlugin alloc] initWithDelegate:self];
  _plugin.viewController = controller;
  return self;
}

- (void)invokeMethod:(NSString*)method arguments:(id)arguments {
  NSData* json = [NSJSONSerialization dataWithJSONObject:arguments ?: [NSNull null]
                                                 options:NSJSONWritingFragmentsAllowed
                                                   error:nil];
  NSString* text = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
  _send(_context, method.UTF8String, text.UTF8String);
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
         updateEditingClient:(int)client
                   withState:(NSDictionary*)state {
  [self invokeMethod:@"TextInputClient.updateEditingState" arguments:@[ @(client), state ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
         updateEditingClient:(int)client
                   withState:(NSDictionary*)state
                     withTag:(NSString*)tag {
  [self invokeMethod:@"TextInputClient.updateEditingStateWithTag" arguments:@[ @(client), @{tag : state} ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
         updateEditingClient:(int)client
                   withDelta:(NSDictionary*)delta {
  [self invokeMethod:@"TextInputClient.updateEditingStateWithDeltas" arguments:@[ @(client), delta ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
        updateFloatingCursor:(FlutterFloatingCursorDragState)state
                  withClient:(int)client
                withPosition:(NSDictionary*)position {
  NSString* stateString;
  switch (state) {
    case FlutterFloatingCursorDragStateStart:
      stateString = @"FloatingCursorDragState.start";
      break;
    case FlutterFloatingCursorDragStateUpdate:
      stateString = @"FloatingCursorDragState.update";
      break;
    case FlutterFloatingCursorDragStateEnd:
      stateString = @"FloatingCursorDragState.end";
      break;
  }
  [self invokeMethod:@"TextInputClient.updateFloatingCursor" arguments:@[ @(client), stateString, position ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
               performAction:(FlutterTextInputAction)action
                  withClient:(int)client {
  NSString* actionString;
  switch (action) {
    case FlutterTextInputActionUnspecified:
      actionString = @"TextInputAction.unspecified";
      break;
    case FlutterTextInputActionDone:
      actionString = @"TextInputAction.done";
      break;
    case FlutterTextInputActionGo:
      actionString = @"TextInputAction.go";
      break;
    case FlutterTextInputActionSend:
      actionString = @"TextInputAction.send";
      break;
    case FlutterTextInputActionSearch:
      actionString = @"TextInputAction.search";
      break;
    case FlutterTextInputActionNext:
      actionString = @"TextInputAction.next";
      break;
    case FlutterTextInputActionContinue:
      actionString = @"TextInputAction.continueAction";
      break;
    case FlutterTextInputActionJoin:
      actionString = @"TextInputAction.join";
      break;
    case FlutterTextInputActionRoute:
      actionString = @"TextInputAction.route";
      break;
    case FlutterTextInputActionEmergencyCall:
      actionString = @"TextInputAction.emergencyCall";
      break;
    case FlutterTextInputActionNewline:
      actionString = @"TextInputAction.newline";
      break;
  }
  [self invokeMethod:@"TextInputClient.performAction" arguments:@[ @(client), actionString ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    showAutocorrectionPromptRectForStart:(NSUInteger)start
                                     end:(NSUInteger)end
                              withClient:(int)client {
  [self invokeMethod:@"TextInputClient.showAutocorrectionPromptRect" arguments:@[ @(client), @(start), @(end) ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView showToolbar:(int)client {
  [self invokeMethod:@"TextInputClient.showToolbar" arguments:@[ @(client) ]];
}

- (void)flutterTextInputViewScribbleInteractionBegan:(FlutterTextInputView*)textInputView {
  [self invokeMethod:@"TextInputClient.scribbleInteractionBegan" arguments:nil];
}

- (void)flutterTextInputViewScribbleInteractionFinished:(FlutterTextInputView*)textInputView {
  [self invokeMethod:@"TextInputClient.scribbleInteractionFinished" arguments:nil];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    insertTextPlaceholderWithSize:(CGSize)size
                       withClient:(int)client {
  [self invokeMethod:@"TextInputClient.insertTextPlaceholder" arguments:@[ @(client), @(size.width), @(size.height) ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView removeTextPlaceholder:(int)client {
  [self invokeMethod:@"TextInputClient.removeTextPlaceholder" arguments:@[ @(client) ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    didResignFirstResponderWithTextInputClient:(int)client {
  [self invokeMethod:@"TextInputClient.onConnectionClosed" arguments:@[ @(client) ]];
}

// The field still has the keyboard as far as the screen knows, as it gives it up only by its own calls.
- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    didRestoreFirstResponderWithTextInputClient:(int)client
                                         result:(FlutterResult)callback {
  callback(@YES);
}

// The edit menu closed by itself (a press outside it, or an item chosen): the screen's told, as Flutter's engine tells
// its framework, so it isn't shown again till a finger asks. One the screen took away it knows of, and one closed by an
// item that changed what's selected (Select All, Paste) the screen shows again or not by that change, which reached
// it first.
- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    willDismissEditMenuWithTextInputClient:(int)client {
  if (!_menuShown) {
    return;
  }
  _menuShown = NO;
  if (!NSEqualRanges(((FlutterTextRange*)textInputView.selectedTextRange).range, _menuSelection)) {
    return;
  }
  [self invokeMethod:@"ContextMenu.onDismissSystemContextMenu" arguments:@[ @(client) ]];
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    performContextMenuCustomActionWithActionID:(NSString*)actionID
                               textInputClient:(int)client {
  // The screen does it, after the menu closes: that closing is the item's, not a person's.
  _menuShown = NO;
  [self invokeMethod:@"ContextMenu.onPerformCustomAction" arguments:@[ @(client), actionID ]];
}

// Flutter's own edit menu's other items: the system's menu does these itself.

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView shareSelectedText:(NSString*)selectedText {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView searchWebWithSelectedText:(NSString*)selectedText {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView lookUpSelectedText:(NSString*)selectedText {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView translateSelectedText:(NSString*)selectedText {
}
@end

extern "C" {

// A text input plugin for the window `controller` shows, which says what it says to `send` with `context`.
void* text_input_new(UIViewController* controller, void* context, TextInputSend send) {
  return (__bridge_retained void*)[[TextInput alloc] initWithController:controller context:context send:send];
}

void text_input_free(void* input) {
  TextInput* text_input = (__bridge_transfer TextInput*)input;
  [text_input.plugin reset];
}

// A method call of Flutter's text input channel, with its arguments as JSON.
void text_input_call(void* input, const char* method, const char* arguments) {
  TextInput* text_input = (__bridge TextInput*)input;
  NSData* json = [NSData dataWithBytes:arguments length:strlen(arguments)];
  id decoded = [NSJSONSerialization JSONObjectWithData:json options:NSJSONReadingFragmentsAllowed error:nil];
  FlutterMethodCall* call =
      [FlutterMethodCall methodCallWithMethodName:@(method)
                                        arguments:decoded == [NSNull null] ? nil : decoded];
  [text_input.plugin handleMethodCall:call
                               result:^(id result){
                               }];
}

// The field's edit menu, UIKit's own (what it offers is what the field can do: Cut, Copy, Paste, Select, Select All,
// Look Up…), beside `x`, `y`, `width`, `height` in the window's points, or none (`shown` false): moved where it's
// already shown.
void text_input_menu(void* input, bool shown, double x, double y, double width, double height) {
  if (@available(iOS 16.0, *)) {
    TextInput* text_input = (__bridge TextInput*)input;
    FlutterTextInputPlugin* plugin = text_input.plugin;
    if (!shown) {
      if (text_input.menuShown) {
        text_input.menuShown = NO;
        [plugin hideEditMenu];
      }
      return;
    }
    CGRect rect = CGRectMake(x, y, width, height);
    FlutterTextInputView* view = (FlutterTextInputView*)plugin.textInputView;
    if (text_input.menuShown && view) {
      view.editMenuTargetRect = [plugin.viewController.view convertRect:rect toView:view];
      [view.editMenuInteraction updateVisibleMenuPositionAnimated:NO];
      text_input.menuSelection = ((FlutterTextRange*)view.selectedTextRange).range;
      return;
    }
    NSDictionary* target = @{@"x" : @(x), @"y" : @(y), @"width" : @(width), @"height" : @(height)};
    text_input.menuShown = [plugin showEditMenu:@{@"targetRect" : target}];
    text_input.menuSelection = ((FlutterTextRange*)view.selectedTextRange).range;
  }
}

// iOS's magnifier over what's at `x`, `y` in the window's points, with the caret it follows, as its own fields show it
// while a selection is moved, or none (`shown` false).
void text_input_magnifier(void* input, bool shown, double x, double y, double caret_x, double caret_y, double caret_width, double caret_height) {
  if (@available(iOS 17.0, *)) {
    TextInput* text_input = (__bridge TextInput*)input;
    UITextLoupeSession* loupe = text_input.loupe;
    if (!shown) {
      [loupe invalidate];
      text_input.loupe = nil;
      return;
    }
    CGPoint at = CGPointMake(x, y);
    if (!loupe) {
      loupe = [UITextLoupeSession beginLoupeSessionAtPoint:at
                                   fromSelectionWidgetView:nil
                                                    inView:text_input.plugin.viewController.view];
      text_input.loupe = loupe;
    }
    [loupe moveToPoint:at withCaretRect:CGRectMake(caret_x, caret_y, caret_width, caret_height) trackingCaret:YES];
  }
}

}  // extern "C"
