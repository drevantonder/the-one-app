// What Flutter's engine makes in Swift that its iOS text input plugin names: its logger, which `text_input.mm` says
// to the system's log.

#import <Foundation/Foundation.h>

@interface FlutterLogger : NSObject
+ (void)logError:(NSString*)message;
@end
