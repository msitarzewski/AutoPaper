import AutopaperCore
import SwiftUI

/// Settings: General · Providers · Accounts · Memory · Budget · About. Short panes size the window to their
/// content; longer panes scroll within a comfortable height, even on a large display. Settings reopens on the
/// last pane viewed (HIG: "Restore the most recently viewed pane").
struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("settingsPane") private var pane = "general"

    var body: some View {
        TabView(selection: $pane) {
            Tab("General", systemImage: "gearshape", value: "general") {
                GeneralSettings()
            }
            Tab("Providers", systemImage: "paintpalette", value: "providers") {
                ProvidersSettings()
            }
            Tab("Accounts", systemImage: "key", value: "accounts") {
                AccountsSettings()
            }
            Tab("Memory", systemImage: "brain", value: "memory") {
                MemorySettings()
            }
            Tab("Budget", systemImage: "dollarsign.circle", value: "budget") {
                BudgetSettings()
            }
            Tab("About", systemImage: "info.circle", value: "about") {
                AboutSettings()
            }
        }
        .frame(width: 520)
        .alert("Couldn't Save That", isPresented: Binding(
            get: { model.settingsError != nil },
            set: { if !$0 { model.settingsError = nil } }
        )) {
            Button("OK") { model.settingsError = nil }
        } message: {
            Text(model.settingsError ?? "")
        }
    }
}

/// A grouped form sized to its content, capped at a comfortable height or the space available on a small screen.
struct SettingsPane<Content: View>: View {
    @Environment(AppModel.self) private var model
    @ViewBuilder let content: (EngineSettings) -> Content
    /// The form's full height, measured from its scroll content.
    @State private var contentHeight: CGFloat = 0

    var body: some View {
        if let settings = model.settings {
            let limit = Self.maxHeight
            let overflowing = contentHeight > limit
            Form {
                content(settings)
            }
            .formStyle(.grouped)
            .onScrollGeometryChange(for: CGFloat.self) { geometry in
                geometry.contentSize.height + geometry.contentInsets.top + geometry.contentInsets.bottom
            } action: { _, height in
                contentHeight = height
            }
            .scrollDisabled(!overflowing)
            .fixedSize(horizontal: false, vertical: !overflowing)
            .frame(height: overflowing ? limit : nil)
        } else {
            ProgressView("Opening your library…")
                .frame(maxWidth: .infinity, minHeight: 200)
        }
    }
}

extension SettingsPane {
    /// Keep long panes compact, with room for the window's title bar and toolbar on smaller screens.
    static var maxHeight: CGFloat {
        #if DEBUG
        // For testing the scrolling on a large display: `-SettingsMaxPaneHeight 500` at launch (Debug builds only).
        let override = UserDefaults.standard.double(forKey: "SettingsMaxPaneHeight")
        if override > 0 { return override }
        #endif
        return min(600, max(320, (NSScreen.main?.visibleFrame.height ?? 800) - 100))
    }
}

extension AppModel {
    /// A binding to one setting: reads the engine's settings, saves through `updateSettings`.
    func setting<Value>(_ keyPath: WritableKeyPath<EngineSettings, Value>, _ fallback: Value) -> Binding<Value> {
        Binding(
            get: { self.settings?[keyPath: keyPath] ?? fallback },
            set: { value in self.updateSettings { $0[keyPath: keyPath] = value } }
        )
    }
}
