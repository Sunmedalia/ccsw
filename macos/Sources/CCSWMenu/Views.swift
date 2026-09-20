import SwiftUI
import AppKit
import Charts
import ServiceManagement

let sea = Color(red: 0.29, green: 0.63, blue: 0.68)
let sky = Color(red: 0.48, green: 0.62, blue: 0.83)

struct GlassBackground: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .hudWindow; view.blendingMode = .behindWindow; view.state = .active
        return view
    }
    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}
struct MenuPanel: View {
    @ObservedObject var store: MenuStore
    let openSettings: () -> Void
    var openManagement: () -> Void = {}
    @State private var expandedAccounts = false
    @State private var expandedAccount: String?
    @State private var days = 1
    @State private var codexContent = "subscription"
    private var client: String { ["claude": "Claude", "codex": "Codex", "pi": "Pi"][store.selectedClient] ?? "Claude" }
    @State private var chartTokens = false
    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().opacity(0.55)
            ScrollView {
                VStack(alignment: .leading, spacing: 9) {
                    if let text = store.message {
                        HStack(alignment: .top) {
                            Image(systemName: store.messageIsError ? "exclamationmark.circle" : "checkmark.circle").foregroundStyle(store.messageIsError ? Color.orange : sea)
                            Text(text).font(.caption).fixedSize(horizontal: false, vertical: true)
                            Spacer(minLength: 0)
                            Button { store.message = nil } label: { Image(systemName: "xmark").font(.caption2) }.buttonStyle(.plain)
                                .accessibilityLabel(store.t("关闭提示", "Dismiss message"))
                        }.padding(10).background(.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 9))
                    }
                    if let snapshot = store.snapshot {
                        currentConfiguration(snapshot)
                        Divider().opacity(0.4)
                        if store.selectedClient == "codex" {
                            Picker("", selection: $codexContent) {
                                Text(store.t("ChatGPT 订阅", "ChatGPT plan")).tag("subscription")
                                Text(store.t("API 用量", "API usage")).tag("api")
                            }.pickerStyle(.segmented).labelsHidden().controlSize(.small)
                            if codexContent == "subscription" { subscription(snapshot) }
                            else { usage(snapshot) }
                        } else { usage(snapshot) }
                        if !snapshot.errors.isEmpty {
                            Label(store.t("部分数据不可用：", "Some data unavailable: ") + snapshot.errors.map(\.section).joined(separator: ", "), systemImage: "exclamationmark.triangle")
                                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        }
                    } else {
                        VStack(spacing: 12) {
                            Image(systemName: "square.stack.3d.up").font(.largeTitle).foregroundStyle(sea)
                            Text(store.t("正在读取你的配置", "Reading your configuration")).font(.headline)
                            Text(store.t("订阅、用量和模型，一处查看。", "Subscriptions, usage and models in one place.")).font(.caption).foregroundStyle(.secondary)
                            if store.loading { ProgressView().controlSize(.small) }
                        }.frame(maxWidth: .infinity).padding(.vertical, 50)
                    }
                }.padding(14)
            }
            Divider().opacity(0.55)
            footer
        }
        .frame(width: 320)
        .background(GlassBackground())
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.white.opacity(0.14), lineWidth: 0.5).allowsHitTesting(false))
        .tint(sea)
        .environment(\.locale, Locale(identifier: store.isChinese ? "zh-Hans" : "en"))
    }
    private var header: some View {
        Picker(store.t("客户端", "Client"), selection: $store.selectedClient) {
            Text("Claude Code").tag("claude")
            Text("Codex").tag("codex")
            Text("Pi").tag("pi")
        }.pickerStyle(.segmented).labelsHidden().padding(.horizontal, 12).padding(.vertical, 12)
            .onChange(of: store.selectedClient) { client in
                if client == "codex" {
                    codexContent = store.snapshot?.clients.first(where: { $0.id == "codex" })?.provider == "openai" ? "subscription" : "api"
                }
            }
    }
    private func sectionTitle(_ zh: String, _ en: String, trailing: String? = nil) -> some View {
        HStack {
            Text(store.t(zh, en)).font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
            Spacer()
            if let trailing { Text(trailing).font(.system(size: 10, design: .monospaced)).foregroundStyle(.tertiary) }
        }
    }
    @ViewBuilder private func subscription(_ snapshot: DashboardSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            sectionTitle("订阅额度", "SUBSCRIPTION", trailing: "CODEX")
            if let account = store.currentAccount {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(account.title).font(.system(size: 16, weight: .semibold)).lineLimit(1).help(account.title)
                        Text(account.email).font(.caption).foregroundStyle(.secondary).lineLimit(1).help(account.email)
                    }
                    Spacer(minLength: 6)
                    Text(account.plan?.capitalized ?? store.t("未知套餐", "Unknown plan"))
                        .font(.system(size: 10, weight: .medium)).padding(.horizontal, 8).padding(.vertical, 4)
                        .background(sea.opacity(0.12), in: Capsule())
                }
                accountMarkers(account)
                quota(account)
                DisclosureGroup(isExpanded: $expandedAccounts) {
                    VStack(alignment: .leading, spacing: 9) {
                        ForEach(snapshot.accounts) { a in
                            DisclosureGroup(isExpanded: Binding(get: { expandedAccount == a.id }, set: { value in
                                expandedAccount = value ? a.id : nil
                                if value { Task { await store.refreshQuota(a) } }
                            })) {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text(a.email).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                                    Text(a.workspace).font(.caption2).foregroundStyle(.tertiary).lineLimit(1).help(a.workspace)
                                    accountMarkers(a)
                                    quota(a)
                                    HStack {
                                        Button(store.t("使用此账号", "Use account")) { Task { await store.action(["use-account", a.id]) } }
                                        Spacer()
                                        Button(store.t("刷新", "Refresh")) { Task { await store.refreshQuota(a, force: true) } }
                                    }.controlSize(.small).disabled(store.busy || store.loading)
                                }.padding(.vertical, 8)
                            } label: {
                                HStack {
                                    Text(a.title).lineLimit(1)
                                    Spacer()
                                    Text(a.plan?.capitalized ?? "—").font(.caption2).foregroundStyle(.secondary)
                                    if a.selected { Image(systemName: "checkmark.circle.fill").foregroundStyle(sea) }
                                }.font(.caption)
                            }
                        }
                    }.padding(.top, 10)
                } label: { Text(store.t("所有账号", "All accounts") + " · \(snapshot.accounts.count)").font(.caption).foregroundStyle(.secondary) }
            } else {
                Text(store.t("尚未保存 Codex 订阅账号", "No saved Codex subscriptions")).font(.subheadline)
                Text(store.t("在 GUI 的订阅账号页登录或导入。", "Sign in or import in the GUI subscriptions page."))
                    .font(.caption).foregroundStyle(.secondary)
                Button(store.t("管理账号", "Manage accounts"), action: openManagement).controlSize(.small)
            }
        }
    }
    private func accountMarkers(_ account: SubscriptionAccount) -> some View {
        HStack(spacing: 8) {
            if account.selected { Label(store.t("已选择", "Selected"), systemImage: "checkmark.circle").foregroundStyle(sea) }
            if account.localLogin { Label(store.t("本机登录", "Local login"), systemImage: "person.crop.circle").foregroundStyle(.secondary) }
            if !account.selected && !account.localLogin { Text(store.t("已保存账号", "Saved account")).foregroundStyle(.secondary) }
        }.font(.system(size: 10))
    }
    private func quota(_ account: SubscriptionAccount) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if account.windows.isEmpty {
                Text(store.t("额度未知 · 点击刷新查询", "Quota unknown · refresh to check")).font(.caption).foregroundStyle(.secondary)
            }
            ForEach(account.windows) { window in
                VStack(spacing: 5) {
                    HStack {
                        Text(windowTitle(window)).font(.system(size: 11, weight: .medium))
                        Spacer()
                        Text(window.usedPercent.map { String(format: "%.0f%%", $0) } ?? "—")
                            .font(.system(size: 12, weight: .medium, design: .monospaced))
                        Text(store.t("已用", "used")).font(.system(size: 10)).foregroundStyle(.secondary)
                    }
                    GeometryReader { geometry in
                        ZStack(alignment: .leading) {
                            Capsule().fill(.primary.opacity(0.055))
                            Capsule().fill(window.fraction >= 0.9 ? Color.orange.opacity(0.7) : (window.window == "primary" ? sea : sky))
                                .frame(width: geometry.size.width * window.fraction)
                        }
                    }.frame(height: 5).accessibilityLabel(windowTitle(window)).accessibilityValue(window.usedPercent.map { "\($0)%" } ?? store.t("未知", "Unknown"))
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        Text(resetLabel(window.resetsAt, now: context.date)).font(.system(size: 10)).foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
            }
            TimelineView(.periodic(from: .now, by: 30)) { context in
                HStack(spacing: 4) {
                    if account.refreshFailed { Image(systemName: "exclamationmark.circle").foregroundStyle(.orange) }
                    Text(freshness(account, now: context.date))
                }.font(.system(size: 10)).foregroundStyle(.secondary)
            }
        }
    }
    private func windowTitle(_ w: QuotaWindow) -> String {
        let duration: String
        if let minutes = w.durationMinutes, minutes > 0 {
            if minutes % 1440 == 0 { duration = "\(minutes / 1440)" + store.t(" 天", " days") }
            else if minutes % 60 == 0 { duration = "\(minutes / 60)" + store.t(" 小时", " hours") }
            else { duration = "\(minutes)" + store.t(" 分钟", " min") }
        } else { duration = w.window == "primary" ? store.t("主要窗口", "Primary window") : store.t("次要窗口", "Secondary window") }
        return (w.bucket == "codex" ? "" : w.bucket + " · ") + duration
    }
    private func resetLabel(_ timestamp: TimeInterval?, now: Date) -> String {
        guard let timestamp else { return store.t("重置时间未知", "Reset time unknown") }
        let seconds = Int(timestamp - now.timeIntervalSince1970)
        guard seconds > 0 else { return store.t("已到重置时间 · 等待刷新", "Reset due · awaiting refresh") }
        let hours = seconds / 3600, minutes = (seconds % 3600) / 60
        let value = hours >= 24 ? "\(hours / 24)d \(hours % 24)h" : "\(hours)h \(minutes)m"
        return store.t("距重置 ", "Resets in ") + value
    }
    private func freshness(_ a: SubscriptionAccount, now: Date) -> String {
        guard let refreshed = a.refreshedAt else { return store.t("尚未刷新", "Not refreshed yet") + (a.refreshFailed ? store.t(" · 查询失败", " · query failed") : "") }
        let age = max(0, Int(now.timeIntervalSince1970 - refreshed) / 60)
        let time = age == 0 ? store.t("刚刚更新", "Updated just now") : store.t("\(age) 分钟前更新", "Updated \(age)m ago")
        return time + (a.refreshFailed ? store.t(" · 刷新失败，显示缓存", " · refresh failed, cached") : age >= 5 ? store.t(" · 缓存", " · cached") : "")
    }
    @ViewBuilder private func usage(_ snapshot: DashboardSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionTitle("代理用量", "PROXY USAGE")
            HStack {
                Picker("", selection: $days) {
                    Text(store.t("今日", "Today")).tag(1)
                    Text(store.t("7 天", "7 days")).tag(7)
                    Text(store.t("30 天", "30 days")).tag(30)
                }.pickerStyle(.segmented).labelsHidden()

            }
            if client == "Pi" {
                Text(store.t("Pi 直连尚未接入统计", "Pi direct requests are not metered")).font(.caption).foregroundStyle(.secondary)
            } else if let usage = snapshot.usage, usage.available,
                      let range = usage.ranges.first(where: { $0.days == days && $0.client == client }) {
                HStack(spacing: 0) {
                    metric(store.t("调用", "Calls"), compact(range.totals.calls))
                    metric(store.t("输入 Token", "Input tokens"), range.totals.tokenLabel(range.totals.input))
                    metric(store.t("输出 Token", "Output tokens"), range.totals.tokenLabel(range.totals.output))
                }
                HStack {
                    Text(store.t("趋势", "Trend")).font(.caption2).foregroundStyle(.secondary)
                    Spacer()
                    Picker("", selection: $chartTokens) { Text(store.t("调用", "Calls")).tag(false); Text("Tokens").tag(true) }
                        .labelsHidden().pickerStyle(.segmented).frame(width: 125).controlSize(.mini)
                }
                Chart(range.points) { point in
                    BarMark(x: .value("Time", point.label), y: .value("Usage", chartTokens ? point.totals.input + point.totals.output : point.totals.calls))
                        .foregroundStyle(sea.opacity(0.7)).cornerRadius(2)
                        .accessibilityLabel(point.label)
                        .accessibilityValue(chartTokens ? point.totals.tokenLabel(point.totals.input + point.totals.output) : "\(point.totals.calls)")
                }.chartXAxis(.hidden).chartYAxis(.hidden).frame(height: 36)
                HStack {
                    Text(range.points.first?.label ?? "")
                    Spacer()
                    Text(range.points.last?.label ?? "")
                }.font(.system(size: 9, design: .monospaced)).foregroundStyle(.tertiary)
                if range.totals.unknown > 0 {
                    Text(store.t("部分请求缺少 Token 数据，图表仅显示已知值。", "Some requests have unknown token usage; chart shows known values."))
                        .font(.caption2).foregroundStyle(.secondary)
                }
                if range.totals.failed + range.totals.interrupted + range.totals.pending > 0 {
                    Text(store.t("失败 \(range.totals.failed) · 中断 \(range.totals.interrupted) · 进行中 \(range.totals.pending)", "Failed \(range.totals.failed) · interrupted \(range.totals.interrupted) · pending \(range.totals.pending)"))
                        .font(.caption2).foregroundStyle(.secondary)
                }
                Text(store.t("账本时区 ", "Ledger timezone ") + offsetLabel(usage.offsetSeconds)).font(.system(size: 9)).foregroundStyle(.tertiary)
            } else {
                Text(snapshot.usage == nil ? store.t("用量暂时不可用", "Usage unavailable") : store.t("尚无代理用量记录", "No proxy usage recorded yet"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Text(store.t("仅统计经过 CCSW 的请求；不含 ChatGPT 订阅与 Pi 直连。", "Only requests through CCSW; excludes ChatGPT subscriptions and direct Pi traffic."))
                .font(.system(size: 10)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
    }
    private func offsetLabel(_ seconds: Int) -> String { String(format: "UTC%@%02d:%02d", seconds >= 0 ? "+" : "−", abs(seconds) / 3600, abs(seconds) % 3600 / 60) }
    private func metric(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(value).font(.system(size: 20, weight: .medium, design: .rounded)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.65)
            Text(title).font(.system(size: 10)).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
    @ViewBuilder private func currentConfiguration(_ snapshot: DashboardSnapshot) -> some View {
        if let config = snapshot.clients.first(where: { $0.id == store.selectedClient }) {
            VStack(alignment: .leading, spacing: 7) {
                HStack {
                    Text(store.t("当前配置", "Current configuration")).font(.system(size: 10)).foregroundStyle(.secondary)
                    Spacer()
                    Text(statusLabel(config.status)).font(.system(size: 9)).foregroundStyle(config.status == "conflict" ? Color.orange : .secondary)
                }
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(config.id == "codex" && config.provider == "openai" ? (store.currentAccount?.title ?? "ChatGPT") : (config.providerName ?? store.t("选择厂商", "Choose provider")))
                            .font(.system(size: 13, weight: .semibold)).lineLimit(1)
                        Text(config.model ?? store.t("选择模型", "Choose model"))
                            .font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary).lineLimit(1)
                            .help(config.model ?? "")
                    }
                    Spacer(minLength: 8)
                    Menu {
                        if config.id == "codex" && !snapshot.accounts.isEmpty {
                            Section(store.t("ChatGPT 账号", "ChatGPT accounts")) {
                                ForEach(snapshot.accounts) { a in Button(a.title) { Task { await store.action(["use-account", a.id]) } } }
                            }
                        }
                        ForEach(config.providers.filter { $0.enabled && !$0.models.isEmpty }) { provider in
                            Section(provider.name) {
                                ForEach(provider.models) { model in
                                    Button(model.name) { Task { await store.action(["apply", "--client", config.id, "--profile", provider.id, "--model", model.id]) } }
                                }
                            }
                        }
                        Divider()
                        Button(store.t("管理配置…", "Manage configurations…"), action: openManagement)
                    } label: { Image(systemName: "chevron.up.chevron.down").foregroundStyle(sea) }
                        .menuStyle(.borderlessButton).menuIndicator(.hidden).frame(width: 24)
                        .disabled(store.busy || store.loading)
                        .accessibilityLabel(store.t("切换厂商与模型", "Switch provider and model"))
                }.padding(10).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 9))
                if config.id == "claude" {
                    Text(store.t("通过代理切换，无需重启 Claude Code", "Switch through the proxy · no Claude Code restart"))
                        .font(.system(size: 9)).foregroundStyle(.secondary)
                }
            }
        }
    }
    private func statusLabel(_ status: String) -> String {
        switch status {
        case "synced": return store.t("已同步", "Synced")
        case "pending": return store.t("待同步", "Pending")
        case "conflict": return store.t("外部变更", "Changed externally")
        case "native": return store.t("原生配置", "Native")
        case "unavailable": return store.t("不可用", "Unavailable")
        default: return store.t("未接管", "Unmanaged")
        }
    }
    private var footer: some View {
        HStack(spacing: 11) {
            Menu {
                if let proxy = store.snapshot?.proxy {
                    Text(proxy.listen)
                    Text(store.t("\(proxy.routes) 条路由", "\(proxy.routes) routes"))
                    Button(proxy.running ? store.t("停止代理", "Stop proxy") : store.t("启动代理", "Start proxy")) {
                        Task { await store.action([proxy.running ? "proxy-stop" : "proxy-start"]) }
                    }.disabled(store.busy || store.loading)
                } else { Text(store.t("代理状态未知", "Proxy status unknown")) }
            } label: {
                HStack(spacing: 5) {
                    Circle().fill(store.snapshot?.proxy?.running == true ? sea : .gray.opacity(0.45)).frame(width: 5, height: 5)
                    Text(store.t("代理", "Proxy")).font(.system(size: 10))
                }
            }.menuStyle(.borderlessButton).fixedSize()
            Spacer(minLength: 0)
            if store.busy || store.loading { ProgressView().controlSize(.mini).scaleEffect(0.7) }
            Button { Task { if store.selectedClient == "codex" && codexContent == "subscription" { await store.refreshAll() } else { await store.refresh() } } } label: { Image(systemName: "arrow.clockwise") }
                .disabled(store.busy || store.loading).help(store.t("刷新", "Refresh")).accessibilityLabel(store.t("刷新", "Refresh"))
            Button(action: openManagement) { Image(systemName: "slider.horizontal.3") }.help(store.t("管理配置", "Manage configurations")).accessibilityLabel(store.t("管理配置", "Manage configurations"))
            Button(action: openSettings) { Image(systemName: "gearshape") }.help(store.t("设置", "Settings")).accessibilityLabel(store.t("设置", "Settings"))
            Button { NSApp.terminate(nil) } label: { Image(systemName: "power") }.help(store.t("退出菜单栏应用", "Quit menu app")).accessibilityLabel(store.t("退出菜单栏应用", "Quit menu app"))
        }.buttonStyle(.plain).foregroundStyle(.secondary).padding(.horizontal, 14).padding(.vertical, 10)
    }
}
extension String { var nilIfEmpty: String? { isEmpty ? nil : self } }

