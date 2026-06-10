import SwiftUI

struct SettingsView: View {
    @AppStorage("serverURL") private var serverURL: String = ""
    @AppStorage("displayName") private var displayName: String = ""
    @AppStorage("wifiOnlyUploads") private var wifiOnlyUploads: Bool = true
    @AppStorage("wifiOnlyDownloads") private var wifiOnlyDownloads: Bool = true
    @AppStorage("maxUploadSizeMb") private var maxUploadSizeMb: Int = 100
    @AppStorage("batteryMinPercent") private var batteryMinPercent: Int = 15
    @Environment(\.dismiss) private var dismiss
    @State private var companionToken: String = ""
    @State private var authState: AuthState = AuthStore.shared.authState
    @State private var isTesting = false
    @State private var testResult: String?
    @State private var testSuccess = false
    @State private var showError = false
    @State private var errorMessage = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("Server") {
                    TextField("Server URL (https://...)", text: $serverURL)
                        .keyboardType(.URL)
                        .autocapitalization(.none)
                        .foregroundColor(testResult != nil && !testSuccess ? Color.red : Color.primary)

                    if let result = testResult {
                        HStack {
                            Text(result)
                                .font(.caption)
                                .foregroundColor(testSuccess ? Color.green : Color.red)
                            Spacer()
                        }
                    }

                    Button(action: testConnection) {
                        if isTesting {
                            HStack {
                                ProgressView()
                                    .scaleEffect(0.8)
                                Text("Testing...")
                            }
                        } else {
                            Text("Test Connection")
                        }
                    }
                    .disabled(isTesting || serverURL.isEmpty)

                    TextField("Display Name (optional)", text: $displayName)
                }

                Section("Authentication") {
                    HStack {
                        Text("Status")
                        Spacer()
                        Text(authState.displayText)
                            .foregroundColor(authState == .connected ? .green : .secondary)
                    }

                    SecureField(AuthStore.shared.companionToken == nil ? "Companion token" : "New companion token", text: $companionToken)

                    Button(action: testConnection) {
                        if isTesting {
                            HStack {
                                ProgressView()
                                    .scaleEffect(0.8)
                                Text("Testing token...")
                            }
                        } else {
                            Text("Test Token")
                        }
                    }
                    .disabled(isTesting || serverURL.isEmpty)

                    Button("Clear Token", role: .destructive) {
                        clearToken()
                    }
                }

                Section("Network") {
                    Toggle("WiFi-only uploads", isOn: $wifiOnlyUploads)
                    Toggle("WiFi-only downloads", isOn: $wifiOnlyDownloads)
                    Stepper("Max upload size on cellular: \(maxUploadSizeMb) MB", value: $maxUploadSizeMb, in: 10...1000, step: 10)
                }

                Section("Battery") {
                    Stepper("Minimum battery: \(batteryMinPercent)%", value: $batteryMinPercent, in: 5...50, step: 5)
                }
            }
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") {
                        Task {
                            await saveSettings()
                        }
                    }
                }
            }
            .alert("Error", isPresented: $showError) {
                Button("OK") { }
            } message: {
                Text(errorMessage)
            }
            .onAppear {
                authState = AuthStore.shared.authState
            }
        }
    }

    private func testConnection() {
        guard !serverURL.isEmpty else {
            testResult = "Please enter a server URL"
            testSuccess = false
            return
        }

        isTesting = true
        testResult = nil

        Task {
            do {
                let tokenForTest = companionToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? AuthStore.shared.companionToken
                    : companionToken
                let api = EnkoduApi(serverURL: serverURL, authTokenProvider: { tokenForTest })
                let authResult = await api.testProtectedConnection()
                if authResult == .connected {
                    AuthStore.shared.serverURL = serverURL
                    if !companionToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        try AuthStore.shared.saveDeviceToken(companionToken)
                    }
                    AuthStore.shared.authState = .connected
                } else {
                    AuthStore.shared.authState = authResult.authState
                }
                await MainActor.run {
                    authState = AuthStore.shared.authState
                    testSuccess = authResult == .connected
                    testResult = authResult.message
                    isTesting = false
                }
            } catch {
                await MainActor.run {
                    testSuccess = false
                    testResult = "✗ Server unreachable: \(error.localizedDescription)"
                    isTesting = false
                }
            }
        }
    }

    private func saveSettings() async {
        guard !serverURL.isEmpty else {
            errorMessage = "Please enter a server URL"
            showError = true
            return
        }

        if !SettingsView.validateServerURL(serverURL) {
            errorMessage = "Invalid URL. Must start with http:// or https://"
            showError = true
            return
        }

        do {
            let tokenForTest = companionToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? AuthStore.shared.companionToken
                : companionToken
            let api = EnkoduApi(serverURL: serverURL, authTokenProvider: { tokenForTest })
            let authResult = await api.testProtectedConnection()
            if authResult == .connected {
                AuthStore.shared.serverURL = serverURL
                if !companionToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    try AuthStore.shared.saveDeviceToken(companionToken)
                    companionToken = ""
                }
                AuthStore.shared.authState = .connected
                authState = .connected
            } else {
                AuthStore.shared.authState = authResult.authState
                authState = authResult.authState
                errorMessage = authResult.message
                showError = true
                return
            }
        } catch {
            errorMessage = "Cannot reach server: \(error.localizedDescription). Save anyway?"
            showError = true
            return
        }

        dismiss()
    }

    private func clearToken() {
        do {
            try AuthStore.shared.clearDeviceToken()
            companionToken = ""
            authState = AuthStore.shared.authState
            testResult = "Companion token cleared"
            testSuccess = false
        } catch {
            errorMessage = "Could not clear token: \(error.localizedDescription)"
            showError = true
        }
    }

    static func validateServerURL(_ url: String) -> Bool {
        return !url.isEmpty && (url.hasPrefix("http://") || url.hasPrefix("https://"))
    }
}
