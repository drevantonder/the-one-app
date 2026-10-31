import SwiftUI

/// A shape from SVG path data in a 24-point box, scaled to fit its frame.
struct SVGShape: Shape {
    let path: Path

    init(_ data: String) { path = parseSVGPath(data) }

    func path(in rect: CGRect) -> Path {
        path.applying(CGAffineTransform(scaleX: rect.width / 24, y: rect.height / 24).translatedBy(x: rect.minX, y: rect.minY))
    }
}

/// Reads SVG path data: M L H V C S Q T A Z, absolute and relative.
func parseSVGPath(_ data: String) -> Path {
    var p = Path()
    let chars = Array(data.utf8)
    var i = 0
    var cmd: UInt8 = 0
    var cur = CGPoint.zero, start = CGPoint.zero, ctrl = CGPoint.zero
    var last: UInt8 = 0

    func skip() { while i < chars.count, chars[i] == 32 || chars[i] == 44 || chars[i] == 9 || chars[i] == 10 || chars[i] == 13 { i += 1 } }
    func number() -> CGFloat? {
        skip()
        let begin = i
        if i < chars.count, chars[i] == 45 || chars[i] == 43 { i += 1 }
        var dot = false, digits = false
        while i < chars.count {
            let c = chars[i]
            if c >= 48 && c <= 57 { digits = true; i += 1 }
            else if c == 46 && !dot { dot = true; i += 1 }
            else if (c == 101 || c == 69) && digits {
                i += 1
                if i < chars.count, chars[i] == 45 || chars[i] == 43 { i += 1 }
            } else { break }
        }
        guard digits, let v = Double(String(decoding: chars[begin..<i], as: UTF8.self)) else { i = begin; return nil }
        return CGFloat(v)
    }
    // Arc flags may be written without separators, as in "0 01".
    func flag() -> Bool? {
        skip()
        guard i < chars.count, chars[i] == 48 || chars[i] == 49 else { return nil }
        defer { i += 1 }
        return chars[i] == 49
    }

    while true {
        skip()
        guard i < chars.count else { break }
        let c = chars[i]
        if (c >= 65 && c <= 90) || (c >= 97 && c <= 122) {
            cmd = c
            i += 1
            if c == 90 || c == 122 {
                p.closeSubpath()
                cur = start
                last = c
                continue
            }
        } else if cmd == 0 {
            break
        }
        let rel = cmd >= 97
        let o = rel ? cur : .zero
        switch cmd | 32 {
        case 109:  // m
            guard let x = number(), let y = number() else { return p }
            cur = CGPoint(x: o.x + x, y: o.y + y)
            start = cur
            p.move(to: cur)
            cmd = rel ? 108 : 76  // further pairs are lines
        case 108:
            guard let x = number(), let y = number() else { return p }
            cur = CGPoint(x: o.x + x, y: o.y + y)
            p.addLine(to: cur)
        case 104:
            guard let x = number() else { return p }
            cur.x = rel ? cur.x + x : x
            p.addLine(to: cur)
        case 118:
            guard let y = number() else { return p }
            cur.y = rel ? cur.y + y : y
            p.addLine(to: cur)
        case 99:
            guard let x1 = number(), let y1 = number(), let x2 = number(), let y2 = number(), let x = number(), let y = number() else { return p }
            let c1 = CGPoint(x: o.x + x1, y: o.y + y1)
            ctrl = CGPoint(x: o.x + x2, y: o.y + y2)
            cur = CGPoint(x: o.x + x, y: o.y + y)
            p.addCurve(to: cur, control1: c1, control2: ctrl)
        case 115:
            guard let x2 = number(), let y2 = number(), let x = number(), let y = number() else { return p }
            let smooth = [99, 115].contains(last | 32)
            let c1 = smooth ? CGPoint(x: 2 * cur.x - ctrl.x, y: 2 * cur.y - ctrl.y) : cur
            ctrl = CGPoint(x: o.x + x2, y: o.y + y2)
            cur = CGPoint(x: o.x + x, y: o.y + y)
            p.addCurve(to: cur, control1: c1, control2: ctrl)
        case 113:
            guard let x1 = number(), let y1 = number(), let x = number(), let y = number() else { return p }
            ctrl = CGPoint(x: o.x + x1, y: o.y + y1)
            cur = CGPoint(x: o.x + x, y: o.y + y)
            p.addQuadCurve(to: cur, control: ctrl)
        case 116:
            guard let x = number(), let y = number() else { return p }
            ctrl = [113, 116].contains(last | 32) ? CGPoint(x: 2 * cur.x - ctrl.x, y: 2 * cur.y - ctrl.y) : cur
            cur = CGPoint(x: o.x + x, y: o.y + y)
            p.addQuadCurve(to: cur, control: ctrl)
        case 97:
            guard let rx = number(), let ry = number(), let rot = number(), let large = flag(), let sweep = flag(),
                  let x = number(), let y = number() else { return p }
            let end = CGPoint(x: o.x + x, y: o.y + y)
            addArc(&p, from: cur, to: end, rx: abs(rx), ry: abs(ry), rotation: rot, large: large, sweep: sweep)
            cur = end
        default:
            return p
        }
        last = cmd
    }
    return p
}

