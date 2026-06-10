import SwiftUI

struct OnboardingView: View {
    @AppStorage("hasCompletedOnboarding") private var hasCompletedOnboarding: Bool = false
    @State private var step = 0
    @State private var serverURL = ""
    @State private var companionToken = ""
    @State private var isTesting = false
    @State private var testResult: String?
    @State private var testSuccess = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        if hasCompletedOnboarding {
            ContentView()
        } else {
            onboardingContent
        }
    }

    private var onboardingContent: some View {
        VStack(spacing: 24) {
            if step == 0 {
                welcomeView
            } else {
                serverSetupView
            }
        }
        .padding(24)
    }

    private var welcomeView: some View {
        VStack(spacing: 16) {
            Text("Welcome to Enkodu")
                .font(.largeTitle)
                .fontWeight(.bold)
                .multilineTextAlignment(.center)

            Text("Enkodu converts your videos to efficient AV1 format while preserving quality.")
                .font(.body)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)

            VStack(alignment: .leading, spacing: 8) {
                Text("Before you begin:")
                    .font(.headline)
                Text("1. You need an Enkodu server")
                Text("2. Your device must support AV1 hardware decoding")
                Text("3. Uploads work best on WiFi")
            }
            .font(.body)
            .foregroundColor(.secondary)

            Spacer()

            Button("Get Started") {
                step = 1
            }
            .buttonStyle(.borderedProminent)

            Button("Skip") {
                hasCompletedOnboarding = true
            }
            .buttonStyle(.borderless)
        }
    }

    private var serverSetupView: some View {
        VStack(spacing: 16) {
            Text("Server Setup")
                .font(.title)
                .fontWeight(.bold)

            Text("Enter your Enkodu server URL:")
                .font(.body)
                .foregroundColor(.secondary)

            TextField("https://your-server.com", text: $serverURL)
                .keyboardType(.URL)
                .autocapitalization(.none)
                .textFieldStyle(.roundedBorder)

            SecureField("Companion token", text: $companionToken)
                .textFieldStyle(.roundedBorder)

            if let result = testResult {
                Text(result)
                    .font(.caption)
                    .foregroundColor(testSuccess ? .green : .red)
            }

            Button("Test Connection") {
                testConnection()
            }
            .disabled(isTesting || serverURL.isEmpty)
            .buttonStyle(.bordered)

            Spacer()

            Button("Continue") {
                UserDefaults.standard.set(serverURL, forKey: "serverURL")
                hasCompletedOnboarding = true
            }
            .disabled(serverURL.isEmpty || !testSuccess)
            .buttonStyle(.borderedProminent)

            Button("Skip for now") {
                hasCompletedOnboarding = true
            }
            .buttonStyle(.borderless)
        }
    }

    private func testConnection() {
        guard !serverURL.isEmpty else { return }
        isTesting = true
        testResult = nil

        Task {
            do {
                let token = companionToken
                let api = EnkoduApi(serverURL: serverURL, authTokenProvider: { token })
                let authResult = await api.testProtectedConnection()
                if authResult == .connected {
                    AuthStore.shared.serverURL = serverURL
                    try AuthStore.shared.saveDeviceToken(token)
                    AuthStore.shared.authState = .connected
                } else {
                    AuthStore.shared.authState = authResult.authState
                }
                await MainActor.run {
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
}
