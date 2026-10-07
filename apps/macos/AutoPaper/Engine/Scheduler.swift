import AppKit

/// When to ask the engine for a scheduled wallpaper.
///
/// A coalesced one-shot `Timer` at the engine's `next_due()` (with tolerance, so macOS can group it with other
/// work), re-armed after every generation and settings change. Timers don't fire while the Mac sleeps, so wake,
/// screen unlock, returning to this login session and clock changes also ask: `run_if_due` makes a wallpaper only
/// when one is due. After a wake it waits a little, so the network is back before a provider is called (an
/// `Offline` failure would otherwise bring back a liked wallpaper instead of a new one).
@MainActor
final class Scheduler {
    private var timer: Timer?
    private var observers: [(NotificationCenter, any NSObjectProtocol)] = []
    private var pendingCheck: Task<Void, Never>?
    private let onDue: @MainActor () -> Void

    /// Delay after waking before checking; the network usually needs a few seconds.
    static let wakeDelay: Duration = .seconds(20)

    init(onDue: @escaping @MainActor () -> Void) {
        self.onDue = onDue
    }

    /// Starts listening for wake, unlock, session and clock changes.
    func start() {
        guard observers.isEmpty else { return }
        let workspace = NSWorkspace.shared.notificationCenter
        observe(workspace, NSWorkspace.didWakeNotification, after: Self.wakeDelay)
        observe(workspace, NSWorkspace.screensDidWakeNotification, after: .seconds(3))
        observe(workspace, NSWorkspace.sessionDidBecomeActiveNotification, after: .seconds(3))
        observe(DistributedNotificationCenter.default(), Notification.Name("com.apple.screenIsUnlocked"), after: .seconds(3))
        observe(NotificationCenter.default, .NSSystemClockDidChange, after: .seconds(1))
    }

    /// Arms the timer for `due`; nil (paused, or only when asked) disarms it. A time in the past fires at once.
    func schedule(at due: Date?) {
        timer?.invalidate()
        timer = nil
        guard let due else { return }
        let wait = max(due.timeIntervalSinceNow, 0.5)
        let timer = Timer(fire: Date.now.addingTimeInterval(wait), interval: 0, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.onDue() }
        }
        // Up to 5% late (at most a minute) lets macOS coalesce the wake-up with other timers.
        timer.tolerance = min(60, max(1, wait * 0.05))
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func observe(_ center: NotificationCenter, _ name: Notification.Name, after delay: Duration) {
        let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.check(after: delay) }
        }
        observers.append((center, token))
    }

    /// One check after `delay`; a later event replaces an earlier pending one.
    private func check(after delay: Duration) {
        pendingCheck?.cancel()
        pendingCheck = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled else { return }
            self?.onDue()
        }
    }
}
