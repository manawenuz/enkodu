import SwiftUI

struct ErrorAlert: ViewModifier {
    @Binding var isPresented: Bool
    let title: String
    let message: String
    let retryAction: (() -> Void)?

    func body(content: Content) -> some View {
        content
            .alert(title, isPresented: $isPresented) {
                if let retry = retryAction {
                    Button("Retry", role: .none, action: retry)
                    Button("Cancel", role: .cancel) {}
                } else {
                    Button("OK", role: .cancel) {}
                }
            } message: {
                Text(message)
            }
    }
}

extension View {
    func errorAlert(
        isPresented: Binding<Bool>,
        title: String = "Error",
        message: String,
        retryAction: (() -> Void)? = nil
    ) -> some View {
        modifier(ErrorAlert(
            isPresented: isPresented,
            title: title,
            message: message,
            retryAction: retryAction
        ))
    }
}

struct ErrorStateView: View {
    let title: String
    let message: String
    let retryAction: (() -> Void)?

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 48))
                .foregroundColor(.red)

            Text(title)
                .font(.title3)
                .fontWeight(.semibold)

            Text(message)
                .font(.body)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)

            if let retry = retryAction {
                Button("Retry", action: retry)
                    .buttonStyle(.borderedProminent)
                    .padding(.top, 8)
            }
        }
        .padding(24)
    }
}

struct TransferErrorCard: View {
    let error: String
    let retryAction: () -> Void
    let dismissAction: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            HStack {
                Image(systemName: "exclamationmark.circle.fill")
                    .foregroundColor(.red)
                Text("Transfer Failed")
                    .font(.headline)
                Spacer()
            }

            Text(error)
                .font(.subheadline)
                .foregroundColor(.secondary)

            HStack(spacing: 16) {
                Button("Dismiss", action: dismissAction)
                    .buttonStyle(.bordered)
                Button("Retry", action: retryAction)
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(16)
        .background(Color.red.opacity(0.08))
        .cornerRadius(12)
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Color.red.opacity(0.2), lineWidth: 1)
        )
    }
}

enum UserFacingError: LocalizedError {
    case serverUnreachable
    case uploadFailed
    case downloadFailed
    case checksumMismatch
    case av1NotSupported
    case networkUnavailable
    case batteryTooLow
    case diskFull

    var errorDescription: String? {
        switch self {
        case .serverUnreachable:
            return "Cannot reach the Enkodu server. Check your network connection and server URL in settings."
        case .uploadFailed:
            return "Upload failed after multiple retries. The server may be busy or your network connection is unstable."
        case .downloadFailed:
            return "Download failed. Please check your connection and try again."
        case .checksumMismatch:
            return "The downloaded file appears corrupted. It has been removed for your safety."
        case .av1NotSupported:
            return "This device cannot play AV1 videos efficiently. The upgrade feature is disabled."
        case .networkUnavailable:
            return "No network connection available. Please connect to WiFi or enable cellular data."
        case .batteryTooLow:
            return "Battery too low for transfers. Please charge your device and try again."
        case .diskFull:
            return "Not enough storage space. Please free up space and try again."
        }
    }

    var recoverySuggestion: String? {
        switch self {
        case .serverUnreachable:
            return "Check Settings > Server URL and verify the server is running."
        case .uploadFailed, .downloadFailed:
            return "Try again when you have a stronger connection."
        case .checksumMismatch:
            return "The job will need to be re-processed. Contact support if this persists."
        case .av1NotSupported:
            return "You can still monitor queue status and view job progress."
        case .networkUnavailable:
            return "Enable WiFi or cellular data in your device settings."
        case .batteryTooLow:
            return "Connect your device to a charger."
        case .diskFull:
            return "Delete unused apps or media to free up space."
        }
    }
}
