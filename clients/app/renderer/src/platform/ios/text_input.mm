// A native window's text input on an iPhone: Flutter's FlutterTextInputPlugin (clients/app/renderer/flutter), with
// what it has of Flutter's engine standing in for the native host: its text input delegate, which says what the
// plugin's view did as Flutter's text input channel's method calls, as FlutterEngine.mm's does, and a C interface to it
// for the window's Rust (`mod.rs`). The calls are JSON: the screen's `TextInput` calls go in (`text_input_call`), and
// the plugin's `TextInputClient` calls come out (the `send` it's made with).

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

// Flutter's own edit menu's (ContextMenu) and its platform plugin's: a native window shows the system's, which asks
// none of these.
- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    willDismissEditMenuWithTextInputClient:(int)client {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView shareSelectedText:(NSString*)selectedText {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView searchWebWithSelectedText:(NSString*)selectedText {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView lookUpSelectedText:(NSString*)selectedText {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView translateSelectedText:(NSString*)selectedText {
}

- (void)flutterTextInputView:(FlutterTextInputView*)textInputView
    performContextMenuCustomActionWithActionID:(NSString*)actionID
                               textInputClient:(int)client {
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

}  // extern "C"
