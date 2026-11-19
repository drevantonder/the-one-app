// Ours, in place of Flutter's FlutterPlatformViews_Internal.h, which its semantics objects import for a native view
// embedded in the page, which a native window has none of.
#pragma once
#import <UIKit/UIKit.h>

@class FlutterPlatformViewsController;

@interface FlutterTouchInterceptingView : UIView
- (void)setFlutterAccessibilityContainer:(NSObject*)flutterAccessibilityContainer;
@end
