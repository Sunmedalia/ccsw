import Foundation

struct DashboardSnapshot: Decodable {
    let schemaVersion: Int
    let generatedAt: TimeInterval
    let configPath: String
    let accounts: [SubscriptionAccount]
    let clients: [ClientConfiguration]
    let proxy: ProxyStatus?
    let usage: UsageSummary?
    let errors: [SectionError]

    static func decode(_ data: Data) throws -> DashboardSnapshot {
        let value = try decoder().decode(Self.self, from: data)
        guard value.schemaVersion == 1 else { throw BridgeError.incompatible }
        return value
    }
}
func decoder() -> JSONDecoder {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
}
struct SubscriptionAccount: Decodable, Identifiable {
    let id: String
    let name: String
    let email: String
    let workspace: String
    let plan: String?
    let selected: Bool
    let localLogin: Bool
    let refreshedAt: TimeInterval?
    let refreshFailed: Bool
    let windows: [QuotaWindow]
    var title: String { name.isEmpty ? email : name }
}
struct QuotaWindow: Decodable, Identifiable {
    let id: String
    let bucket: String
    let window: String
    let usedPercent: Double?
    let durationMinutes: Int?
    let resetsAt: TimeInterval?
    var fraction: Double { min(1, max(0, (usedPercent ?? 0) / 100)) }
}
struct ClientConfiguration: Decodable, Identifiable {
    let id: String
    let provider: String?
    let model: String?
    let status: String
    let providers: [Provider]
    var title: String { ["claude": "Claude Code", "codex": "Codex", "pi": "Pi Agent"][id] ?? id }
    var providerName: String? { providers.first(where: { $0.id == provider })?.name ?? provider }
}
struct Provider: Decodable, Identifiable {
    let id: String
    let name: String
    let enabled: Bool
    let defaultModel: String
    let models: [ConfiguredModel]
}
struct ConfiguredModel: Decodable, Identifiable { let id: String; let name: String }
struct ProxyStatus: Decodable { let running: Bool; let listen: String; let routes: Int }
struct SectionError: Decodable { let section: String; let code: String }
struct UsageSummary: Decodable {
    let available: Bool
    let offsetSeconds: Int
    let ranges: [UsageRange]
}
struct UsageRange: Decodable {
    let days: Int
    let client: String
    let totals: UsageTotals
    let points: [UsagePoint]
}
struct UsagePoint: Decodable, Identifiable {
    let label: String
    let totals: UsageTotals
    var id: String { label }
}
struct UsageTotals: Decodable {
    let calls: Int64
    let input: Int64
    let output: Int64
    let unknown: Int64
    let failed: Int64
    let pending: Int64
    let interrupted: Int64
    let cacheRead: Int64
    let cacheWrite: Int64
    func tokenLabel(_ value: Int64) -> String {
        if calls > 0 && unknown == calls { return "?" }
        return compact(value) + (unknown > 0 ? " + ?" : "")
    }
}
struct ActionResult: Decodable {
    let schemaVersion: Int
    let ok: Bool
    let restartRequired: Bool
    let error: String?
    let saved: Bool?
    let syncConflicts: [String]?
}
func compact(_ value: Int64) -> String {
    if value >= 1_000_000 { return String(format: "%.1fM", Double(value) / 1_000_000) }
    if value >= 10_000 { return String(format: "%.1fK", Double(value) / 1_000) }
    return value.formatted()
}

struct RefreshPolicy {
    private(set) var attempts: [String: Date] = [:]
    func isDue(_ account: SubscriptionAccount, at now: Date = Date()) -> Bool {
        let successful = account.refreshedAt.map(Date.init(timeIntervalSince1970:)) ?? .distantPast
        let latest = max(successful, attempts[account.id] ?? .distantPast)
        return now.timeIntervalSince(latest) >= 300
    }
    mutating func attempted(_ id: String, at date: Date = Date()) { attempts[id] = date }
}
