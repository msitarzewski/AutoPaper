import AppKit
import AutopaperCore
import CoreGraphics
import ImageIO

/// Puts a wallpaper on every display, in one of two ways (Settings → General → Show wallpapers, app-spec 3a):
///
/// - **Over my wallpaper** (the default; AudioPaper's `DesktopOverlay`, same author, MIT): one borderless,
///   click-through window per display, just above the desktop picture and below the icons, on every Space
///   (`OverlayWindow`). The person's own wallpaper is never touched — macOS's moving Aerials and dynamic pictures
///   keep running underneath — so pausing, quitting or Restore My Wallpaper uncovers it exactly.
/// - **As my wallpaper**: `NSWorkspace.setDesktopImageURL` sets the real desktop picture, so it also shows in Mission
///   Control and on the lock screen. macOS only changes the Space each display is showing, so the file is set again
///   whenever the active Space changes. Before a display's own picture is first covered it's recorded (per display),
///   so Restore My Wallpaper and switching to Over my wallpaper can put it back.
///
/// Either way the engine renders the image at each display's native pixel size first (`render_for_display`,
/// centre-cropped; cached as `renders/<id>-<w>x<h>.jpg`, so every new wallpaper is a new file — macOS ignores a URL it
/// already shows). macOS gives apps no way to set the lock screen picture (it shows the desktop picture there), so the
/// engine's `settings.set_lock_screen` is intentionally not read on macOS; Settings → General says why.
///
/// As my wallpaper: a picture the person chose themselves is theirs, and only a new wallpaper replaces it. Putting the
/// current one back (at launch, on a Space change, for a new display) happens only where a display shows one of
/// AutoPaper's own renders, no picture, or a file that's gone.
@MainActor
final class DesktopService {
    /// How wallpapers are shown. Changing it doesn't move anything by itself: `AppModel` releases the old way.
    var mode: WallpaperMode
    /// As my wallpaper: the file each display was given, by display number, re-applied on Space changes.
    private var applied: [CGDirectDisplayID: URL] = [:]
    /// Over my wallpaper: one window per display, by the display's UUID.
    private var overlays: [String: OverlayWindow] = [:]
    /// The person's own pictures were put back (Restore My Wallpaper, or a switch to Over my wallpaper): a Space still
    /// showing an AutoPaper render gets its picture back when it's next visited (macOS only changes the Space on
    /// screen).
    private var releasingOtherSpaces = false
    private var spaceObserver: (any NSObjectProtocol)?
    private let defaults = UserDefaults.standard
    /// The engine's renders folder (`<data dir>/renders`): a desktop picture in it is AutoPaper's.
    var rendersDirectory: URL?
    /// Called after a Space change, once any re-applying is done.
    var onChange: (@MainActor () -> Void)?

    /// Each display's own picture (by display UUID: its path, or "" when it can't be set again), and its scaling
    /// options, recorded before As my wallpaper first covers it.
    private static let ownPicturesKey = "ownDesktopPictures"
    private static let ownOptionsKey = "ownDesktopPictureOptions"

