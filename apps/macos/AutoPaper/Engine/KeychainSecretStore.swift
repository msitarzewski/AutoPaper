import AutopaperCore
import Foundation
import Security
import Synchronization

/// The engine's `SecretStore`: API keys in the login Keychain (generic passwords under the service
/// `com.autopaper.credentials`, one account per key, named by the core: `openai.api_key`, `google.api_key`,
/// `openai_compatible.api_key@<origin>`). AudioPaper's approach: never written to defaults or disk, read once
/// and remembered (the Keychain is slow and can block on an access prompt), and saves report whether the
/// Keychain took them, so Settings never says "Saved" for a key that wasn't.
///
/// The core calls it from its own threads, never the main thread; Settings calls it from a detached task.
final class KeychainSecretStore: SecretStore {
    static let shared = KeychainSecretStore()

    private let service: String
    /// Values already read (nil = known to be absent), shared by every caller so a save in Settings is seen by
    /// the engine at once.
    private let memo = Mutex<[String: String?]>([:])

    init(service: String = "com.autopaper.credentials") {
        self.service = service
    }

    // MARK: SecretStore (called by the core)

    func get(account: String) -> String? {
        if let known = memo.withLock({ $0[account] }) { return known }
        switch read(account) {
        case .found(let value):
            memo.withLock { $0[account] = .some(value) }
            return value
        case .absent:
            memo.withLock { $0[account] = .some(nil) }
            return nil
        case .failed:
            // A locked keychain or a denied access prompt isn't "no key": not remembered, so the next call reads
            // again instead of reporting a missing key until AutoPaper is relaunched.
            return nil
        }
    }

    func set(account: String, value: String) {
        save(value, account: account)
    }

    func delete(account: String) {
        save(nil, account: account)
    }

    // MARK: Settings

    /// Saves `value` (or removes the key when it is nil or empty after trimming). Returns whether the Keychain
    /// accepted it. An existing key is updated in place, so a save that fails (a locked keychain, a denied access
    /// prompt) leaves the previous key working.
    @discardableResult
    func save(_ value: String?, account: String) -> Bool {
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let trimmed, !trimmed.isEmpty else {
            let deleted = SecItemDelete(base as CFDictionary)
            guard deleted == errSecSuccess || deleted == errSecItemNotFound else {
                log.error("Keychain delete failed for \(account, privacy: .public): \(deleted)")
                memo.withLock { $0[account] = nil }  // unknown now: read again next time
                return false
            }
            memo.withLock { $0[account] = .some(nil) }
            return true
        }
        let data = Data(trimmed.utf8)
        var status = SecItemUpdate(base as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var add = base
            add[kSecValueData as String] = data
            add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
            add[kSecAttrLabel as String] = "AutoPaper: \(account)"
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else {
            log.error("Keychain save failed for \(account, privacy: .public): \(status)")
            memo.withLock { $0[account] = nil }  // the previous key may still be there: read again next time
            return false
        }
        memo.withLock { $0[account] = .some(trimmed) }
        return true
    }

    /// What the Keychain said: the key, no key, or an error (which says nothing about whether there is one).
    private enum Read {
        case found(String)
        case absent
        case failed
    }

    private func read(_ account: String) -> Read {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            guard let data = item as? Data, let string = String(data: data, encoding: .utf8), !string.isEmpty else { return .absent }
            return .found(string)
        case errSecItemNotFound:
            return .absent
        default:
            log.error("Keychain read failed for \(account, privacy: .public): \(status)")
            return .failed
        }
    }
}
