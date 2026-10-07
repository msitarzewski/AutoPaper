import AutopaperCore
import Foundation
import os

let log = Logger(subsystem: "com.autopaper", category: "app")

/// The engine's settings record (`AutopaperCore.Settings`), named apart from SwiftUI's `Settings` scene.
typealias EngineSettings = AutopaperCore.Settings

/// The one `Engine`, and a way to call it off the main thread.
///
/// The engine's sync methods block the calling thread (SQLite, files; `render_for_display`, `prune` and
/// `clear_history` for hundreds of milliseconds or more), so the app never calls them on the main thread: `call`
/// runs them on a concurrent queue (the engine is thread-safe). Its async methods (`generate`, `run_if_due`,
/// `make_echo`, `list_models`, `test_provider`) are awaited directly; they run on the core's tokio runtime.
final class CoreBridge: Sendable {
    let engine: Engine
    private let queue = DispatchQueue(label: "com.autopaper.engine", qos: .userInitiated, attributes: .concurrent)
    /// Changes the person makes (settings, keywords, ratings) run one at a time in the order they were made, so
    /// a quick second change can never be saved before the first.
    private let writes = DispatchQueue(label: "com.autopaper.engine.writes", qos: .userInitiated)

    private init(engine: Engine) {
        self.engine = engine
    }

    /// Opens the engine on a background queue (it loads the 133 MB embedding model and migrates the database), and
    /// registers `detail` for every generation's progress in detail (stage, fraction, time left), whichever call
    /// started it.
    static func open(secrets: KeychainSecretStore, detail: ProgressDetailObserver) async throws -> CoreBridge {
        let config = try Self.configuration()
        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                continuation.resume(with: Result {
                    let engine = try Engine.open(config: config, secrets: secrets)
                    engine.setProgressDetailObserver(observer: detail)
                    return CoreBridge(engine: engine)
                })
            }
        }
    }

    /// Runs a sync engine call off the main thread.
    func call<T: Sendable>(_ work: @escaping @Sendable (Engine) throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [engine] in
                continuation.resume(with: Result { try work(engine) })
            }
        }
    }

    /// Runs a sync engine call that changes something, after every change asked for before it.
    ///
    /// `nonisolated(nonsending)`: it starts on the caller's actor (the main actor) and puts the work on the serial
    /// queue before it suspends, so writes reach the queue in the order they were called. A plain nonisolated async
    /// method would first hop to the global executor, where two quick writes (each a whole settings record) could
    /// swap places and the older one be saved last.
    nonisolated(nonsending) func write<T: Sendable>(_ work: @escaping @Sendable (Engine) throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            writes.async { [engine] in
                continuation.resume(with: Result { try work(engine) })
            }
        }
    }

    /// The engine's data folder: the sandbox container's Application Support/AutoPaper.
    static func dataDirectory() throws -> URL {
        let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        return support.appending(path: "AutoPaper", directoryHint: .isDirectory)
    }

    /// Where the database and images live (the sandbox container's Application Support/AutoPaper), where the
    /// bundled embedding model is, the person's language, and the User-Agent's client part.
    static func configuration() throws -> EngineConfig {
        let data = try dataDirectory()
        let model = Bundle.main.resourceURL?.appending(path: "Models/bge-small-en-v1.5", directoryHint: .isDirectory)
        let os = ProcessInfo.processInfo.operatingSystemVersion
        return EngineConfig(
            dataDir: data.path(percentEncoded: false),
            modelDir: model?.path(percentEncoded: false) ?? "",
            locale: Locale.current.identifier(.bcp47),
            client: "macOS/\(os.majorVersion).\(os.minorVersion) AutoPaper/\(AppInfo.version)"
        )
    }
}

enum AppInfo {
    static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }

    static var build: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "0"
    }
}

/// Hands progress stages from the engine's thread to the main actor.
final class ProgressRelay: ProgressObserver {
    private let handler: @Sendable (ProgressStage) -> Void

    init(_ handler: @escaping @Sendable (ProgressStage) -> Void) {
        self.handler = handler
    }

    func onProgress(stage: ProgressStage) {
        handler(stage)
    }
}

/// Hands progress in detail (`ProgressDetail`: the stage, and while painting how far along and about how long is
/// left) from the engine's thread to the main actor. Registered once, when the engine opens.
final class ProgressDetailRelay: ProgressDetailObserver {
    private let handler: @Sendable (ProgressDetail) -> Void

    init(_ handler: @escaping @Sendable (ProgressDetail) -> Void) {
        self.handler = handler
    }

    func onProgressDetail(detail: ProgressDetail) {
        handler(detail)
    }
}
