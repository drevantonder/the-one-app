// What Flutter's semantics name of Skia's SkM44.h (SkM44, SkV4), as Skia has them: a 4×4 matrix, column-major, for
// its iOS accessibility bridge (`accessibility.mm`).
#pragma once

struct SkV4 {
  float x, y, z, w;
};

class SkM44 {
 public:
  constexpr SkM44() : fMat{1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1} {}

  bool operator==(const SkM44&) const = default;

  SkM44 operator*(const SkM44& b) const {
    SkM44 out;
    for (int c = 0; c < 4; c++) {
      for (int r = 0; r < 4; r++) {
        float sum = 0;
        for (int k = 0; k < 4; k++) {
          sum += fMat[k * 4 + r] * b.fMat[c * 4 + k];
        }
        out.fMat[c * 4 + r] = sum;
      }
    }
    return out;
  }

  SkV4 map(float x, float y, float z, float w) const {
    float v[4] = {x, y, z, w}, out[4];
    for (int r = 0; r < 4; r++) {
      out[r] = fMat[r] * v[0] + fMat[4 + r] * v[1] + fMat[8 + r] * v[2] + fMat[12 + r] * v[3];
    }
    return {out[0], out[1], out[2], out[3]};
  }

  float rc(int r, int c) const { return fMat[c * 4 + r]; }

 private:
  float fMat[16];
};
