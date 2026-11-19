// What Flutter's semantics name of Skia's SkRect.h (SkRect, SkPoint), as Skia has them, for its iOS accessibility
// bridge (`accessibility.mm`).
#pragma once
#include <algorithm>
#include <span>

struct SkPoint {
  float fX = 0, fY = 0;
  static constexpr SkPoint Make(float x, float y) { return {x, y}; }
  float x() const { return fX; }
  float y() const { return fY; }
};

struct SkRect {
  float fLeft = 0, fTop = 0, fRight = 0, fBottom = 0;
  static constexpr SkRect MakeEmpty() { return {}; }
  static constexpr SkRect MakeXYWH(float x, float y, float w, float h) { return {x, y, x + w, y + h}; }
  bool operator==(const SkRect&) const = default;
  float x() const { return fLeft; }
  float y() const { return fTop; }
  float width() const { return fRight - fLeft; }
  float height() const { return fBottom - fTop; }
  bool setBoundsCheck(std::span<const SkPoint> points) {
    setBounds(points);
    return !points.empty();
  }
  void setBounds(std::span<const SkPoint> points) {
    if (points.empty()) {
      *this = {};
      return;
    }
    fLeft = fRight = points[0].fX;
    fTop = fBottom = points[0].fY;
    for (const SkPoint& point : points) {
      fLeft = std::min(fLeft, point.fX);
      fRight = std::max(fRight, point.fX);
      fTop = std::min(fTop, point.fY);
      fBottom = std::max(fBottom, point.fY);
    }
  }
};
