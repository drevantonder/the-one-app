// What an iPhone does for a native window's page itself (`Device` in mod.rs): its picker, the pop-up menu a `<select>`
// opens in Safari, from a button over the picker's box; its share sheet; its haptics; its text size; and a tap on
// its status bar.

#include <string>

#import <UIKit/UIKit.h>

typedef void (*DevicePicked)(void* context, int64_t node, const char* value);

// A button over a picker's box whose menu is the picker, which goes as its menu does.
@interface OnePicker : UIButton
@end

@implementation OnePicker
- (void)contextMenuInteraction:(UIContextMenuInteraction*)interaction
       willEndForConfiguration:(UIContextMenuConfiguration*)configuration
                      animator:(id<UIContextMenuInteractionAnimating>)animator {
  [super contextMenuInteraction:interaction willEndForConfiguration:configuration animator:animator];
  [animator addCompletion:^{
    [self removeFromSuperview];
  }];
}
@end

// Shows the picker for picker node `node` of the window `context` over `view`, at its box (in points), of `options`
// (JSON: each its `value`, `label` and whether it's `disabled`), `chosen` the one chosen or NULL. What's chosen goes to
// `picked`. → whether it showed.
extern "C" bool device_pick(UIView* view, double x, double y, double width, double height, const char* options, const char* chosen, void* context, int64_t node, DevicePicked picked) {
  NSData* json = [NSData dataWithBytes:options length:strlen(options)];
  NSArray* all = [NSJSONSerialization JSONObjectWithData:json options:0 error:nil];
  if (![all isKindOfClass:[NSArray class]]) return false;
  NSString* now = chosen ? [NSString stringWithUTF8String:chosen] : nil;
  NSMutableArray<UIMenuElement*>* actions = [NSMutableArray array];
  for (NSDictionary* option in all) {
    if (![option isKindOfClass:[NSDictionary class]]) continue;
    NSString* value = [option[@"value"] description];
    NSString* label = [option[@"label"] isKindOfClass:[NSString class]] ? option[@"label"] : value;
    std::string held = value.UTF8String;
    UIAction* action = [UIAction actionWithTitle:label image:nil identifier:nil handler:^(UIAction*) {
      picked(context, node, held.c_str());
    }];
    if ([value isEqualToString:now]) action.state = UIMenuElementStateOn;
    if ([option[@"disabled"] boolValue]) action.attributes = UIMenuElementAttributesDisabled;
    [actions addObject:action];
  }
  OnePicker* button = [OnePicker buttonWithType:UIButtonTypeCustom];
  button.frame = CGRectMake(x, y, width, height);
  button.menu = [UIMenu menuWithTitle:@"" image:nil identifier:nil options:UIMenuOptionsSingleSelection children:actions];
  button.showsMenuAsPrimaryAction = YES;
  [view addSubview:button];
  if (@available(iOS 17.4, *)) {
    [button performPrimaryAction];
    return true;
  }
  [button removeFromSuperview];
  return false;
}

// The share sheet for `text`, over `view`'s window.
extern "C" bool device_share(UIView* view, const char* text) {
  UIViewController* over = view.window.rootViewController;
  while (over.presentedViewController) over = over.presentedViewController;
  if (!over) return false;
  UIActivityViewController* sheet = [[UIActivityViewController alloc] initWithActivityItems:@[ [NSString stringWithUTF8String:text] ] applicationActivities:nil];
  sheet.popoverPresentationController.sourceView = view;
  [over presentViewController:sheet animated:YES completion:nil];
  return true;
}

// A haptic, by its name: an impact's style, a selection, or a notification's outcome. → whether there's one by it.
extern "C" bool device_haptic(const char* kind) {
  NSString* named = [NSString stringWithUTF8String:kind];
  NSDictionary<NSString*, NSNumber*>* impacts = @{
    @"light" : @(UIImpactFeedbackStyleLight),
    @"medium" : @(UIImpactFeedbackStyleMedium),
    @"heavy" : @(UIImpactFeedbackStyleHeavy),
    @"rigid" : @(UIImpactFeedbackStyleRigid),
    @"soft" : @(UIImpactFeedbackStyleSoft),
  };
  NSDictionary<NSString*, NSNumber*>* outcomes = @{
    @"success" : @(UINotificationFeedbackTypeSuccess),
    @"warning" : @(UINotificationFeedbackTypeWarning),
    @"error" : @(UINotificationFeedbackTypeError),
  };
  if (impacts[named]) {
    [[[UIImpactFeedbackGenerator alloc] initWithStyle:(UIImpactFeedbackStyle)impacts[named].integerValue] impactOccurred];
  } else if (outcomes[named]) {
    [[[UINotificationFeedbackGenerator alloc] init] notificationOccurred:(UINotificationFeedbackType)outcomes[named].integerValue];
  } else if ([named isEqualToString:@"selection"]) {
    [[[UISelectionFeedbackGenerator alloc] init] selectionChanged];
  } else {
    return false;
  }
  return true;
}

// The text size its person chose (Dynamic Type), as its body text's size in points.
extern "C" double device_text_size() {
  return [UIFont preferredFontForTextStyle:UIFontTextStyleBody].pointSize;
}

typedef void (*DeviceTapped)(void* context);

// A scroll view a point high along a window's top, which iOS asks as its status bar is tapped (`scrollsToTop`): it
// says no, and tells the window, whose page scrolls to its top itself.
@interface OneStatusBar : UIScrollView <UIScrollViewDelegate>
@property(nonatomic) void* context;
@property(nonatomic) DeviceTapped tapped;
@end

@implementation OneStatusBar
- (BOOL)scrollViewShouldScrollToTop:(UIScrollView*)scrollView {
  self.tapped(self.context);
  return NO;
}
@end

extern "C" void device_status_bar(UIView* view, void* context, DeviceTapped tapped) {
  OneStatusBar* bar = [[OneStatusBar alloc] initWithFrame:CGRectMake(0, 0, view.bounds.size.width, 1)];
  bar.autoresizingMask = UIViewAutoresizingFlexibleWidth;
  bar.contentSize = CGSizeMake(view.bounds.size.width, 3);
  bar.contentOffset = CGPointMake(0, 1);
  bar.scrollsToTop = YES;
  bar.context = context;
  bar.tapped = tapped;
  bar.delegate = bar;
  [view addSubview:bar];
}
