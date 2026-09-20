import Foundation

struct ManagementSnapshot: Decodable {
    let schemaVersion: Int
    let providers: [ManagedProvider]
    let readonlyPi: [String]
    let errors: [SectionError]
}
struct ManagedProvider: Codable, Equatable, Identifiable {
    var client: String
    var id: String
    var revision: String?
    var name: String
    var enabled: Bool
    var baseUrl: String
    var endpointRedacted: Bool
    var apiFormat: String
    var credentialKind: String
    var credentialPresent: Bool
    var credentialChange: String?
    var secret: String?
    var defaultModel: String
    var models: [ManagedModel]
    var aliases: ModelAliases
    var subagentModel: String?
    var fallbackModels: [String]
    var identity: String { client + ":" + id }
    static func new(client: String) -> ManagedProvider {
        ManagedProvider(client: client, id: "", name: "", enabled: true, baseUrl: "https://", endpointRedacted: false,
                        apiFormat: "anthropic", credentialKind: "bearer", credentialPresent: false, credentialChange: "replace", secret: "",
                        defaultModel: "", models: [], aliases: ModelAliases(), fallbackModels: [])
    }
}
struct ManagedModel: Codable, Equatable {
    var id: String = ""
    var label: String?
    var description: String?
    var maxOutputTokens: Int?
    var contextWindow: Int?
    var reasoningMax: String?
    var enabled: Bool = true
}
struct ModelAliases: Codable, Equatable {
    var opus: String?
    var sonnet: String?
    var haiku: String?
    var fable: String?
}
extension JSONEncoder {
    static var dashboard: JSONEncoder {
        let value = JSONEncoder(); value.keyEncodingStrategy = .convertToSnakeCase; return value
    }
}

func canonicalModel(_ id: String) -> String {
    id.lowercased().hasSuffix("[1m]") ? String(id.dropLast(4)) : id
}
