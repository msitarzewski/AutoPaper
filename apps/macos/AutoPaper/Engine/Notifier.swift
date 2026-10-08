import AutopaperCore
import Foundation
import UserNotifications

/// Notifications, off on macOS unless the person turns them on (Settings → General): "New wallpaper: <title>" with
/// Like, Dislike and Show for each new wallpaper made on schedule, and one each when the month's budget runs out
/// and when a key stops working. Never for routine failures (offline, a provider that's down).
final class Notifier: NSObject, UNUserNotificationCenterDelegate, Sendable {
    static let shared = Notifier()

    static let enabledKey = "notifyNewWallpapers"
    private static let category = "new-wallpaper"
    private static let like = "like"
    private static let dislike = "dislike"
    private static let show = "show"
    private static let generationKey = "generation"
    /// A key notification's Keychain account: clicking it opens Accounts with that key's field focused.
    private static let accountKey = "account"
    /// The Settings pane a problem's notification opens when clicked (spec 6a: the notification is the link).
    private static let paneKey = "pane"
    /// Remembered so each warning is sent once: the budget per month, keys until a wallpaper is made again.
    private static let budgetWarnedKey = "notifiedBudgetMonth"
    private static let keysWarnedKey = "notifiedKeyProblem"

    var isEnabled: Bool { UserDefaults.standard.bool(forKey: Self.enabledKey) }

    /// Registers the actions and becomes the delegate; called at launch, so a click on a notification posted
    /// before the app quit still arrives.
    func register() {
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let actions = [
            UNNotificationAction(identifier: Self.like, title: "Like"),
            UNNotificationAction(identifier: Self.dislike, title: "Dislike"),
            UNNotificationAction(identifier: Self.show, title: "Show", options: [.foreground]),
        ]
        center.setNotificationCategories([UNNotificationCategory(identifier: Self.category, actions: actions, intentIdentifiers: [])])
    }

    /// Asks macOS for permission when the person turns notifications on. Returns whether they're allowed.
    func requestPermission() async -> Bool {
        (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])) ?? false
    }

    func newWallpaper(_ generation: Generation) {
        clearKeyWarning()
        guard isEnabled else { return }
        let content = UNMutableNotificationContent()
        content.title = "New wallpaper: \(generation.concept.title)"
        content.body = generation.concept.summary
        content.categoryIdentifier = Self.category
        content.userInfo = [Self.generationKey: generation.id]
        post(content, id: "wallpaper-\(generation.id)")
    }

    /// A new wallpaper was made, so keys work again: the next key problem may be told once more.
    func clearKeyWarning() {
        UserDefaults.standard.removeObject(forKey: Self.keysWarnedKey)
    }

    /// Once per budget month. The notification is the link to the fix (spec 6a): it ends with the action ("Raise
    /// the budget.") and clicking it opens Settings → Budget.
    func budgetSpent(_ status: BudgetStatus) {
        let month = status.month
        guard status.blocked, isEnabled, UserDefaults.standard.string(forKey: Self.budgetWarnedKey) != month else { return }
        UserDefaults.standard.set(month, forKey: Self.budgetWarnedKey)
        let problem = SettingsProblem.budget(status) ?? SettingsProblem.overBudget(bringingBack: false)
        let content = UNMutableNotificationContent()
        content.title = "New wallpapers are waiting for the budget"
        content.body = problem.spoken
        content.userInfo = [Self.paneKey: SettingsPaneID.budget.rawValue]
        post(content, id: "budget-\(month)")
    }

    /// Once, until a new wallpaper is made again. The notification is the link to the fix (spec 6a): its text is
    /// the action ("Add your OpenAI key") and clicking it opens Settings → Accounts with that key's field focused.
    func keyProblem(_ problem: SettingsProblem) {
        guard isEnabled, !UserDefaults.standard.bool(forKey: Self.keysWarnedKey) else { return }
        UserDefaults.standard.set(true, forKey: Self.keysWarnedKey)
        let content = UNMutableNotificationContent()
        content.title = "AutoPaper can't make new wallpapers"
        content.body = problem.spoken
        var info: [String: String] = [Self.paneKey: SettingsPaneID.accounts.rawValue]
        if case .accounts(let account?) = problem.place { info[Self.accountKey] = account }
        content.userInfo = info
        post(content, id: "keys")
    }

    private func post(_ content: UNMutableNotificationContent, id: String) {
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: id, content: content, trigger: nil)) { error in
            if let error { log.error("Notification failed: \(error.localizedDescription, privacy: .public)") }
        }
    }

    // MARK: UNUserNotificationCenterDelegate

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .list]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let action = response.actionIdentifier
        let info = response.notification.request.content.userInfo
        let id = info[Self.generationKey] as? String
        let account = info[Self.accountKey] as? String
        // A problem's notification opens where it's fixed; a key notification from before panes were recorded is
        // still a key's.
        let pane = (info[Self.paneKey] as? String).flatMap(SettingsPaneID.init(rawValue:))
            ?? (response.notification.request.identifier == "keys" ? .accounts : nil)
        await MainActor.run {
            let model = AppModel.shared
            switch action {
            case Self.like: if let id { model.rate(id: id, .liked, toggle: false) }
            case Self.dislike: if let id { model.rate(id: id, .disliked, toggle: false) }
            default:
                if let pane { model.openSettings(pane, focusing: account) } else { model.showMainWindow() }
            }
        }
    }
}
