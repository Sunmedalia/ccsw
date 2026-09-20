import Foundation
import SwiftUI

@MainActor final class MenuStore: ObservableObject {
    @Published var snapshot: DashboardSnapshot?
    @Published var loading = false
    @Published var busy = false
    @Published var message: String?
    @Published var messageIsError = false
    @Published var visible = false
    @Published var managementVisible = false
    @Published var managementLoading = false
    @Published var selectedClient: String = "claude"
    @Published var management: ManagementSnapshot?
    @Published var workspace: WorkspaceState?
    @Published var ledger: Ledger?
    @Published var workspaceLoading = false
    @Published var ledgerLoading = false
    @Published var preferencesDirty = false
    @Published var language: String { didSet { defaults.set(language, forKey: "language") } }
    private let defaults: UserDefaults
    private let execute: ([String], [String: String]) async throws -> Data
    private let write: ([String], [String: String], Data) async throws -> Data
    private var loop: Task<Void, Never>?
    private var refreshPolicy = RefreshPolicy()
    private var snapshotFailed = false
    var isChinese: Bool {
        language == "zh-Hans" || (language == "system" && Locale.preferredLanguages.first?.hasPrefix("zh") == true)
    }
    func t(_ zh: String, _ en: String) -> String { isChinese ? zh : en }
    var currentAccount: SubscriptionAccount? {
        guard let accounts = snapshot?.accounts else { return nil }
        return accounts.first(where: \.selected) ?? accounts.first(where: \.localLogin) ?? accounts.first
    }
    var activeAccount: SubscriptionAccount? {
        guard let client = snapshot?.clients.first(where: { $0.id == "codex" }), client.provider == "openai" else { return nil }
        return snapshot?.accounts.first(where: \.localLogin)
    }
    init(defaults: UserDefaults = .standard,
         execute: @escaping ([String], [String: String]) async throws -> Data = { try await Bridge.run($0, environment: $1) },
         write: @escaping ([String], [String: String], Data) async throws -> Data = { try await Bridge.run($0, environment: $1, input: $2) }) {
        self.defaults = defaults; self.execute = execute; self.write = write
        language = defaults.string(forKey: "language") ?? "system"
    }
    func start() {
        guard loop == nil else { return }
        loop = Task { [weak self] in
            await self?.refresh()
            while !Task.isCancelled {
                await self?.refreshActiveIfDue()
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                guard !Task.isCancelled else { break }
                if self?.visible == true || self?.managementVisible == true { await self?.refresh() }
                if self?.managementVisible == true { await self?.refreshManagement(); await self?.refreshWorkspace() }
            }
        }
    }
    func stop() { loop?.cancel(); loop = nil }
    func refresh() async {
        guard !loading && !busy else { return }
        loading = true
        defer { loading = false }
        do {
            snapshot = try DashboardSnapshot.decode(await execute(["snapshot"], Bridge.environment(defaults: defaults)))
            if snapshotFailed { message = nil; snapshotFailed = false }
        } catch {
            snapshotFailed = true
            showError(error is BridgeError && (error as? BridgeError) == .missingBinary
                ? t("找不到内置 CCSW，请重新构建应用。", "Bundled CCSW missing. Rebuild the app.")
                : t("读取失败，保留上次数据。请检查配置路径后刷新。", "Could not read status. Previous data retained. Check paths and refresh."))
        }
    }
    func action(_ arguments: [String], quiet: Bool = false) async {
        guard !busy && !loading else { return }
        busy = true
        defer { busy = false }
        do {
            let result = try decoder().decode(ActionResult.self, from: await execute(["action"] + arguments, Bridge.environment(defaults: defaults)))
            guard result.schemaVersion == 1 else { throw BridgeError.incompatible }
            if result.ok {
                if quiet && arguments.first == "refresh-account" { message = nil; messageIsError = false }
                if !quiet {
                    messageIsError = false
                    message = result.restartRequired
                        ? t("配置已写入。请重启对应客户端；现有会话不会自动切换。", "Configuration saved. Restart the client; existing chats keep their settings.")
                        : (arguments.contains("claude") ? t("代理配置已更新，无需重启 Claude Code。", "Proxy configuration updated. No Claude Code restart needed.") : t("已更新", "Updated"))
                }
            } else {
                switch result.error {
                case "busy": showError(t("另一个操作正在执行，请稍后重试。", "Another operation is running. Try again shortly."))
                case "configuration_saved_sync_paused": showError(syncPausedMessage(result))
                case "configuration_saved_sync_failed": showError(t("配置已保存，但同步操作失败。请在代理与状态页检查服务、端口和配置后重试。", "Configuration saved, but synchronization failed. Inspect service, port and configuration in Proxy & status."))
                default: showError(t("操作失败。请检查网络、登录或代理与状态页。", "Operation failed. Check network, login or Proxy & status."))
                }
            }
            // Read after every result, including partial failure. Never optimistically mark a selection.
            snapshot = try DashboardSnapshot.decode(await execute(["snapshot"], Bridge.environment(defaults: defaults)))
        } catch { showError(t("操作未能完成，请刷新状态后重试。", "Operation did not complete. Refresh status before retrying.")) }
    }
    func refreshQuota(_ account: SubscriptionAccount, force: Bool = false) async {
        guard !busy && !loading && (force || refreshPolicy.isDue(account)) else { return }
        refreshPolicy.attempted(account.id)
        await action(["refresh-account", account.id], quiet: true)
    }
    func refreshActiveIfDue() async {
        guard let account = activeAccount, refreshPolicy.isDue(account), !loading && !busy else { return }
        // Recheck the actual account before a background query after external switches.
        await refresh()
        if let actual = activeAccount { await refreshQuota(actual) }
    }
    func refreshAll() async {
        await refresh()
        if let account = currentAccount { await refreshQuota(account, force: true) }
    }
    func refreshManagement() async {
        guard !busy && !managementLoading else { return }
        managementLoading = true
        defer { managementLoading = false }
        do {
            let data = try await execute(["management"], Bridge.environment(defaults: defaults))
            let result = try decoder().decode(ManagementSnapshot.self, from: data)
            guard result.schemaVersion == 1 else { throw BridgeError.incompatible }
            management = result
        } catch { showError(t("无法读取配置，请检查路径后重试。", "Could not read configurations. Check paths and retry.")) }
    }
    func saveProvider(_ provider: ManagedProvider) async -> Bool {
        do { return await managementAction("save-profile", data: try JSONEncoder.dashboard.encode(provider)) }
        catch { showError(t("配置格式无效。", "Invalid configuration.")); return false }
    }
    func deleteProvider(_ provider: ManagedProvider) async -> Bool {
        guard let revision = provider.revision else { return false }
        let value = ["client": provider.client, "id": provider.id, "revision": revision]
        do { return await managementAction("delete-profile", data: try JSONEncoder().encode(value)) }
        catch { return false }
    }
    func managementAction(_ action: String, data: Data, operation: String? = nil) async -> Bool {
        guard !busy && !loading else { return false }
        busy = true
        var success = false
        do {
            let data = try await write(["action", action] + (operation.map { [$0] } ?? []), Bridge.environment(defaults: defaults), data)
            let result = try decoder().decode(ActionResult.self, from: data)
            guard result.schemaVersion == 1 else { throw BridgeError.incompatible }
            if result.ok {
                success = true; messageIsError = false
                message = result.restartRequired
                    ? t("已完成。请重启 Codex 客户端并打开新会话。", "Completed. Restart Codex and open a new chat.")
                    : t("已保存，与 TUI 共享同一份配置。", "Saved to the configuration shared with the TUI.")
            } else {
                switch result.error {
                case "edit_conflict": showError(t("配置已被 TUI 或另一窗口修改。请保留草稿，关闭后重新打开最新配置再编辑。", "Changed in the TUI or another window. Keep your draft and reopen the latest configuration before editing."))
                case "invalid_input": showError(t("请检查地址、默认模型、角色引用及 Token 参数；替换认证时需填写密钥。", "Check endpoint, default model, role references and token limits. Replacing credentials requires a key."))
                case "configuration_saved_sync_paused": showError(syncPausedMessage(result)); success = true
                case "configuration_saved_sync_failed": showError(t("配置已保存，但同步操作失败。请在代理与状态页检查后重试。", "Saved, but synchronization failed. Inspect Proxy & status and retry.")); success = true
                default: showError(t("保存未完成，请检查配置是否正被其他窗口使用。", "Could not save. Check whether another window is using this configuration."))
                }
            }
        } catch { showError(t("操作失败，请重新读取实际配置。", "Operation failed. Reload the actual configuration.")) }
        busy = false
        await refresh(); await refreshManagement(); await refreshWorkspace()
        return success
    }
    func syncPausedMessage(_ result: ActionResult) -> String {
        let fields = (result.syncConflicts ?? []).joined(separator: ", ")
        return t("配置已保存。Claude 设置已被外部修改，自动同步已暂停以保留这些改动。请到“代理与状态”查看并重新接管。", "Saved. Claude settings changed externally; automatic synchronization is paused to preserve those changes. Inspect Proxy & status to reconnect.") + (fields.isEmpty ? "" : "\n" + fields)
    }
    func manage(_ operation: String, _ payload: [String: Any] = [:]) async -> Bool {
        do { return await managementAction("manage", data: try JSONSerialization.data(withJSONObject: payload), operation: operation) }
        catch { showError(t("输入无效", "Invalid input")); return false }
    }
    func refreshWorkspace() async {
        guard !busy && !workspaceLoading else { return }
        workspaceLoading = true; defer { workspaceLoading = false }
        do {
            let value = try decoder().decode(WorkspaceState.self, from: await execute(["workspace"], Bridge.environment(defaults: defaults)))
            guard value.schemaVersion == 1 else { throw BridgeError.incompatible }; workspace = value
        }
        catch { showError(t("无法读取客户端设置或状态，请检查配置文件。", "Could not read client settings or status. Check configuration files.")) }
    }
    func refreshLedger() async {
        guard !busy && !ledgerLoading else { return }
        ledgerLoading = true; defer { ledgerLoading = false }
        do {
            let value = try decoder().decode(Ledger.self, from: await execute(["usage-detail"], Bridge.environment(defaults: defaults)))
            guard value.schemaVersion == 1 else { throw BridgeError.incompatible }; ledger = value
        }
        catch { showError(t("无法读取用量账本。", "Could not read the usage ledger.")) }
    }
    func probe(_ kind: String, provider: ManagedProvider, model: String? = nil) async throws -> ProbeResult {
        let payload = try JSONEncoder.dashboard.encode(provider)
        let result = try decoder().decode(ProbeResult.self, from: await write(["probe", kind] + (model.map { ["--model", $0] } ?? []), Bridge.environment(defaults: defaults), payload))
        guard result.schemaVersion == 1 else { throw BridgeError.incompatible }; return result
    }
    func openTUI() {
        do { try Bridge.openTerminal() }
        catch { showError(t("无法打开终端。", "Could not open Terminal.")) }
    }
    func showError(_ text: String) { messageIsError = true; message = text }
}
extension BridgeError: Equatable {}
