// A native window's screen reader on an iPhone: Flutter's iOS semantics objects (SemanticsObject.mm and the rest, in
// clients/app/renderer/flutter), each a UIAccessibilityElement VoiceOver reads and acts on, kept by a bridge of ours as
// Flutter's own AccessibilityBridge keeps them (accessibility_bridge.mm's UpdateSemantics, which `Update` follows), and
// a C interface to it for the window's Rust (`a11y.rs`): the screen's semantic tree comes in as Flutter's semantics
// nodes, as JSON (`accessibility_update`), and what VoiceOver asks of a node goes out as Flutter's semantics action (the
// `send` it's made with).

#include <algorithm>
#include <memory>
#include <vector>

#import "flutter/shell/platform/darwin/ios/framework/Source/FlutterSemanticsScrollView.h"
#import "flutter/shell/platform/darwin/ios/framework/Source/SemanticsObject.h"
#import "flutter/shell/platform/darwin/ios/framework/Source/TextInputSemanticsObject.h"

typedef void (*AccessibilitySend)(void* context, int32_t node, int32_t action);

// The text input plugin's view, whose text a text field's semantics object reads and edits (`text_input.mm`).
extern "C" UIView<UITextInput>* text_input_view(void* input);

@implementation FlutterTouchInterceptingView
- (void)setFlutterAccessibilityContainer:(NSObject*)flutterAccessibilityContainer {
}
@end

namespace {

constexpr int32_t kRootNodeId = 0;
constexpr int32_t kSemanticObjectIdInvalid = -1;

class Bridge final : public flutter::AccessibilityBridgeIos {
 public:
  Bridge(UIView* view, void* text_input, void* context, AccessibilitySend send)
      : view_(view), text_input_(text_input), context_(context), send_(send) {
    objects_ = [[NSMutableDictionary alloc] init];
  }

  ~Bridge() override {
    *self_ = nullptr;
    view_.accessibilityElements = nil;
  }

  UIView* view() const override { return view_; }
  UIView* ViewIfLoaded() const override { return view_; }
  UIView* AccessibilityElementInitializationContainer() const override { return view_; }
  bool isVoiceOverRunning() const override { return UIAccessibilityIsVoiceOverRunning(); }
  UIView<UITextInput>* textInputView() override { return text_input_view(text_input_); }

  void DispatchSemanticsAction(int32_t id, flutter::SemanticsAction action) override {
    send_(context_, id, static_cast<int32_t>(action));
  }

  void DispatchSemanticsAction(int32_t id, flutter::SemanticsAction action, fml::MallocMapping args) override {
    send_(context_, id, static_cast<int32_t>(action));
  }

  void AccessibilityObjectDidBecomeFocused(int32_t id) override { last_focused_ = id; }

  void AccessibilityObjectDidLoseFocus(int32_t id) override {
    if (last_focused_ == id) {
      last_focused_ = kSemanticObjectIdInvalid;
    }
  }

  NSString* GetDefaultLocale() override { return @"en"; }
  FlutterPlatformViewsController* GetPlatformViewsController() const override { return nil; }

