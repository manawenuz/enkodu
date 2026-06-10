import Foundation

enum AuthMode: String, Equatable {
    case deviceToken = "device_token"
    case session
}

enum AuthState: String, Equatable {
    case notConfigured
    case connected
    case tokenRejected
    case sessionExpired
    case permissionDenied
    case serverUnreachable

    var displayText: String {
        switch self {
        case .notConfigured:
            return "Not configured"
        case .connected:
            return "Connected"
        case .tokenRejected:
            return "Token rejected"
        case .sessionExpired:
            return "Session expired"
        case .permissionDenied:
            return "Permission denied"
        case .serverUnreachable:
            return "Server unreachable"
        }
    }
}

final class AuthStore {
    static let shared = AuthStore()

    private enum Keys {
        static let serverURL = "serverURL"
        static let authMode = "authMode"
        static let authState = "authState"
        static let companionToken = "companionToken"
    }

    private let defaults: UserDefaults
    private let keychain: KeychainStore

    init(defaults: UserDefaults = .standard, keychain: KeychainStore = KeychainStore()) {
        self.defaults = defaults
        self.keychain = keychain
    }

    var serverURL: String {
        get { defaults.string(forKey: Keys.serverURL) ?? "" }
        set { defaults.set(newValue, forKey: Keys.serverURL) }
    }

    var authMode: AuthMode {
        get {
            AuthMode(rawValue: defaults.string(forKey: Keys.authMode) ?? "") ?? .deviceToken
        }
        set {
            defaults.set(newValue.rawValue, forKey: Keys.authMode)
        }
    }

    var authState: AuthState {
        get {
            AuthState(rawValue: defaults.string(forKey: Keys.authState) ?? "") ?? .notConfigured
        }
        set {
            defaults.set(newValue.rawValue, forKey: Keys.authState)
        }
    }

    var companionToken: String? {
        try? keychain.read(account: Keys.companionToken)
    }

    func saveDeviceToken(_ token: String) throws {
        let trimmed = token.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            try clearDeviceToken()
            return
        }
        try keychain.save(trimmed, account: Keys.companionToken)
        authMode = .deviceToken
    }

    func clearDeviceToken() throws {
        try keychain.delete(account: Keys.companionToken)
        if authMode == .deviceToken {
            authState = .notConfigured
        }
    }
}
