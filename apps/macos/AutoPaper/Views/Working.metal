#include <metal_stdlib>
#include <SwiftUI/SwiftUI_Metal.h>
using namespace metal;

/// The wallpaper's slow ripple while a new one is being made (`WorkingEffect`). Two crossing waves move each pixel by
/// at most `amplitude` points, so the picture breathes rather than swims.
[[ stitchable ]] float2 workingRipple(float2 position, float time, float2 size, float amplitude) {
    float2 uv = position / max(size, float2(1.0));
    float swell = 0.65 + 0.35 * sin(uv.x * 3.1 + uv.y * 2.3 + time * 0.6);
    float2 offset = float2(
        sin(uv.y * 11.0 + time * 1.4) + 0.5 * sin(uv.x * 5.0 - time * 0.8),
        cos(uv.x * 9.0 + time * 1.1) + 0.5 * cos(uv.y * 4.0 + time * 0.7)
    );
    return position + offset * (amplitude * swell / 1.5);
}