  // The tree is now `nodes`, as Flutter's AccessibilityBridge::UpdateSemantics takes it.
  void Update(flutter::SemanticsNodeUpdates nodes) {
    BOOL layoutChanged = NO;
    BOOL scrollOccured = NO;
    BOOL needsAnnouncement = NO;
    for (const auto& entry : nodes) {
      const flutter::SemanticsNode& node = entry.second;
      SemanticsObject* object = GetOrCreateObject(node.id, nodes);
      layoutChanged = layoutChanged || [object nodeWillCauseLayoutChange:&node];
      scrollOccured = scrollOccured || [object nodeWillCauseScroll:&node];
      needsAnnouncement = [object nodeShouldTriggerAnnouncement:&node];
      [object setSemanticsNode:&node];
      NSMutableArray* children = [[NSMutableArray alloc] initWithCapacity:node.childrenInTraversalOrder.size()];
      for (int32_t child : node.childrenInTraversalOrder) {
        [children addObject:GetOrCreateObject(child, nodes)];
      }
      NSMutableArray* hitTestChildren = [[NSMutableArray alloc] initWithCapacity:node.childrenInHitTestOrder.size()];
      for (int32_t child : node.childrenInHitTestOrder) {
        [hitTestChildren addObject:GetOrCreateObject(child, nodes)];
      }
      object.children = children;
      object.childrenInHitTestOrder = hitTestChildren;
      if (needsAnnouncement) {
        NSString* announcement = [[NSString alloc] initWithUTF8String:object.node.label.c_str()];
        UIAccessibilityPostNotification(
            UIAccessibilityAnnouncementNotification,
            [[NSAttributedString alloc] initWithString:announcement
                                            attributes:@{UIAccessibilitySpeechAttributeQueueAnnouncement : @YES}]);
      }
    }

    SemanticsObject* root = objects_[@(kRootNodeId)];
    bool routeChanged = false;
    SemanticsObject* lastAdded = nil;
    if (root) {
      id container = [root accessibilityContainer];
      view_.accessibilityElements = container ? @[ container ] : nil;
      NSMutableArray<SemanticsObject*>* newRoutes = [[NSMutableArray alloc] init];
      [root collectRoutes:newRoutes];
      for (SemanticsObject* route in newRoutes) {
        if (std::find(previous_routes_.begin(), previous_routes_.end(), [route uid]) == previous_routes_.end()) {
          lastAdded = route;
        }
      }
      if (lastAdded == nil && [newRoutes count] > 0) {
        lastAdded = [newRoutes lastObject];
      }
      if (lastAdded != nil &&
          ([lastAdded uid] != previous_route_id_ || [newRoutes count] != previous_routes_.size())) {
        previous_route_id_ = [lastAdded uid];
        routeChanged = true;
      }
      previous_routes_.clear();
      for (SemanticsObject* route in newRoutes) {
        previous_routes_.push_back([route uid]);
      }
    } else {
      view_.accessibilityElements = nil;
    }

    NSMutableArray<NSNumber*>* doomed = [NSMutableArray arrayWithArray:objects_.allKeys];
    if (root) {
      Keep(root, doomed);
    }
    [objects_ removeObjectsForKeys:doomed];
    for (SemanticsObject* object in objects_.allValues) {
      [object accessibilityBridgeDidFinishUpdate];
    }

    layoutChanged = layoutChanged || [doomed count] > 0;
    if (routeChanged) {
      UIAccessibilityPostNotification(UIAccessibilityScreenChangedNotification, [lastAdded routeName]);
    }
    if (layoutChanged) {
      SemanticsObject* next = FindNextFocusable();
      SemanticsObject* lastFocused = objects_[@(last_focused_)];
      UIAccessibilityPostNotification(UIAccessibilityLayoutChangedNotification,
                                      (routeChanged || next != lastFocused) ? next.nativeAccessibility : nil);
    } else if (scrollOccured) {
      UIAccessibilityPostNotification(UIAccessibilityPageScrolledNotification,
                                      FindNextFocusable().nativeAccessibility);
    }
  }

 private:
  fml::WeakPtr<flutter::AccessibilityBridgeIos> Weak() { return fml::WeakPtr<flutter::AccessibilityBridgeIos>(self_); }

  SemanticsObject* Create(const flutter::SemanticsNode& node) {
    if (node.flags.isTextField && !node.flags.isReadOnly) {
      return [[TextInputSemanticsObject alloc] initWithBridge:Weak() uid:node.id];
    } else if (!node.flags.isInMutuallyExclusiveGroup &&
               (node.flags.isToggled != flutter::SemanticsTristate::kNone ||
                node.flags.isChecked != flutter::SemanticsCheckState::kNone)) {
      return [[FlutterSwitchSemanticsObject alloc] initWithBridge:Weak() uid:node.id];
    } else if (node.flags.hasImplicitScrolling) {
      return [[FlutterScrollableSemanticsObject alloc] initWithBridge:Weak() uid:node.id];
    }
    return [[FlutterSemanticsObject alloc] initWithBridge:Weak() uid:node.id];
  }

  SemanticsObject* GetOrCreateObject(int32_t uid, flutter::SemanticsNodeUpdates& updates) {
    SemanticsObject* object = objects_[@(uid)];
    if (!object) {
      object = Create(updates[uid]);
      objects_[@(uid)] = object;
      return object;
    }
    auto entry = updates.find(object.node.id);
    if (entry == updates.end()) {
      return object;
    }
    const flutter::SemanticsNode& node = entry->second;
    // A node whose kind changed is a new object, as Flutter's bridge replaces it.
    if (object.node.flags.isTextField != node.flags.isTextField ||
        object.node.flags.isReadOnly != node.flags.isReadOnly ||
        (object.node.flags.isChecked == flutter::SemanticsCheckState::kNone) !=
            (node.flags.isChecked == flutter::SemanticsCheckState::kNone) ||
        (object.node.flags.isToggled == flutter::SemanticsTristate::kNone) !=
            (node.flags.isToggled == flutter::SemanticsTristate::kNone) ||
        object.node.flags.hasImplicitScrolling != node.flags.hasImplicitScrolling) {
      SemanticsObject* replacement = Create(node);
      NSUInteger at = [object.parent.children indexOfObject:object];
      object.children = @[];
      if (object.parent && at != NSNotFound) {
        [object.parent replaceChildAtIndex:at withChild:replacement];
      }
      objects_[@(uid)] = replacement;
      object = replacement;
    }
    return object;
  }

  void Keep(SemanticsObject* object, NSMutableArray<NSNumber*>* doomed) {
    [doomed removeObject:@(object.uid)];
    for (SemanticsObject* child in [object children]) {
      Keep(child, doomed);
    }
  }

  SemanticsObject* FindNextFocusable() {
    if (last_focused_ == kSemanticObjectIdInvalid) {
      return nil;
    }
    return FindFirstFocusable(objects_[@(last_focused_)]);
  }

