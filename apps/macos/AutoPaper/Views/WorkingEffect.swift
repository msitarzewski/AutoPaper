import AutopaperCore
import CoreGraphics
import SwiftUI

/// While a new wallpaper is being made, the one showing looks like it's being remade: it ripples gently (a Metal
/// distortion, `Working.metal`) and a slow smoke in its own colours drifts over it. Reduce Motion: a still veil,
/// nothing moves. The motion runs only while making one, and it's decorative to accessibility. The stage goes on the
/// picture in a `StageCapsule`, added by the caller outside the image's own accessibility element (so VoiceOver reads
/// it on its own).
struct WorkingEffect: ViewModifier {
    let active: Bool
    /// The picture's colours (3 × 3 regions), for the smoke; nil until measured.
    let palette: [Color]?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var started = Date.now

    func body(content: Content) -> some View {
        if active {
            TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: reduceMotion)) { timeline in
                let time = reduceMotion ? 0 : Float(timeline.date.timeIntervalSince(started))
                let amplitude: Float = reduceMotion ? 0 : 2.5
                content
                    .saturation(0.85)
                    .visualEffect { view, proxy in
                        view.distortionEffect(
                            ShaderLibrary.workingRipple(.float(time), .float2(proxy.size), .float(amplitude)),
                            maxSampleOffset: CGSize(width: 4, height: 4)
                        )
                    }
                    .overlay {
                        Smoke(palette: palette, time: Double(time))
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                    }
            }
            .onAppear { started = .now }
        } else {
            content
        }
    }
}

/// Drifting light in the picture's own colours: a 3 × 3 mesh whose inner points wander, lightened and blurred,
/// screened over the image.
private struct Smoke: View {
    let palette: [Color]?
    let time: Double

    var body: some View {
        let colors = palette ?? Array(repeating: Color.white, count: 9)
        MeshGradient(width: 3, height: 3, points: points, colors: colors)
            .blur(radius: 28)
            .blendMode(.screen)
            .opacity(palette == nil ? 0.25 : 0.55)
    }

    /// Corners and edges stay put; the centre and edge midpoints drift slowly on their own circles.
    private var points: [SIMD2<Float>] {
        func drift(_ x: Float, _ y: Float, _ phase: Double, _ reach: Float) -> SIMD2<Float> {
            SIMD2(x + reach * Float(sin(time * 0.45 + phase)), y + reach * Float(cos(time * 0.38 + phase * 1.3)))
        }
        return [
            [0, 0], drift(0.5, 0, 0.0, 0.18) * [1, 0], [1, 0],
            drift(0, 0.5, 1.1, 0.18) * [0, 1], drift(0.5, 0.5, 2.3, 0.24), drift(1, 0.5, 3.7, 0.18) * [0, 1] + [1, 0],
            [0, 1], drift(0.5, 1, 4.9, 0.18) * [1, 0] + [0, 1], [1, 1],
        ]
    }
}

/// The stage, on the picture in a Liquid Glass capsule (the system's material, so Reduce Transparency and Increase
/// Contrast apply): a ring that fills as the painting goes (when the engine has real numbers: ComfyUI's steps, or how
/// long this painter took here before; otherwise a spinner) and "About 6 minutes left" under the stage. Stopping is
/// the toolbar's Stop (Esc, user 2026-10-06), the window's one control for it; ⌘. and the menu bar menu also cancel.
struct StageCapsule: View {
    let work: WorkPresentation

    var body: some View {
        HStack(spacing: 10) {
            WorkIndicator(work: work)
            VStack(alignment: .leading, spacing: 1) {
                Text(work.stage)
                    .accessibilityAddTraits(.updatesFrequently)
                if let timeLeft = work.timeLeft {
                    // Spoken with the ring ("40 percent, about 6 minutes left"), so not twice.
                    Text(timeLeft)
                        .font(.callout)
                        .monospacedDigit()
                        .accessibilityHidden(true)
                }
            }
            .fixedSize()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .glassEffect(.regular, in: .capsule)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Making a new wallpaper")
    }
}

/// A ring that fills with the painting's progress when the engine has a real number for it, else the system's
/// spinner. The ring is spoken as "Progress, 40 percent, about 6 minutes left"; the spinner is decorative (the stage
/// beside it says what's happening).
struct WorkIndicator: View {
    let work: WorkPresentation

    var body: some View {
        if let fraction = work.fraction {
            ProgressView(value: Double(fraction))
                .progressViewStyle(.circular)
                .controlSize(.small)
                .accessibilityLabel("Progress")
                .accessibilityValue(work.spokenProgress ?? Formatting.percent(fraction))
        } else {
            ProgressView()
                .controlSize(.small)
                .accessibilityHidden(true)
        }
    }
}

/// The average colour of each of 3 × 3 regions of a picture, lightened for smoke, row by row (top first).
enum ImagePalette {
    static func colors(of image: CGImage) -> [Color]? {
        let side = 3
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(
                data: buffer.baseAddress, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return false }
            context.interpolationQuality = .medium
            context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
            return true
        }
        guard drawn else { return nil }
        // A bitmap context's memory starts at the picture's top row, which is the order the mesh wants.
        return (0..<side).flatMap { row in
            (0..<side).map { column in
                let i = (row * side + column) * 4
                let lift = 0.35 // toward white: smoke, not paint
                func channel(_ value: UInt8) -> Double { Double(value) / 255 * (1 - lift) + lift }
                return Color(.sRGB, red: channel(pixels[i]), green: channel(pixels[i + 1]), blue: channel(pixels[i + 2]))
            }
        }
    }
}

extension View {
    /// See `WorkingEffect`.
    func working(_ active: Bool, palette: [Color]?) -> some View {
        modifier(WorkingEffect(active: active, palette: palette))
    }
}
