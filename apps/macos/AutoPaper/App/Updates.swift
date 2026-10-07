import AppKit
import Observation
import Sparkle

/// Sparkle auto-update (AudioPaper's `Updates`). Sparkle asks on the second launch whether to check automatically
/// (about once a day), and Settings → About can change that answer; "Check for Updates…" checks now. Updates are
/// verified against the EdDSA public key in Info.plist (`SUPublicEDKey`) before anything is installed, and a
/// sandboxed install goes through Sparkle's installer launcher (`SUEnableInstallerLauncherService`).
@MainActor
final class Updates: NSObject {
    static let shared = Updates()

    private lazy var controller = SPUStandardUpdaterController(
        startingUpdater: true, updaterDelegate: nil, userDriverDelegate: self
    )

    /// Starts the updater (scheduled checks, if the person allowed them). Call once at launch.
    func start() {
        _ = controller
    }

    /// Check for Updates… (the app menu, the menu bar menu, Settings → About's Check Now): the person asked, so
    /// AutoPaper comes forward with Sparkle's window.
    func checkForUpdates() {
        NSApp.activate()
        controller.checkForUpdates(nil)
    }

    /// The update preferences shown in Settings → About, bound to Sparkle's own.
    private(set) lazy var settings = UpdateSettings(updater: controller.updater)
}

/// Sparkle's update preferences for SwiftUI. Reads and writes go straight to `SPUUpdater`, which stores them, so
/// the second-launch question and these switches never disagree.
@MainActor
@Observable
final class UpdateSettings {
    @ObservationIgnored private let updater: SPUUpdater
    @ObservationIgnored private var observations: [NSKeyValueObservation] = []

    var automaticallyChecks: Bool {
        didSet { if updater.automaticallyChecksForUpdates != automaticallyChecks { updater.automaticallyChecksForUpdates = automaticallyChecks } }
    }
    var automaticallyDownloads: Bool {
        didSet { if updater.automaticallyDownloadsUpdates != automaticallyDownloads { updater.automaticallyDownloadsUpdates = automaticallyDownloads } }
    }
    private(set) var lastChecked: Date?
    private(set) var canCheck: Bool

    init(updater: SPUUpdater) {
        self.updater = updater
        automaticallyChecks = updater.automaticallyChecksForUpdates
        automaticallyDownloads = updater.automaticallyDownloadsUpdates
        lastChecked = updater.lastUpdateCheckDate
        canCheck = updater.canCheckForUpdates
        // Sparkle changes these itself (the second-launch answer, a finished check, a check in progress), always on
        // the main thread.
        observations = [
            updater.observe(\.automaticallyChecksForUpdates) { [weak self] updater, _ in
                MainActor.assumeIsolated { self?.automaticallyChecks = updater.automaticallyChecksForUpdates }
            },
            updater.observe(\.automaticallyDownloadsUpdates) { [weak self] updater, _ in
                MainActor.assumeIsolated { self?.automaticallyDownloads = updater.automaticallyDownloadsUpdates }
            },
            updater.observe(\.lastUpdateCheckDate) { [weak self] updater, _ in
                MainActor.assumeIsolated { self?.lastChecked = updater.lastUpdateCheckDate }
            },
            updater.observe(\.canCheckForUpdates) { [weak self] updater, _ in
                MainActor.assumeIsolated { self?.canCheck = updater.canCheckForUpdates }
            },
        ]
    }
}

extension Updates: @preconcurrency SPUStandardUserDriverDelegate {
    /// AutoPaper lives in the menu bar, so a scheduled update shouldn't pop a window over whatever the person is
    /// doing: Sparkle shows it gently, and the app comes forward when it does.
    var supportsGentleScheduledUpdateReminders: Bool { true }

    func standardUserDriverWillHandleShowingUpdate(_ handleShowingUpdate: Bool, forUpdate update: SUAppcastItem, state: SPUUserUpdateState) {
        NSApp.activate()
    }
}
