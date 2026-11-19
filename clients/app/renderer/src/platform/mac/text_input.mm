// A native window's text input on a Mac: Flutter's FlutterTextInputPlugin (clients/app/renderer/flutter), with what it
// has of Flutter's engine (its method channel, its view controller) standing in for the native host, and a C
// interface to it for the window's Rust (`text_input.rs`). The method calls are Flutter's text input channel's, as
// JSON: the screen's `TextInput` calls go in (`text_input_call`), and the plugin's `TextInputClient` calls come out
// (the `send` it's made with).

#import "flutter/shell/platform/darwin/common/framework/Headers/FlutterCodecs.h"
#import "flutter/shell/platform/darwin/macos/framework/Source/FlutterTextInputPlugin.h"
#import "flutter/shell/platform/darwin/macos/framework/Source/FlutterViewController_Internal.h"

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

@implementation FlutterJSONMethodCodec
+ (instancetype)sharedInstance {
  static FlutterJSONMethodCodec* codec = [[FlutterJSONMethodCodec alloc] init];
  return codec;
}
@end

// What carries the channel's calls: out to the window's Rust as JSON, and in to the plugin's handler.
@interface TextInputMessenger : NSObject <FlutterBinaryMessenger>
@property(nonatomic) void* context;
@property(nonatomic) TextInputSend send;
@property(nonatomic, copy) FlutterMethodCallHandler handler;
@end

@implementation TextInputMessenger
@end

@implementation FlutterMethodChannel {
  TextInputMessenger* _messenger;
}

+ (instancetype)methodChannelWithName:(NSString*)name
                      binaryMessenger:(NSObject<FlutterBinaryMessenger>*)messenger
                                codec:(FlutterJSONMethodCodec*)codec {
  FlutterMethodChannel* channel = [[FlutterMethodChannel alloc] init];
  channel->_messenger = (TextInputMessenger*)messenger;
  return channel;
}

- (void)invokeMethod:(NSString*)method arguments:(id)arguments {
  NSData* json = [NSJSONSerialization dataWithJSONObject:arguments ?: [NSNull null]
                                                 options:NSJSONWritingFragmentsAllowed
                                                   error:nil];
  NSString* text = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
  _messenger.send(_messenger.context, method.UTF8String, text.UTF8String);
}

- (void)setMethodCallHandler:(FlutterMethodCallHandler)handler {
  _messenger.handler = handler;
}
@end

// What a FlutterView is to the plugin: where its rects are, from the native view's top left. It covers the native
// view, takes no pointer, and gives the keys it's given as first responder to the native view, its next responder.
@interface TextInputCoordinates : NSView
@end

@implementation TextInputCoordinates
- (BOOL)isFlipped {
  return YES;
}

- (NSView*)hitTest:(NSPoint)point {
  return nil;
}

- (BOOL)acceptsFirstResponder {
  return YES;
}
@end

@interface FlutterViewController ()
@property(nonatomic) TextInputCoordinates* coordinates;
@end

// It isn't the view's controller: it holds it weakly, as the view holds it, and isn't its next responder.
@implementation FlutterViewController {
  __weak NSView* _nativeView;
}

- (instancetype)initWithNativeView:(NSView*)view {
  self = [super initWithNibName:nil bundle:nil];
  _coordinates = [[TextInputCoordinates alloc] initWithFrame:view.bounds];
  _coordinates.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
  [view addSubview:_coordinates];
  _nativeView = view;
  return self;
}

- (NSView*)view {
  return _nativeView;
}

- (BOOL)isViewLoaded {
  return _nativeView != nil;
}

- (void)keyDown:(NSEvent*)event {
  [self.view keyDown:event];
}

- (void)keyUp:(NSEvent*)event {
  [self.view keyUp:event];
}

- (void)flagsChanged:(NSEvent*)event {
  [self.view flagsChanged:event];
}

- (void)mouseDown:(NSEvent*)event {
  [self.view mouseDown:event];
}

- (void)mouseUp:(NSEvent*)event {
  [self.view mouseUp:event];
}

- (void)mouseDragged:(NSEvent*)event {
  [self.view mouseDragged:event];
}

- (void)rightMouseDown:(NSEvent*)event {
  [self.view rightMouseDown:event];
}

- (void)rightMouseUp:(NSEvent*)event {
  [self.view rightMouseUp:event];
}

