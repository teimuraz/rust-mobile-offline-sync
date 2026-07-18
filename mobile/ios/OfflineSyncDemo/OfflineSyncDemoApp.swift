import SwiftUI

// This app is ONE device. All data operations go through the Rust `ItemService`
// (fully offline-capable); the Rust `SyncRunner` pushes/pulls against the backend.
// There is no Swift reimplementation of the event log, the fold, the sync, or the
// networking.
//
// Requires the backend running:  cargo run -p backend --bin server
// To see multi-device convergence, run the app in two simulators side by side —
// each install gets its own replica id — or check `curl localhost:4000/items`.

private let backendURL = "http://127.0.0.1:4000"

/// One replica id per install, generated on first launch and persisted — the
/// demo-scale analog of the real app's replica id stored in the local database.
private func installReplicaId() -> String {
    let key = "replica_id"
    if let existing = UserDefaults.standard.string(forKey: key) { return existing }
    let fresh = "ios-" + UUID().uuidString.prefix(8).lowercased()
    UserDefaults.standard.set(fresh, forKey: key)
    return fresh
}

@main
struct OfflineSyncDemoApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

/// Bridges the Rust sync runner's "data changed" callback (fired from a background
/// task) back onto the main actor so the UI refreshes.
final class ItemsChangedListener: SyncListener {
    private let onChange: @MainActor () async -> Void

    init(onChange: @escaping @MainActor () async -> Void) {
        self.onChange = onChange
    }

    func onItemsChanged() {
        Task { @MainActor in await self.onChange() }
    }
}

/// This device: local-only `ItemService` for data, `SyncRunner` for syncing.
@MainActor
final class DeviceVM: ObservableObject {
    let replicaId: String
    let service: ItemService
    let runner: SyncRunner
    @Published var items: [ItemView] = []

    /// Starts/stops the Rust background sync job. On by default.
    @Published var backgroundSync = true {
        didSet {
            if backgroundSync {
                let runner = self.runner
                Task { await runner.start(intervalMs: 5000) }
            } else {
                runner.stop()
            }
        }
    }

    /// Simulated connectivity, tracked by the Rust runner. While off, edits pile
    /// up locally; flipping back on triggers an immediate sync (sync-on-reconnect).
    @Published var online = true {
        didSet {
            let (runner, online) = (self.runner, self.online)
            Task { await runner.setOnline(online: online) }
        }
    }

    init() {
        // Wire like the real SDK: EventSourcedStores builds the event log, the
        // storage, and the replica identity; the service and the runner share them.
        let replicaId = installReplicaId()
        let stores = EventSourcedStores(replicaId: replicaId)
        self.replicaId = replicaId
        self.service = ItemService(stores: stores)
        self.runner = SyncRunner(stores: stores, serverUrl: backendURL)
        // Refresh the list whenever a (background) sync pulled changes.
        runner.setListener(listener: ItemsChangedListener { [weak self] in
            await self?.refresh()
        })
        // Background sync is on by default (didSet doesn't fire for the initial
        // property value, so start the job explicitly).
        let runner = self.runner
        Task {
            await runner.start(intervalMs: 5000)
        }
        Task { await refresh() }
    }

    func refresh() async {
        items = await service.items()
    }

    func addRandomItem() async {
        let names = ["Wrench", "Bolt", "Nut", "Gear", "Valve", "Pipe", "Sensor", "Cable"]
        _ = await service.createItem(name: names.randomElement()!, quantity: Int64.random(in: 1...9))
        await refresh()
    }

    func changeQuantity(_ item: ItemView, by delta: Int64) async {
        await service.setQuantity(id: item.id, quantity: max(0, item.quantity + delta))
        await refresh()
    }

    func delete(_ item: ItemView) async {
        await service.delete(id: item.id)
        await refresh()
    }

    /// Manual sync round against the backend.
    func sync() async {
        do {
            try await runner.syncNow()
            await refresh()
        } catch {
            // Backend unreachable, etc. Local state is intact; next sync retries.
            print("sync failed: \(error)")
        }
    }
}

struct ContentView: View {
    @StateObject private var device = DeviceVM()

    var body: some View {
        NavigationStack {
            List {
                Section {
                    HStack {
                        Image(systemName: "server.rack")
                        Text("Backend")
                        Spacer()
                        Text(backendURL)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Image(systemName: "iphone")
                        Text("This device")
                        Spacer()
                        Text(device.replicaId)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Toggle(isOn: $device.online) {
                        Label("Online", systemImage: device.online ? "wifi" : "wifi.slash")
                    }
                    Toggle("Background sync (every 5s)", isOn: $device.backgroundSync)
                }

                Section("Items") {
                    if device.items.isEmpty {
                        Text("No items yet — add some. Works offline; Sync when you're back online.")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(device.items, id: \.id) { item in
                        ItemRow(
                            item: item,
                            onInc: { Task { await device.changeQuantity(item, by: 1) } },
                            onDec: { Task { await device.changeQuantity(item, by: -1) } },
                            onDelete: { Task { await device.delete(item) } }
                        )
                    }
                }
            }
            .navigationTitle("Offline Sync")
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        Task { await device.addRandomItem() }
                    } label: {
                        Label("Add item", systemImage: "plus")
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task { await device.sync() }
                    } label: {
                        Label("Sync", systemImage: "arrow.triangle.2.circlepath")
                    }
                }
            }
        }
    }
}

struct ItemRow: View {
    let item: ItemView
    let onInc: () -> Void
    let onDec: () -> Void
    let onDelete: () -> Void

    var body: some View {
        HStack {
            Text(item.name)
            Spacer()
            Button(action: onDec) { Image(systemName: "minus.circle") }
                .buttonStyle(.plain)
            Text("\(item.quantity)")
                .monospacedDigit()
                .frame(minWidth: 24)
            Button(action: onInc) { Image(systemName: "plus.circle") }
                .buttonStyle(.plain)
            Button(action: onDelete) { Image(systemName: "trash") }
                .buttonStyle(.plain)
                .foregroundStyle(.red)
                .padding(.leading, 8)
        }
    }
}