  SemanticsObject* FindFirstFocusable(SemanticsObject* parent) {
    SemanticsObject* current = parent ?: objects_[@(kRootNodeId)];
    if (!current) {
      return nil;
    }
    if (current.isAccessibilityElement) {
      return current;
    }
    for (SemanticsObject* child in [current children]) {
      if (SemanticsObject* found = FindFirstFocusable(child)) {
        return found;
      }
    }
    return nil;
  }

  // What its semantics objects reach it by, null once it's gone.
  std::shared_ptr<flutter::AccessibilityBridgeIos*> self_ = std::make_shared<flutter::AccessibilityBridgeIos*>(this);
  __weak UIView* view_;
  void* text_input_;
  void* context_;
  AccessibilitySend send_;
  NSMutableDictionary<NSNumber*, SemanticsObject*>* objects_;
  int32_t last_focused_ = kSemanticObjectIdInvalid;
  int32_t previous_route_id_ = 0;
  std::vector<int32_t> previous_routes_;
};

std::string Text(NSDictionary* node, NSString* key) {
  NSString* text = node[key];
  return [text isKindOfClass:[NSString class]] ? std::string(text.UTF8String) : std::string();
}

bool Flag(NSDictionary* flags, NSString* key) {
  return [flags[key] boolValue];
}

// A node as `a11y.rs` gives it, as Flutter's semantics node.
flutter::SemanticsNode Node(NSDictionary* json) {
  flutter::SemanticsNode node;
  node.id = [json[@"id"] intValue];
  node.label = Text(json, @"label");
  node.value = Text(json, @"value");
  node.hint = Text(json, @"hint");
  node.actions = [json[@"actions"] intValue];
  node.headingLevel = [json[@"heading"] intValue];
  node.textDirection = 2;
  NSArray* rect = json[@"rect"];
  if ([rect isKindOfClass:[NSArray class]] && rect.count == 4) {
    node.rect = SkRect::MakeXYWH([rect[0] floatValue], [rect[1] floatValue], [rect[2] floatValue], [rect[3] floatValue]);
  }
  for (NSNumber* child in json[@"children"]) {
    node.childrenInTraversalOrder.push_back(child.intValue);
  }
  node.childrenInHitTestOrder.assign(node.childrenInTraversalOrder.rbegin(), node.childrenInTraversalOrder.rend());
  NSDictionary* flags = json[@"flags"];
  node.flags.isButton = Flag(flags, @"button");
  node.flags.isHeader = Flag(flags, @"header");
  node.flags.isLink = Flag(flags, @"link");
  node.flags.isImage = Flag(flags, @"image");
  node.flags.isTextField = Flag(flags, @"textField");
  node.flags.isObscured = Flag(flags, @"obscured");
  node.flags.isMultiline = Flag(flags, @"multiline");
  node.flags.isLiveRegion = Flag(flags, @"liveRegion");
  node.flags.scopesRoute = Flag(flags, @"scopesRoute");
  node.flags.namesRoute = Flag(flags, @"namesRoute");
  node.flags.isInMutuallyExclusiveGroup = Flag(flags, @"mutuallyExclusive");
  node.flags.isChecked = static_cast<flutter::SemanticsCheckState>([flags[@"checked"] intValue]);
  node.flags.isToggled = static_cast<flutter::SemanticsTristate>([flags[@"toggled"] intValue]);
  node.flags.isSelected = static_cast<flutter::SemanticsTristate>([flags[@"selected"] intValue]);
  node.flags.isEnabled = static_cast<flutter::SemanticsTristate>([flags[@"enabled"] intValue]);
  node.flags.isFocused = static_cast<flutter::SemanticsTristate>([flags[@"focused"] intValue]);
  return node;
}

}  // namespace

extern "C" {

// A screen reader's bridge for `view`, which says what VoiceOver asks to `send` with `context`; `text_input` is the
// window's text input plugin (`text_input_new`).
void* accessibility_new(UIView* view, void* text_input, void* context, AccessibilitySend send) {
  return new Bridge(view, text_input, context, send);
}

void accessibility_free(void* bridge) {
  delete static_cast<Bridge*>(bridge);
}

// The window's semantic tree, as JSON: an array of nodes, the window's own first (id 0).
void accessibility_update(void* bridge, const char* json) {
  NSData* data = [NSData dataWithBytes:json length:strlen(json)];
  NSArray* nodes = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  flutter::SemanticsNodeUpdates updates;
  for (NSDictionary* node in nodes) {
    flutter::SemanticsNode converted = Node(node);
    updates[converted.id] = converted;
  }
  static_cast<Bridge*>(bridge)->Update(std::move(updates));
}

// Whether a screen reader is listening, as Flutter's view controller asks.
bool accessibility_running() {
  return UIAccessibilityIsVoiceOverRunning() || UIAccessibilityIsSwitchControlRunning();
}

}  // extern "C"
