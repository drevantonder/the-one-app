// Ours, in place of Flutter's FlutterCodecs.h and FlutterChannels.h, for what FlutterTextInputPlugin uses of them: a
// method channel whose calls go to and come from the native host's Rust as JSON (`platform/mac/text_input.mm`).

#import <Foundation/Foundation.h>

#import "flutter/shell/platform/darwin/common/framework/Headers/FlutterBinaryMessenger.h"

typedef void (^FlutterResult)(id _Nullable result);

extern NSObject const* _Nonnull FlutterMethodNotImplemented;

@interface FlutterMethodCall : NSObject
+ (nonnull instancetype)methodCallWithMethodName:(nonnull NSString*)method arguments:(nullable id)arguments;
@property(readonly, nonatomic, nonnull) NSString* method;
@property(readonly, nonatomic, nullable) id arguments;
@end

typedef void (^FlutterMethodCallHandler)(FlutterMethodCall* _Nonnull call, FlutterResult _Nonnull result);

@interface FlutterError : NSObject
+ (nonnull instancetype)errorWithCode:(nonnull NSString*)code
                              message:(nullable NSString*)message
                              details:(nullable id)details;
@end

@interface FlutterJSONMethodCodec : NSObject
+ (nonnull instancetype)sharedInstance;
@end

@interface FlutterMethodChannel : NSObject
+ (nonnull instancetype)methodChannelWithName:(nonnull NSString*)name
                              binaryMessenger:(nonnull NSObject<FlutterBinaryMessenger>*)messenger
                                        codec:(nonnull FlutterJSONMethodCodec*)codec;
- (void)invokeMethod:(nonnull NSString*)method arguments:(nullable id)arguments;
- (void)setMethodCallHandler:(nullable FlutterMethodCallHandler)handler;
@end
