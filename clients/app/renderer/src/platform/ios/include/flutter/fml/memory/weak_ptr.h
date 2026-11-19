// Ours, in place of Flutter's fml/memory/weak_ptr.h, for its iOS accessibility bridge: a pointer to the window's bridge
// (`accessibility.mm`) that's null once the bridge is gone, as a semantics object VoiceOver still holds can outlive it.
#pragma once

#include <memory>

#include "flutter/fml/logging.h"

namespace fml {

template <typename T>
class WeakPtr {
 public:
  WeakPtr() = default;
  // `cell` points at the object while it lives; it sets it null as it goes.
  explicit WeakPtr(std::shared_ptr<T*> cell) : cell_(std::move(cell)) {}
  T* get() const { return cell_ ? *cell_ : nullptr; }
  explicit operator bool() const { return get() != nullptr; }
  T* operator->() const { return get(); }
  T& operator*() const { return *get(); }

 private:
  std::shared_ptr<T*> cell_;
};

}  // namespace fml
