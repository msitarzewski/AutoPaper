import AppKit
import AutopaperCore

/// Dock presence and the Dock menu. AutoPaper is a menu bar utility: it shows in the Dock only while one of its
/// windows (the main window, the welcome) is open, or when its menu bar extra is hidden, so there's always a way
/// in (AudioPaper's `updateDockPresence`).
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var openWindows: Set<String> = []
    /// Set when quitting, so windows closing on the way out don't count as the person closing them.
    private var isTerminating = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Hiding the menu bar extra (the Settings toggle, or ⌘-dragging it out of the menu bar while Settings is
        // closed) brings the Dock icon at once, and showing it again takes the Dock icon away.
        AppModel.shared.menuBarVisibilityChanged = { [weak self] in self?.updateDockPresence() }
        updateDockPresence()
        AppModel.shared.start()
        Updates.shared.start()
        // Tell Siri and Shortcuts about AutoPaper's phrases (Intents/Intents.swift); without this a fresh or
        // locally built install may not be matched until the system gets round to it.
        AutoPaperShortcuts.updateAppShortcutParameters()
    }

    func windowDidOpen(_ id: String) {
        openWindows.insert(id)
        if id == WindowID.main { UserDefaults.standard.set(true, forKey: AppModel.Keys.mainWindowOpen) }
        updateDockPresence()
        // Only the welcome comes forward by itself: it opens on the first launch, which the person started. The main
        // window reopening at launch (perhaps at login) doesn't take focus from what the person is doing; opening it
        // from a menu, the Dock or a notification activates AutoPaper there (`AppModel.showMainWindow`).
        if id == WindowID.welcome { Self.bringForwardAtFirstLaunch() }
    }

    /// The welcome opens on the first launch, which the person started, so it should be in front. Launch Services
    /// doesn't activate an agent (LSUIElement) app it opens, and with no event of AutoPaper's own the cooperative
    /// `activate()` is declined, leaving the welcome behind whatever was in front (checked on macOS 27 with Stage
    /// Manager). So the frontmost app is named as the source of the activation.
    private static func bringForwardAtFirstLaunch() {
        NSApp.activate()
        if let front = NSWorkspace.shared.frontmostApplication, front != NSRunningApplication.current {
            NSRunningApplication.current.activate(from: front, options: [])
        }
    }

    func windowDidClose(_ id: String) {
        openWindows.remove(id)
        if id == WindowID.main, !isTerminating {
            UserDefaults.standard.set(false, forKey: AppModel.Keys.mainWindowOpen)
        }
        updateDockPresence()
    }

    func updateDockPresence() {
        let needsDock = DockPolicy.needsDock(openWindows: openWindows.count, showInMenuBar: AppModel.shared.showInMenuBar)
        let policy: NSApplication.ActivationPolicy = needsDock ? .regular : .accessory
        if NSApp.activationPolicy() != policy {
            NSApp.setActivationPolicy(policy)
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        isTerminating = true
        // A wallpaper being made stops at its next stage; nothing half-made is shown.
        AppModel.shared.cancel()
        // Over my wallpaper: quitting uncovers the person's own wallpaper (app-spec 3a).
        AppModel.shared.desktop.closeOverlays()
        return .terminateNow
    }

    /// A menu bar utility keeps running with no windows open; closing the main window must not quit it.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    /// Clicking the Dock icon (or opening AutoPaper again from Finder) opens the main window. The desktop overlay's
    /// windows are always visible, so what counts is a window the person works in (one that can be main).
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !NSApp.windows.contains(where: { ($0.isVisible || $0.isMiniaturized) && $0.canBecomeMain }) {
            AppModel.shared.showMainWindow()
        } else {
            NSApp.activate()
            AppModel.raiseWorkWindows()
        }
        return true
    }

    func applicationDockMenu(_ sender: NSApplication) -> NSMenu? {
        let model = AppModel.shared
        let menu = NSMenu()
        menu.autoenablesItems = false
        menu.addItem(ClosureMenuItem("New Wallpaper Now", enabled: model.phase == .ready && !model.isWorking) {
            model.newWallpaperNow()
        })
        if let current = model.current {
            menu.addItem(ClosureMenuItem("Like", checked: current.rating == .liked) { model.rate(current, .liked) })
            menu.addItem(ClosureMenuItem("Dislike", checked: current.rating == .disliked) { model.rate(current, .disliked) })
        }
        if !model.moods.isEmpty {
            // Mood ▸ the moods (the current one checkmarked), as in the menu bar menu.
            let moods = NSMenu()
            moods.autoenablesItems = false
            for mood in model.moods {
                let item = ClosureMenuItem(mood.name, checked: mood.active) { model.useMood(mood.id) }
                item.setAccessibilityLabel(MoodText.spokenName(mood))
                moods.addItem(item)
            }
            moods.addItem(.separator())
            moods.addItem(ClosureMenuItem("Edit Moods…") { model.editMoods() })
            let item = NSMenuItem(title: "Mood", action: nil, keyEquivalent: "")
            item.submenu = moods
            menu.addItem(item)
        }
        let paused = model.settings?.paused ?? false
        menu.addItem(ClosureMenuItem(paused ? "Resume New Wallpapers" : "Pause New Wallpapers", enabled: model.settings != nil) {
            model.setPaused(!paused)
        })
        menu.addItem(ClosureMenuItem("Restore My Wallpaper", enabled: model.autoPaperShowing) { model.restoreMyWallpaper() })
        return menu
    }
}

/// An NSMenuItem that runs a closure, for the Dock menu.
@MainActor
final class ClosureMenuItem: NSMenuItem {
    private let handler: @MainActor () -> Void

    init(_ title: String, enabled: Bool = true, checked: Bool = false, handler: @escaping @MainActor () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(run), keyEquivalent: "")
        target = self
        isEnabled = enabled
        state = checked ? .on : .off
    }

    required init(coder: NSCoder) { fatalError("init(coder:) is not used") }

    @objc private func run() {
        handler()
    }
}
