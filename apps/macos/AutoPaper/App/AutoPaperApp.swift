import AutopaperCore
import SwiftUI

@main
struct AutoPaperApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    private let model = AppModel.shared

    var body: some Scene {
        // HIG: a menu bar extra shows a menu, not a popover. The rich view is the main window, one item away.
        MenuBarExtra(isInserted: showInMenuBar) {
            MenuBarMenu()
                .environment(model)
        } label: {
            // Template images (Assets.xcassets), tinted by the system for light, dark and selected menu bars.
            let paused = model.settings?.paused ?? false
            Image(paused ? "MenuBarIconPaused" : "MenuBarIcon")
                .accessibilityLabel(paused ? "AutoPaper, paused" : "AutoPaper")
                .background(WindowOpenerCapture(model: model))
        }
        .menuBarExtraStyle(.menu)

        Window("AutoPaper", id: WindowID.main) {
            MainWindow()
                .environment(model)
                .background(WindowOpenerCapture(model: model))
                .onAppear { appDelegate.windowDidOpen(WindowID.main) }
                .onDisappear { appDelegate.windowDidClose(WindowID.main) }
        }
        .defaultSize(width: 1040, height: 780)
        // Reopens if it was open at quit; with the menu bar extra hidden it's the way in, so it always opens.
        .defaultLaunchBehavior(
            UserDefaults.standard.bool(forKey: AppModel.Keys.mainWindowOpen) || !model.showInMenuBar ? .presented : .suppressed
        )
        // Open state is remembered explicitly (above); the system's restoration would only double it.
        .restorationBehavior(.disabled)
        .commands {
            AppCommands(model: model)
        }

        Window("Welcome to AutoPaper", id: WindowID.welcome) {
            WelcomeView()
                .environment(model)
                .onAppear { appDelegate.windowDidOpen(WindowID.welcome) }
                .onDisappear { appDelegate.windowDidClose(WindowID.welcome) }
        }
        .windowResizability(.contentSize)
        .defaultLaunchBehavior(model.welcomeDone ? .suppressed : .presented)
        .restorationBehavior(.disabled)

        // The Dock icon follows Show in Menu Bar through `AppModel.menuBarVisibilityChanged` (set by the app
        // delegate), not an `onChange` here: this view only exists while Settings is open, and the person can also
        // remove the menu bar item by ⌘-dragging it out with Settings closed.
        SwiftUI.Settings {
            SettingsView()
                .environment(model)
        }
    }

    private var showInMenuBar: Binding<Bool> {
        Binding(
            get: { model.showInMenuBar },
            set: { model.showInMenuBar = $0 }
        )
    }
}

/// Settings' panes, by the value `SettingsView` remembers the last one under.
enum SettingsPaneID: String {
    case general, providers, accounts, memory, budget, about

    /// Where Settings remembers the pane viewed last (HIG: reopen on it).
    static let defaultsKey = "settingsPane"
}

/// Opens Settings in front. AutoPaper usually has no Dock icon, so opening the window alone leaves it behind
/// whatever app is frontmost; activate first, then bring the window forward once SwiftUI has made it.
@MainActor
enum SettingsWindow {
    static func show(_ openSettings: OpenSettingsAction) {
        // The request comes from AutoPaper's own menu, so taking focus is what the person asked for.
        NSApp.activate()
        openSettings()
        DispatchQueue.main.async {
            // Best effort: SwiftUI names its Settings window with this identifier (not documented, so it may change;
            // if it does, the window still opens, only perhaps behind the frontmost app's).
            NSApp.windows
                .first { $0.identifier?.rawValue == "com_apple_SwiftUI_Settings_window" && $0.canBecomeKey }?
                .makeKeyAndOrderFront(nil)
        }
    }
}

extension AppModel {
    /// Opens Settings on `pane`, focusing a key's field in Accounts when a key link asked for it. The links that
    /// go straight to a fix (spec 6a) all come through here: the Now view, Providers, a notification.
    func openSettings(_ pane: SettingsPaneID, focusing account: String? = nil) {
        if let account { accountToFocus = account }
        UserDefaults.standard.set(pane.rawValue, forKey: SettingsPaneID.defaultsKey)
        guard let openSettingsAction else { return }
        SettingsWindow.show(openSettingsAction)
    }

    /// Follows a problem's link to where it's fixed (spec 6a).
    func open(_ place: SettingsProblem.Place) {
        switch place {
        case .accounts(let account):
            openSettings(.accounts, focusing: account)
        case .providers(let job, let address):
            if address { providersFocus = job ?? .concepts }
            openSettings(.providers)
        case .budget:
            openSettings(.budget)
        case .appleIntelligence:
            NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.Siri-Settings.extension")!)
        case .systemWallpaper:
            NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.Wallpaper-Settings.extension")!)
        case .mood(let id, let keyword):
            showMood(id, selecting: keyword)
        }
    }
}

/// Hands SwiftUI's window actions to the AppKit side (Dock clicks, reopen, notifications, closing the welcome) the
/// first time any view appears. The menu bar extra's label appears at launch, so it's there before any window is.
struct WindowOpenerCapture: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        Color.clear
            .frame(width: 0, height: 0)
            .onAppear {
                model.openWindowAction = openWindow
                model.dismissWindowAction = dismissWindow
                model.openSettingsAction = openSettings
            }
    }
}