/// An SVG elliptical arc, drawn as cubic curves.
private func addArc(_ p: inout Path, from a: CGPoint, to b: CGPoint, rx: CGFloat, ry: CGFloat, rotation: CGFloat, large: Bool, sweep: Bool) {
    if a == b { return }
    var rx = rx, ry = ry
    if rx == 0 || ry == 0 { p.addLine(to: b); return }
    let phi = rotation * .pi / 180, cosφ = cos(phi), sinφ = sin(phi)
    let dx = (a.x - b.x) / 2, dy = (a.y - b.y) / 2
    let x1 = cosφ * dx + sinφ * dy, y1 = -sinφ * dx + cosφ * dy
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry)
    if lambda > 1 { rx *= sqrt(lambda); ry *= sqrt(lambda) }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1
    var k = sqrt(max(0, num / den))
    if large == sweep { k = -k }
    let cx1 = k * rx * y1 / ry, cy1 = -k * ry * x1 / rx
    let cx = cosφ * cx1 - sinφ * cy1 + (a.x + b.x) / 2, cy = sinφ * cx1 + cosφ * cy1 + (a.y + b.y) / 2
    func angle(_ ux: CGFloat, _ uy: CGFloat, _ vx: CGFloat, _ vy: CGFloat) -> CGFloat {
        let sign: CGFloat = ux * vy - uy * vx < 0 ? -1 : 1
        let dot = (ux * vx + uy * vy) / (sqrt(ux * ux + uy * uy) * sqrt(vx * vx + vy * vy))
        return sign * acos(min(1, max(-1, dot)))
    }
    let t1 = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry)
    var dt = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry)
    if !sweep && dt > 0 { dt -= 2 * .pi } else if sweep && dt < 0 { dt += 2 * .pi }
    let segments = Int(ceil(abs(dt) / (.pi / 2)))
    let step = dt / CGFloat(segments)
    let alpha = 4 / 3 * tan(step / 4)
    var t = t1
    func point(_ t: CGFloat) -> CGPoint {
        CGPoint(x: cx + rx * cos(t) * cosφ - ry * sin(t) * sinφ, y: cy + rx * cos(t) * sinφ + ry * sin(t) * cosφ)
    }
    func derivative(_ t: CGFloat) -> CGPoint {
        CGPoint(x: -rx * sin(t) * cosφ - ry * cos(t) * sinφ, y: -rx * sin(t) * sinφ + ry * cos(t) * cosφ)
    }
    for _ in 0..<segments {
        let p1 = point(t), p2 = point(t + step), d1 = derivative(t), d2 = derivative(t + step)
        p.addCurve(to: p2, control1: CGPoint(x: p1.x + alpha * d1.x, y: p1.y + alpha * d1.y),
                   control2: CGPoint(x: p2.x - alpha * d2.x, y: p2.y - alpha * d2.y))
        t += step
    }
}