- (void)rightMouseDragged:(NSEvent*)event {
  [self.view rightMouseDragged:event];
}

- (void)otherMouseDown:(NSEvent*)event {
  [self.view otherMouseDown:event];
}

- (void)otherMouseUp:(NSEvent*)event {
  [self.view otherMouseUp:event];
}

- (void)otherMouseDragged:(NSEvent*)event {
  [self.view otherMouseDragged:event];
}

- (void)mouseMoved:(NSEvent*)event {
  [self.view mouseMoved:event];
}

- (void)scrollWheel:(NSEvent*)event {
  [self.view scrollWheel:event];
}
@end

@implementation FlutterViewController (Internal)
- (NSView*)flutterView {
  return self.coordinates;
}

// The menu bar has a key equivalent first, as it had before the plugin was the first responder: the native view's
// keyDown: gives the page what it doesn't take.
- (BOOL)isDispatchingKeyEvent:(NSEvent*)event {
  return YES;
}
@end

// The Edit menu's commands are the page's, as their keys are, not the plugin's NSTextView's: a password is never
// copied, and what's pasted is the page's to take.
@implementation FlutterTextInputPlugin (NativeEditMenu)
- (void)copy:(id)sender {
  [self.currentViewController.view tryToPerform:_cmd with:sender];
}

- (void)cut:(id)sender {
  [self.currentViewController.view tryToPerform:_cmd with:sender];
}

- (void)paste:(id)sender {
  [self.currentViewController.view tryToPerform:_cmd with:sender];
}

- (void)selectAll:(id)sender {
  [self.currentViewController.view tryToPerform:_cmd with:sender];
}

- (void)undo:(id)sender {
  [self.currentViewController.view tryToPerform:_cmd with:sender];
}

- (void)redo:(id)sender {
  [self.currentViewController.view tryToPerform:_cmd with:sender];
}
@end

// Flutter's engine, to the plugin: a window has one view.
@interface TextInput : NSObject <FlutterTextInputPluginDelegate>
@property(nonatomic, readonly) FlutterTextInputPlugin* plugin;
@property(nonatomic, readonly) FlutterViewController* controller;
@property(nonatomic, readonly) TextInputMessenger* messenger;
@end

@implementation TextInput
- (instancetype)initWithView:(NSView*)view context:(void*)context send:(TextInputSend)send {
  self = [super init];
  _messenger = [[TextInputMessenger alloc] init];
  _messenger.context = context;
  _messenger.send = send;
  _controller = [[FlutterViewController alloc] initWithNativeView:view];
  _plugin = [[FlutterTextInputPlugin alloc] initWithDelegate:self];
  return self;
}

- (FlutterViewController*)viewControllerForIdentifier:(FlutterViewIdentifier)viewIdentifier {
  return _controller;
}

- (id<FlutterBinaryMessenger>)binaryMessenger {
  return _messenger;
}
@end

extern "C" {

// A text input plugin for `view`, which says what it says to `send` with `context`.
void* text_input_new(NSView* view, void* context, TextInputSend send) {
  return (__bridge_retained void*)[[TextInput alloc] initWithView:view context:context send:send];
}

void text_input_free(void* input) {
  TextInput* text_input = (__bridge_transfer TextInput*)input;
  if (text_input.plugin.superview != nil) {
    [text_input.plugin removeFromSuperview];
  }
}

// A method call of Flutter's text input channel, with its arguments as JSON.
void text_input_call(void* input, const char* method, const char* arguments) {
  TextInput* text_input = (__bridge TextInput*)input;
  NSData* json = [NSData dataWithBytes:arguments length:strlen(arguments)];
  id decoded = [NSJSONSerialization JSONObjectWithData:json options:NSJSONReadingFragmentsAllowed error:nil];
  FlutterMethodCall* call =
      [FlutterMethodCall methodCallWithMethodName:@(method)
                                        arguments:decoded == [NSNull null] ? nil : decoded];
  text_input.messenger.handler(call, ^(id result) {
  });
}

// The plugin's input context handles `event`, a key: → whether it did.
bool text_input_key(void* input, NSEvent* event) {
  return [((__bridge TextInput*)input).plugin handleKeyEvent:event];
}

// Whether its input method is composing.
bool text_input_composing(void* input) {
  return [((__bridge TextInput*)input).plugin hasMarkedText];
}

}  // extern "C"