    init(mode: WallpaperMode = .overlay) {
        self.mode = mode
        spaceObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.activeSpaceDidChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.spaceChanged()
                self?.onChange?()
            }
        }
    }

    /// One render target per screen: the display's stable UUID and its pixel size.
    static func targets() -> [(screen: NSScreen, target: DisplayTarget)] {
        NSScreen.screens.compactMap { screen in
            guard let number = screen.displayNumber else { return nil }
            let (width, height) = pixelSize(of: screen, number: number)
            return (screen, DisplayTarget(id: displayID(number), width: width, height: height))
        }
    }

    /// The largest display's pixel size, for `set_display_hint` (the engine picks the image size from it).
    static func largestDisplay() -> (width: UInt32, height: UInt32)? {
        targets().map(\.target).max { UInt64($0.width) * UInt64($0.height) < UInt64($1.width) * UInt64($1.height) }
            .map { ($0.width, $0.height) }
    }

    /// Renders `generation` for every display (off the main thread) and shows it the current way. As my wallpaper
    /// with `onlyReplacingAutoPaper`: only on displays whose picture AutoPaper may replace (`mayReplace`).
    /// Throws when a render fails (a pruned image is `NotFound`) or no display could show it.
    func show(_ generation: Generation, using core: CoreBridge, onlyReplacingAutoPaper: Bool = false) async throws {
        switch mode {
        case .overlay:
            try await cover(generation, using: core)
        case .replace:
            try await setDesktop(generation, using: core, onlyReplacingAutoPaper: onlyReplacingAutoPaper)
        }
    }

    // MARK: Over my wallpaper

    /// AutoPaper's overlay is on the desktop.
    var isCovering: Bool {
        overlays.values.contains { $0.isCovering }
    }

    /// Shows `generation` in each display's overlay window, cross-fading from what it showed (or fading in). Windows
    /// of displays that are gone are closed.
    private func cover(_ generation: Generation, using core: CoreBridge) async throws {
        let targets = Self.targets()
        guard !targets.isEmpty else { return }
        let files = try await render(generation, for: targets.map(\.target), using: core)
        // Decoded off the main thread: a 5K JPEG takes a moment.
        var images: [String: DecodedImage] = [:]
        for (id, file) in files {
            if let image = await Task.detached(priority: .userInitiated, operation: { DecodedImage(file) }).value {
                images[id] = image
            }
        }
        guard mode == .overlay else { return }
        let fade = OverlayRules.fadeDuration(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
        var shown: Set<String> = []
        for (screen, target) in targets {
            guard let image = images[target.id] else { continue }
            let window = overlays[target.id] ?? OverlayWindow(screen: screen)
            overlays[target.id] = window
            window.place(on: screen)
            window.show(image.image, fade: fade)
            shown.insert(target.id)
        }
        // Displays that were unplugged.
        for (id, window) in overlays where !shown.contains(id) {
            window.close()
            overlays[id] = nil
        }
        if shown.isEmpty { throw AutoPaperError.Internal(detail: "no display could show the overlay") }
    }

    /// Fades the overlay out (instantly when `animated` is false), uncovering the person's own wallpaper.
    func uncover(animated: Bool = true) {
        let fade = animated ? OverlayRules.fadeDuration(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion) : 0
        for window in overlays.values { window.hide(fade: fade) }
    }

    /// Quitting: the windows go at once, so the person's wallpaper is what's left.
    func closeOverlays() {
        for window in overlays.values { window.close() }
        overlays = [:]
    }

    // MARK: As my wallpaper

    private func setDesktop(_ generation: Generation, using core: CoreBridge, onlyReplacingAutoPaper: Bool) async throws {
        let targets = Self.targets().filter { !onlyReplacingAutoPaper || mayReplace($0.screen) }
        guard !targets.isEmpty else { return }
        let files = try await render(generation, for: targets.map(\.target), using: core)
        guard mode == .replace else { return }
        var setAny = false
        var failure: (any Error)?
        releasingOtherSpaces = false
        for screen in NSScreen.screens {
            guard let number = screen.displayNumber, let file = files[Self.displayID(number)] else { continue }
            recordOwnPicture(on: screen)
            do {
                try Self.setDesktop(file, on: screen)
                applied[number] = file
                setAny = true
            } catch {
                failure = error
                log.error("Setting the wallpaper failed on display \(number): \(error.localizedDescription, privacy: .public)")
            }
        }
        if !setAny, let failure { throw failure }
    }

    private func spaceChanged() {
        switch mode {
        case .overlay:
            // The overlay is on every Space already; only a Space still showing a render needs its picture back.
            if releasingOtherSpaces { putBackOwnPictures(on: NSScreen.screens.filter(showsRender)) }
        case .replace:
            if applied.isEmpty, releasingOtherSpaces {
                putBackOwnPictures(on: NSScreen.screens.filter(showsRender))
            } else {
                reapply()
            }
        }
    }

    /// Sets the file again where a display shows an older AutoPaper render (or nothing): after a Space change,
    /// macOS shows that Space's own wallpaper until it's set there too. A Space showing the person's own picture
    /// keeps it.
    private func reapply() {
        for screen in NSScreen.screens {
            guard let number = screen.displayNumber, let file = applied[number],
                  NSWorkspace.shared.desktopImageURL(for: screen)?.standardizedFileURL != file.standardizedFileURL,
                  mayReplace(screen)
            else { continue }
            do {
                try Self.setDesktop(file, on: screen)
            } catch {
                log.error("Re-applying the wallpaper failed on display \(number): \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    /// Stops re-applying what was set: after Clear History AutoPaper has no current wallpaper, and the desktop keeps
    /// its picture (the engine keeps that render) until the next new one.
    func forget() {
        applied = [:]
    }

    /// Displays that AutoPaper may put the current wallpaper back on and that don't already show what it set
    /// (a new display, or an older AutoPaper render on a display since launch).
    func displaysNeedingWallpaper() -> Bool {
        NSScreen.screens.contains { screen in
            guard mayReplace(screen) else { return false }
            guard let number = screen.displayNumber, let file = applied[number] else { return true }
            return NSWorkspace.shared.desktopImageURL(for: screen)?.standardizedFileURL != file.standardizedFileURL
        }
    }

    /// Whether a display's picture is AutoPaper's to replace without a new wallpaper: none, a file that no longer
    /// exists, or one of AutoPaper's own renders. Anything else is a picture the person chose.
    func mayReplace(_ screen: NSScreen) -> Bool {
        let picture = NSWorkspace.shared.desktopImageURL(for: screen)
        let exists = picture.map { $0.isFileURL && FileManager.default.fileExists(atPath: $0.path(percentEncoded: false)) } ?? false
        return DesktopRules.mayReplace(picture: picture, fileExists: exists, rendersFolder: rendersDirectory)
    }

    /// A display's desktop picture is one of AutoPaper's renders.
    private func showsRender(_ screen: NSScreen) -> Bool {
        NSWorkspace.shared.desktopImageURL(for: screen).map { DesktopRules.isRender($0, in: rendersDirectory) } ?? false
    }

    /// A display shows a picture the person chose themselves.
    func showsOwnPicture() -> Bool {
        NSScreen.screens.contains { !mayReplace($0) }
    }

    /// A display's desktop picture is one of AutoPaper's renders (As my wallpaper: what Restore My Wallpaper undoes).
    func showsAutoPaperPicture() -> Bool {
        NSScreen.screens.contains(where: showsRender)
    }

    // MARK: The person's own pictures

    /// Puts the person's own pictures back where a display shows one of AutoPaper's renders, and stops putting
    /// AutoPaper's back on Space changes: Restore My Wallpaper (As my wallpaper), or a switch to Over my wallpaper
    /// (the overlay now covers the desktop, or the person's picture shows). Other Spaces follow when visited. False
    /// when a display's picture couldn't be put back (none recorded, or one that can't be set again, like an Aerial).
    @discardableResult
    func releaseDesktop() -> Bool {
        applied = [:]
        releasingOtherSpaces = true
        return putBackOwnPictures(on: NSScreen.screens.filter(showsRender))
    }

    @discardableResult
    private func putBackOwnPictures(on screens: [NSScreen]) -> Bool {
        let saved = defaults.dictionary(forKey: Self.ownPicturesKey) as? [String: String] ?? [:]
        let options = defaults.dictionary(forKey: Self.ownOptionsKey) as? [String: [String: Any]] ?? [:]
        var all = true
        for screen in screens {
            guard let number = screen.displayNumber else { continue }
            let id = Self.displayID(number)
            guard let path = saved[id], !path.isEmpty else {
                // Nothing recorded (AutoPaper set it before it kept records), or an Aerial: left as it is, and said.
                log.info("No picture of the person's own to put back on display \(id, privacy: .public)")
                all = false
                continue
            }
            let url = URL(filePath: path)
            // macOS's own pictures (.madesktop) keep their own framing; others get the options they had.
            let chosen = url.pathExtension == "madesktop" ? [:] : Self.desktopOptions(from: options[id] ?? [:])
            do {
                try NSWorkspace.shared.setDesktopImageURL(url, for: screen, options: chosen)
            } catch {
                log.error("Putting back the person's picture failed on display \(id, privacy: .public): \(error.localizedDescription, privacy: .public)")
                all = false
            }
        }
        return all
    }

    /// Records a display's own picture before As my wallpaper covers it, so it can be put back. AutoPaper's renders
    /// are never recorded; a newer picture of the person's replaces the record.
    private func recordOwnPicture(on screen: NSScreen) {
        guard let number = screen.displayNumber, let url = NSWorkspace.shared.desktopImageURL(for: screen), url.isFileURL,
              !DesktopRules.isRender(url, in: rendersDirectory)
        else { return }
        let id = Self.displayID(number)
        var saved = defaults.dictionary(forKey: Self.ownPicturesKey) as? [String: String] ?? [:]
        var options = defaults.dictionary(forKey: Self.ownOptionsKey) as? [String: [String: Any]] ?? [:]
        let restorable = DesktopRules.restorablePicture(for: url)
        saved[id] = restorable?.path(percentEncoded: false) ?? ""
        options[id] = Self.storableOptions(NSWorkspace.shared.desktopImageOptions(for: screen) ?? [:])
        defaults.set(saved, forKey: Self.ownPicturesKey)
        defaults.set(options, forKey: Self.ownOptionsKey)
    }

    /// The scaling options as plain values preferences can store (the fill colour, an NSColor, is left out).
    private static func storableOptions(_ options: [NSWorkspace.DesktopImageOptionKey: Any]) -> [String: Any] {
        var stored: [String: Any] = [:]
        if let scaling = options[.imageScaling] as? NSNumber { stored["imageScaling"] = scaling }
        if let clipping = options[.allowClipping] as? NSNumber { stored["allowClipping"] = clipping }
        return stored
    }

    private static func desktopOptions(from stored: [String: Any]) -> [NSWorkspace.DesktopImageOptionKey: Any] {
        var options: [NSWorkspace.DesktopImageOptionKey: Any] = [:]
        if let scaling = stored["imageScaling"] as? NSNumber { options[.imageScaling] = scaling }
        if let clipping = stored["allowClipping"] as? NSNumber { options[.allowClipping] = clipping }
        return options
    }

    // MARK: Rendering

    /// One render per display (by display UUID), made by the engine off the main thread.
    private func render(_ generation: Generation, for targets: [DisplayTarget], using core: CoreBridge) async throws -> [String: URL] {
        let id = generation.id
        var files: [String: URL] = [:]
        try await withThrowingTaskGroup(of: (String, URL).self) { group in
            for target in targets {
                group.addTask {
                    let path = try await core.call { try $0.renderForDisplay(id: id, display: target) }
                    return (target.id, URL(filePath: path))
                }
            }
            for try await (display, url) in group { files[display] = url }
        }
        return files
    }

    /// The render fills the display exactly; these options keep macOS from adding borders if it ever doesn't.
    private static func setDesktop(_ file: URL, on screen: NSScreen) throws {
        try NSWorkspace.shared.setDesktopImageURL(file, for: screen, options: [
            .imageScaling: NSImageScaling.scaleProportionallyUpOrDown.rawValue,
            .allowClipping: true,
        ])
    }

    /// The pixels macOS draws the desktop at: the current display mode's pixel size (the backing size of a
    /// scaled Retina mode), else the frame times the backing scale.
    private static func pixelSize(of screen: NSScreen, number: CGDirectDisplayID) -> (UInt32, UInt32) {
        if let mode = CGDisplayCopyDisplayMode(number), mode.pixelWidth > 0, mode.pixelHeight > 0 {
            return (UInt32(clamping: mode.pixelWidth), UInt32(clamping: mode.pixelHeight))
        }
        let scale = screen.backingScaleFactor
        return (UInt32(clamping: Int((screen.frame.width * scale).rounded())), UInt32(clamping: Int((screen.frame.height * scale).rounded())))
    }

    /// A display's stable identifier (it survives reconnects, unlike the display number), else its number.
    private static func displayID(_ number: CGDirectDisplayID) -> String {
        guard let uuid = CGDisplayCreateUUIDFromDisplayID(number)?.takeRetainedValue() else { return String(number) }
        return CFUUIDCreateString(nil, uuid) as String
    }
}

/// A decoded render. A `CGImage` is immutable, so handing one from the decoding task to the main actor is safe.
private struct DecodedImage: @unchecked Sendable {
    let image: CGImage

    init?(_ file: URL) {
        guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
        else { return nil }
        self.image = image
    }
}

/// A borderless, click-through window just above the desktop picture and below the desktop's icons, on every Space
/// (AudioPaper's `FadeWindow`). It ignores the mouse, so the desktop's own clicks and menus still reach Finder; it
/// isn't in the Window menu, ⌘` or the accessibility tree (it's the desktop's picture, not a window to work in).
@MainActor
final class OverlayWindow: NSWindow {
    /// Bumped by every show and hide, so a fade-out that finishes after a newer show doesn't take the window away.
    private var change = 0
    private(set) var isCovering = false

    init(screen: NSScreen) {
        super.init(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
        level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)) + 1)
        collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenNone]
        ignoresMouseEvents = true
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        isReleasedWhenClosed = false
        isExcludedFromWindowsMenu = true
        animationBehavior = .none
        alphaValue = 0
        setAccessibilityElement(false)
        let view = NSView(frame: NSRect(origin: .zero, size: screen.frame.size))
        view.wantsLayer = true
        view.layer?.contentsGravity = .resizeAspectFill
        view.layer?.backgroundColor = NSColor.black.cgColor
        view.setAccessibilityElement(false)
        contentView = view
        setFrame(screen.frame, display: false)
    }

    func place(on screen: NSScreen) {
        if frame != screen.frame { setFrame(screen.frame, display: false) }
    }

    /// Shows `image`: fading the window in when it isn't showing, else cross-fading from the current picture.
    func show(_ image: CGImage, fade: TimeInterval) {
        guard let layer = contentView?.layer else { return }
        change += 1
        isCovering = true
        if !isVisible || alphaValue < 1 {
            if !isVisible || alphaValue == 0 { layer.contents = image } else { crossFade(layer, to: image, fade: fade) }
            orderFrontRegardless()
            NSAnimationContext.runAnimationGroup { context in
                context.duration = fade
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                animator().alphaValue = 1
            }
            return
        }
        crossFade(layer, to: image, fade: fade)
    }

    private func crossFade(_ layer: CALayer, to image: CGImage, fade: TimeInterval) {
        let transition = CATransition()
        transition.type = .fade
        transition.duration = fade
        transition.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        layer.add(transition, forKey: "contents")
        layer.contents = image
    }

    /// Fades out, then leaves the screen (kept for the next show).
    func hide(fade: TimeInterval) {
        change += 1
        isCovering = false
        guard isVisible else { return }
        let hiding = change
        NSAnimationContext.runAnimationGroup { context in
            context.duration = fade
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            animator().alphaValue = 0
        } completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.change == hiding else { return }
                self.orderOut(nil)
                self.contentView?.layer?.contents = nil
            }
        }
    }

    // Never key or main: it's part of the desktop.
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

extension NSScreen {
    var displayNumber: CGDirectDisplayID? {
        (deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value
    }
}
