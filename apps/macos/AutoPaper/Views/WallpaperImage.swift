import AutopaperCore
import ImageIO
import SwiftUI

/// A wallpaper image from disk, decoded off the main thread at no more pixels than it's shown at (ImageIO
/// thumbnailing). Tries each path in order and shows the first that loads: the original when it's still on disk,
/// else the thumbnail (kept when the storage limit prunes the original). Decorative to accessibility: the view
/// that holds it gives the spoken description.
struct WallpaperImage: View {
    /// Files to try, best first.
    let paths: [String]
    /// Width ÷ height of the picture, for the frame before it loads.
    let aspectRatio: CGFloat
    var contentMode: ContentMode = .fit
    var maxPixelSize: Int = 1_600

    @State private var image: LoadedImage?

    var body: some View {
        ZStack {
            if let image {
                Image(decorative: image.cgImage, scale: 1)
                    .resizable()
                    .aspectRatio(contentMode: contentMode)
            } else {
                Rectangle()
                    .fill(.quaternary)
                    .overlay {
                        Image(systemName: "photo")
                            .font(.title)
                            .foregroundStyle(.secondary)
                    }
            }
        }
        .aspectRatio(aspectRatio, contentMode: contentMode)
        .task(id: paths) {
            image = await LoadedImage.load(paths, maxPixelSize: maxPixelSize)
        }
    }
}

extension WallpaperImage {
    /// The original first (sharp at large sizes), then the 640-pixel thumbnail.
    init(_ generation: Generation, contentMode: ContentMode = .fit, maxPixelSize: Int = 1_600, preferThumbnail: Bool = false) {
        let original = generation.imagePath.map { [$0] } ?? []
        let thumb = generation.thumbPath.map { [$0] } ?? []
        paths = preferThumbnail ? thumb + original : original + thumb
        let ratio = generation.height > 0 ? CGFloat(generation.width) / CGFloat(generation.height) : 16 / 9
        aspectRatio = ratio
        self.contentMode = contentMode
        self.maxPixelSize = maxPixelSize
    }
}

/// A decoded image handed from the decoding task to the view (CGImage is immutable and safe to share).
struct LoadedImage: @unchecked Sendable {
    let cgImage: CGImage

    static func load(_ paths: [String], maxPixelSize: Int) async -> LoadedImage? {
        await Task.detached(priority: .userInitiated) {
            for path in paths {
                let options: [CFString: Any] = [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceCreateThumbnailWithTransform: true,
                    kCGImageSourceShouldCacheImmediately: true,
                    kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
                ]
                if let source = CGImageSourceCreateWithURL(URL(filePath: path) as CFURL, nil),
                   let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) {
                    return LoadedImage(cgImage: image)
                }
            }
            return nil
        }.value
    }
}
