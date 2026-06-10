import SwiftUI

struct QueueStatusView: View {
    @State private var status: StatusResponse?
    @State private var isLoading = false
    @State private var error: String?
    @AppStorage("serverURL") private var serverURL: String = ""

    var body: some View {
        NavigationStack {
            VStack {
                if isLoading && status == nil {
                    ProgressView("Loading...")
                } else if let error = error, status == nil {
                    VStack(spacing: 16) {
                        Text("Error: \(error)")
                            .foregroundColor(.red)
                        Button("Retry") {
                            refreshData()
                        }
                    }
                } else if let status = status {
                    statusView(status)
                }
            }
            .padding()
            .navigationTitle("Queue Status")
            .refreshable {
                await refreshDataAsync()
            }
        }
        .task {
            refreshData()
        }
    }

    private func statusView(_ status: StatusResponse) -> some View {
        VStack(spacing: 16) {
            HStack(spacing: 24) {
                StatusBadge(label: "Pending", value: status.pending, color: .orange)
                StatusBadge(label: "Active", value: status.active, color: .blue)
                StatusBadge(label: "Done", value: status.done, color: .green)
                StatusBadge(label: "Failed", value: status.failed, color: .red)
            }

            Text("Active Jobs")
                .font(.headline)
                .frame(maxWidth: .infinity, alignment: .leading)

            Text("Active job details would appear here")
                .font(.body)
                .foregroundColor(.secondary)
        }
    }

    private func refreshData() {
        if serverURL.isEmpty {
            error = "Server URL not configured"
            return
        }
        isLoading = true
        error = nil

        Task {
            await refreshDataAsync()
        }
    }

    private func refreshDataAsync() async {
        do {
            let api = EnkoduApi(serverURL: serverURL)
            let data = try await api.getStatus()
            await MainActor.run {
                status = data
                isLoading = false
            }
        } catch {
            await MainActor.run {
                self.error = error.localizedDescription
                isLoading = false
            }
        }
    }
}

struct StatusBadge: View {
    let label: String
    let value: Int
    let color: Color

    var body: some View {
        VStack {
            Text("\(value)")
                .font(.title2)
                .fontWeight(.bold)
                .foregroundColor(color)
            Text(label)
                .font(.caption)
                .foregroundColor(.secondary)
        }
        .frame(width: 72, height: 72)
        .background(color.opacity(0.15))
        .cornerRadius(12)
    }
}
