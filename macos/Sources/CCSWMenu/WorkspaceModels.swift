import Foundation

struct WorkspaceState: Decodable {
    let schemaVersion: Int
    let preferences: ClientPreferences
    let preferencesRevision: String
    let syncConflicts: [String]
    let claudeSettings: String
    let codexHome: String
    let piHome: String
    let proxyAutostart: Bool
    let proxyManager: String?
    let piReadonly: [String: String]
    let importCandidate: ImportCandidate?
    let codexRecoveryNeeded: Bool
}
struct ImportCandidate: Decodable { let source: String; let name: String; let model: String; let models: Int }
struct ClientPreferences: Codable, Equatable {
    var claude: ClaudePreferences
    var reasoning: String?
}
struct ClaudePreferences: Codable, Equatable {
    var env: [String: String]
    var hideAttribution: Bool?
}
struct ProbeResult: Decodable {
    let schemaVersion: Int
    let ok: Bool
    let error: String?
    let data: ProbeData?
}
struct ProbeData: Decodable {
    let models: [CatalogModel]?
    let httpStatus: Int?
    let elapsedMs: Int?
}
struct CatalogModel: Decodable, Identifiable {
    let id: String
    let label: String?
    let description: String?
    let maxOutputTokens: Int?
    let contextWindow: Int?
    let reasoningMax: String?
    var managed: ManagedModel { ManagedModel(id: id, label: label, description: description, maxOutputTokens: maxOutputTokens, contextWindow: contextWindow, reasoningMax: reasoningMax, enabled: true) }
}
struct Ledger: Decodable {
    let schemaVersion: Int
    let available: Bool
    let today: String
    let offsetSeconds: Int
    let since: TimeInterval?
    let rows: [LedgerRow]
}
struct LedgerRow: Decodable {
    let hour: Int
    let day: String
    let client: String
    let provider: String
    let name: String
    let model: String
    let kind: String
    let totals: UsageTotals
}
extension UsageTotals {
    static var zero: UsageTotals { UsageTotals(calls: 0, input: 0, output: 0, unknown: 0, failed: 0, pending: 0, interrupted: 0, cacheRead: 0, cacheWrite: 0) }
    func adding(_ other: UsageTotals) -> UsageTotals {
        UsageTotals(calls: calls+other.calls,input:input+other.input,output:output+other.output,unknown:unknown+other.unknown,failed:failed+other.failed,pending:pending+other.pending,interrupted:interrupted+other.interrupted,cacheRead:cacheRead+other.cacheRead,cacheWrite:cacheWrite+other.cacheWrite)
    }
}
