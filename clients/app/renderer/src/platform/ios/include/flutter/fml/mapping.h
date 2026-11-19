// Ours, in place of Flutter's fml/mapping.h, for its iOS accessibility bridge: the bytes an action's arguments are.
#pragma once
#include <cstdint>
#include <cstring>
#include <vector>

namespace fml {

class MallocMapping {
 public:
  MallocMapping() = default;
  static MallocMapping Copy(const void* begin, size_t length) {
    MallocMapping mapping;
    auto bytes = static_cast<const uint8_t*>(begin);
    mapping.data_.assign(bytes, bytes + length);
    return mapping;
  }
  const uint8_t* GetMapping() const { return data_.data(); }
  size_t GetSize() const { return data_.size(); }

 private:
  std::vector<uint8_t> data_;
};

}  // namespace fml
