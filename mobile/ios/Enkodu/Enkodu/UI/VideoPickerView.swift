import SwiftUI
import PhotosUI

struct VideoPickerView: View {
    @State private var selectedItem: PhotosPickerItem?
    @State private var selectedVideoURL: URL?
    @State private var isLoading = false
    var onVideoSelected: (URL) -> Void

    var body: some View {
        PhotosPicker(
            selection: $selectedItem,
            matching: .videos,
            photoLibrary: .shared()
        ) {
            Label("Select Video", systemImage: "photo.on.rectangle.angled")
        }
        .onChange(of: selectedItem) { _, newItem in
            guard let item = newItem else { return }
            isLoading = true
            Task {
                if let data = try? await item.loadTransferable(type: Data.self) {
                    let url = FileManager.default.temporaryDirectory
                        .appendingPathComponent("picked_\(UUID().uuidString).mp4")
                    try? data.write(to: url)
                    await MainActor.run {
                        selectedVideoURL = url
                        isLoading = false
                        onVideoSelected(url)
                    }
                } else {
                    await MainActor.run {
                        isLoading = false
                    }
                }
            }
        }
    }
}
