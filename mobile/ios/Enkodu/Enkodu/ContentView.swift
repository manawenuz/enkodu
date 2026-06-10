import SwiftUI

struct ContentView: View {
    @State private var isUpgradeEnabled = false
    @State private var isChecking = true
    @State private var showSettings = false
    @State private var showQueue = false
    @AppStorage("serverURL") private var serverURL: String = ""

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                if isChecking {
                    ProgressView("Checking device capability...")
                } else if !isUpgradeEnabled {
                    capabilityGateView
                } else {
                    mainView
                }
            }
            .padding()
            .navigationTitle("Enkodu")
        }
        .task {
            await checkCapability()
        }
        .sheet(isPresented: $showSettings) {
            SettingsView()
        }
        .sheet(isPresented: $showQueue) {
            QueueStatusView()
        }
    }

    private var capabilityGateView: some View {
        VStack(spacing: 16) {
            Image(systemName: "play.slash.fill")
                .font(.system(size: 64))
                .foregroundColor(.secondary)

            Text("AV1 Not Supported")
                .font(.title2)
                .fontWeight(.semibold)

            Text("This device cannot play AV1 efficiently. The AV1 upgrade flow is disabled.")
                .multilineTextAlignment(.center)
                .foregroundColor(.secondary)

            Text("You can still view queue status and monitor jobs.")
                .font(.caption)
                .foregroundColor(.secondary)

            Button("View Queue") {
                showQueue = true
            }
            .padding(.top)

            Button("Settings") {
                showSettings = true
            }
        }
    }

    private var mainView: some View {
        VStack(spacing: 16) {
            Text("Ready to upgrade videos")
                .font(.title2)

            VideoPickerView { url in
                guard SettingsView.validateServerURL(serverURL) else {
                    showSettings = true
                    return
                }
                Task {
                    await startUpload(url: url)
                }
            }
            .buttonStyle(.borderedProminent)

            Button("View Queue") {
                showQueue = true
            }

            Button("Settings") {
                showSettings = true
            }
        }
    }

    private func checkCapability() async {
        let result = await Av1CapabilityChecker.check()
        isUpgradeEnabled = result.supported
        isChecking = false
    }

    private func startUpload(url: URL) async {
        do {
            let api = EnkoduApi(serverURL: serverURL)
            let manager = TransferManager.shared
            let result = try await manager.uploadFile(file: url) { bytes, total in
                print("Upload progress: \(bytes)/\(total)")
            }
            print("Upload complete: job \(result.jobId)")
        } catch {
            print("Upload failed: \(error)")
        }
    }
}
