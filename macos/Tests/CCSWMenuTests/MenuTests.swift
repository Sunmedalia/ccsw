import XCTest
@testable import CCSWMenu

final class MenuTests: XCTestCase {
    func fixture() throws -> Data { try Data(contentsOf: Bundle.module.url(forResource: "snapshot", withExtension: "json", subdirectory: "Fixtures")!) }
    func testSnapshotAndUnknownTokenSemantics() throws {
        let snapshot = try DashboardSnapshot.decode(fixture())
        XCTAssertEqual(snapshot.accounts[0].windows[0].fraction, 0.32)
        XCTAssertEqual(snapshot.clients[0].providers[0].models.count, 2)
        let total = try decoder().decode(UsageTotals.self, from: Data(#"{"calls":2,"input":0,"output":0,"unknown":2,"failed":0,"pending":0,"interrupted":0,"cache_read":0,"cache_write":0}"#.utf8))
        XCTAssertEqual(total.tokenLabel(0), "?")
        let incompatible = String(data: try fixture(), encoding: .utf8)!.replacingOccurrences(of: "\"schema_version\": 1", with: "\"schema_version\": 2")
        XCTAssertThrowsError(try DashboardSnapshot.decode(Data(incompatible.utf8)))
    }
    func testQuotaCooldownIncludesFailedAttempts() throws {
        let account = try DashboardSnapshot.decode(fixture()).accounts[0]
        var policy = RefreshPolicy()
        let date = Date(timeIntervalSince1970: account.refreshedAt! + 300)
        XCTAssertTrue(policy.isDue(account, at: date))
        policy.attempted(account.id, at: date)
        XCTAssertFalse(policy.isDue(account, at: date.addingTimeInterval(299)))
        XCTAssertTrue(policy.isDue(account, at: date.addingTimeInterval(300)))
    }
    @MainActor func testSelectionAndLanguage() async throws {
        let data = try fixture()
        let suite = "ccsw-menu-tests-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = MenuStore(defaults: defaults, execute: { _, _ in data })
        await store.refresh()
        XCTAssertEqual(store.currentAccount?.id, String(repeating: "a", count: 32))
        store.language = "zh-Hans"
        XCTAssertEqual(store.t("中文", "English"), "中文")
        store.language = "en"
        XCTAssertEqual(store.t("中文", "English"), "English")
        XCTAssertEqual(defaults.string(forKey: "language"), "en")
    }
    @MainActor func testConcurrentRefreshIsCoalescedAndFailedActionDoesNotSelect() async throws {
        let data = try fixture()
        var calls = 0
        let store = MenuStore(execute: { args, _ in
            calls += 1
            try await Task.sleep(nanoseconds: 30_000_000)
            if args.first == "action" { return Data(#"{"schema_version":1,"ok":false,"restart_required":false,"error":"busy"}"#.utf8) }
            return data
        })
        async let first: Void = store.refresh()
        async let second: Void = store.refresh()
        _ = await (first, second)
        XCTAssertEqual(calls, 1)
        await store.action(["use-account", String(repeating: "b", count: 32)])
        XCTAssertEqual(store.currentAccount?.id, String(repeating: "a", count: 32))
        XCTAssertTrue(store.messageIsError)
        XCTAssertFalse(store.busy)
    }
    func testTerminalArgumentsQuoteMetacharacters() {
        XCTAssertEqual(Bridge.shellQuote("a'b $HOME `whoami`"), "'a'\\''b $HOME `whoami`'")
    }
}

extension MenuTests {
    func managementFixture() throws -> Data {
        try Data(contentsOf: Bundle.module.url(forResource: "management", withExtension: "json", subdirectory: "Fixtures")!)
    }
    @MainActor func testManagementPreservesDraftAndUsesStdinForCredentialChanges() async throws {
        let state = try fixture(), management = try managementFixture()
        var submittedArguments: [String] = []
        var submittedData: Data?
        let store = MenuStore(execute: { args, _ in args.first == "management" ? management : state }, write: { args, _, data in
            submittedArguments = args; submittedData = data
            return Data(#"{"schema_version":1,"ok":false,"restart_required":false,"error":"edit_conflict"}"#.utf8)
        })
        await store.refreshManagement()
        var draft = try XCTUnwrap(store.management?.providers.first)
        draft.name = "Unsaved changes"
        draft.credentialChange = "replace"
        draft.secret = "private-key-in-stdin"
        let saved = await store.saveProvider(draft)
        XCTAssertFalse(saved)
        XCTAssertEqual(draft.name, "Unsaved changes")
        XCTAssertEqual(store.management?.providers.first?.name, "Studio gateway")
        XCTAssertFalse(submittedArguments.joined().contains("private-key"))
        XCTAssertTrue(String(data: try XCTUnwrap(submittedData), encoding: .utf8)!.contains("private-key-in-stdin"))
        XCTAssertTrue(store.messageIsError)
        XCTAssertFalse(store.busy)
    }
    @MainActor func testClaudeApplyDoesNotRequestRestartAndClientTabsAreIndependent() async throws {
        let state = try fixture()
        let suite = "ccsw-tab-tests-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = MenuStore(defaults: defaults, execute: { args, _ in
            if args.first == "action" { return Data(#"{"schema_version":1,"ok":true,"restart_required":false,"error":null}"#.utf8) }
            return state
        })
        store.language = "en"
        store.selectedClient = "claude"
        await store.action(["apply", "--client", "claude", "--profile", "demo", "--model", "one"])
        XCTAssertTrue(store.message?.contains("No Claude Code restart needed") == true)
        store.selectedClient = "pi"
        XCTAssertEqual(store.snapshot?.accounts.count, 2)
        XCTAssertEqual(canonicalModel("model[1m]"), "model")
    }
}

extension MenuTests {
    func workspaceFixture() throws -> Data {
        try Data(contentsOf: Bundle.module.url(forResource: "workspace", withExtension: "json", subdirectory: "Fixtures")!)
    }
    @MainActor func testSavedButPausedClosesDraftAndExplainsConflict() async throws {
        let state = try fixture(), management = try managementFixture(), workspace = try workspaceFixture()
        let store = MenuStore(execute: { args, _ in
            args.first == "workspace" ? workspace : args.first == "management" ? management : state
        }, write: { _, _, _ in
            Data(#"{"schema_version":1,"ok":false,"saved":true,"restart_required":false,"error":"configuration_saved_sync_paused","sync_conflicts":["env","model"]}"#.utf8)
        })
        store.language = "en"
        await store.refreshManagement()
        let saved = await store.saveProvider(try XCTUnwrap(store.management?.providers.first))
        XCTAssertTrue(saved)
        XCTAssertTrue(store.message?.contains("changed externally") == true)
        XCTAssertTrue(store.message?.contains("env, model") == true)
        XCTAssertNotNil(store.workspace)
        XCTAssertFalse(store.busy)
    }
    @MainActor func testUnsavedProbeUsesDraftThroughStdinWithoutSaving() async throws {
        let management = try decoder().decode(ManagementSnapshot.self, from: managementFixture())
        var draft = try XCTUnwrap(management.providers.first)
        draft.revision = nil; draft.id = ""; draft.secret = "draft-key"; draft.credentialChange = "replace"
        var arguments = [String](), payload: Data?
        let store = MenuStore(write: { args, _, data in
            arguments = args; payload = data
            return Data(#"{"schema_version":1,"ok":true,"data":{"models":[{"id":"new-model"}]},"error":null}"#.utf8)
        })
        let result = try await store.probe("models", provider: draft)
        XCTAssertEqual(arguments, ["probe", "models"])
        XCTAssertEqual(result.data?.models?.first?.id, "new-model")
        XCTAssertTrue(String(data: try XCTUnwrap(payload), encoding: .utf8)!.contains("draft-key"))
        XCTAssertNil(store.management)
        XCTAssertFalse(store.busy)
    }
}