struct SettingsPanel: View {
    @ObservedObject var store: MenuStore
    @State private var paths: [String: String] = [:]
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
    @State private var feedback: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(store.t("让配置随手可见", "Your configuration, close at hand")).font(.title2.weight(.semibold))
            Form {
                Picker(store.t("语言", "Language"), selection: $store.language) {
                    Text(store.t("跟随系统", "System")).tag("system")
                    Text("简体中文").tag("zh-Hans"); Text("English").tag("en")
                }
                Toggle(store.t("登录时启动", "Launch at login"), isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { enabled in
                        do {
                            if enabled { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
                            if SMAppService.mainApp.status == .requiresApproval {
                                feedback = store.t("请在系统设置的登录项中允许 CCSW Menu。", "Allow CCSW Menu in System Settings → Login Items.")
                            }
                        } catch {
                            feedback = store.t("无法修改登录项，请将应用移至应用程序目录后重试。", "Could not change login item. Move the app to Applications and retry.")
                            launchAtLogin = SMAppService.mainApp.status == .enabled
                        }
                    }
            }
            Divider()
            Text(store.t("配置路径", "Configuration paths")).font(.headline)
            Text(store.t("留空沿用 CCSW 默认路径。目录覆盖与终端环境独立；支持 ~/。", "Leave blank for CCSW defaults. Overrides are independent of your terminal; ~/ is supported."))
                .font(.caption).foregroundStyle(.secondary)
            ForEach(Bridge.pathKeys, id: \.self) { key in
                HStack {
                    Text(pathTitle(key)).font(.caption).frame(width: 115, alignment: .leading).help(key)
                    TextField(key, text: Binding(get: { paths[key] ?? "" }, set: { paths[key] = $0 }))
                        .textFieldStyle(.roundedBorder).font(.system(size: 11, design: .monospaced))
                }
            }
            Text(store.t("状态与缓存路径填写 XDG 父目录，CCSW 会追加 /ccsw。Codex 程序路径可指向本机 codex 可执行文件。", "State and cache paths are XDG parent directories; CCSW appends /ccsw. Codex executable may point to your local codex binary."))
                .font(.caption2).foregroundStyle(.secondary)
            if let path = store.snapshot?.configPath { Text(path).font(.caption2).foregroundStyle(.secondary).textSelection(.enabled) }
            if let feedback { Text(feedback).font(.caption).foregroundStyle(.secondary) }
            Spacer(minLength: 0)
            HStack {
                Text("CCSW Menu · 0.3.0").font(.caption2).foregroundStyle(.tertiary)
                Spacer()
                Button(store.t("保存路径", "Save paths")) {
                    let expanded = paths.values.filter { !$0.isEmpty }.map { NSString(string: $0).expandingTildeInPath }
                    guard expanded.allSatisfy({ $0.hasPrefix("/") && !$0.contains("\n") }) else {
                        feedback = store.t("请输入绝对路径或 ~/ 路径。", "Use absolute paths or ~/ paths."); return
                    }
                    for key in Bridge.pathKeys { UserDefaults.standard.set(paths[key] ?? "", forKey: key) }
                    feedback = store.t("已保存", "Saved")
                    Task { await store.refresh() }
                }.keyboardShortcut(.defaultAction).disabled(store.busy || store.loading)
            }
        }.padding(24).frame(width: 540, height: 640).tint(sea)
            .onAppear { for key in Bridge.pathKeys { paths[key] = UserDefaults.standard.string(forKey: key) ?? "" } }
    }
    private func pathTitle(_ key: String) -> String {
        switch key {
        case "CCSW_CONFIG": return store.t("CCSW 配置文件", "CCSW config file")
        case "XDG_STATE_HOME": return store.t("状态父目录", "State parent")
        case "XDG_CACHE_HOME": return store.t("缓存父目录", "Cache parent")
        case "CLAUDE_CONFIG_DIR": return store.t("Claude 目录", "Claude directory")
        case "CODEX_HOME": return store.t("Codex 目录", "Codex directory")
        case "PI_CODING_AGENT_DIR": return store.t("Pi 目录", "Pi directory")
        default: return store.t("Codex 程序", "Codex executable")
        }
    }
}
