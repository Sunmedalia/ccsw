import SwiftUI
import AppKit
import Charts

struct AccountsPanel: View {
    @ObservedObject var store: MenuStore
    @StateObject private var login = LoginSession()
    @State private var selected: String?
    @State private var name = ""
    @State private var confirmRemove = false
    private var account: SubscriptionAccount? { store.snapshot?.accounts.first { $0.id == selected } }
    var body: some View {
        HSplitView {
            ScrollView {
                VStack(spacing: 5) {
                    ForEach(store.snapshot?.accounts ?? []) { a in
                        Button { selected = a.id; name = a.name } label: {
                            VStack(alignment: .leading, spacing: 5) {
                                HStack { Text(a.title).lineLimit(1); Spacer(); if a.selected { Image(systemName: "checkmark.circle.fill").foregroundStyle(sea) } }
                                Text(a.plan?.capitalized ?? "ChatGPT").font(.caption).foregroundStyle(.secondary)
                            }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
                                .background(selected == a.id ? sea.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 8)).contentShape(Rectangle())
                        }.buttonStyle(.plain)
                    }
                }.padding(10)
            }.frame(minWidth: 190, idealWidth: 230, maxWidth: 280)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text(store.t("ChatGPT 订阅账号", "ChatGPT subscriptions")).font(.title2.weight(.semibold))
                    TextField(store.t("账号名称／备注", "Account name / label"), text: $name).textFieldStyle(.roundedBorder)
                    HStack {
                        Button(store.t("浏览器登录", "Browser login")) { login.start(name: name, device: false) }
                        Button(store.t("设备码登录", "Device login")) { login.start(name: name, device: true) }
                        Menu(store.t("导入", "Import")) {
                            Button(store.t("导入本机登录", "Import local login")) { Task { _ = await store.manage("account-import", ["name": name]) } }
                            Button(store.t("从 auth.json 导入…", "Import auth.json…")) { importFile() }
                        }
                    }.disabled(login.running || store.busy)
                    if login.running {
                        HStack { ProgressView().controlSize(.small); Text(store.t("等待登录完成", "Waiting for sign-in")); Spacer(); Button(store.t("取消登录", "Cancel login")) { login.cancel() } }
                    }
                    if !login.progress.isEmpty {
                        Text(loginMessage).font(.caption).textSelection(.enabled).padding(10).frame(maxWidth: .infinity, alignment: .leading).background(.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))
                    }
                    Divider()
                    if let account {
                        HStack(alignment: .top) {
                            VStack(alignment: .leading, spacing: 5) {
                                Text(account.title).font(.headline); Text(account.email).font(.caption).foregroundStyle(.secondary)
                                Text(account.workspace).font(.caption2).foregroundStyle(.secondary)
                            }
                            Spacer(); Text(account.plan?.capitalized ?? "—").foregroundStyle(sea)
                        }
                        HStack {
                            if account.selected { Label(store.t("已应用", "Applied"), systemImage: "checkmark.circle") }
                            if account.localLogin { Label(store.t("本机登录", "Local login"), systemImage: "person.crop.circle") }
                        }.font(.caption).foregroundStyle(.secondary)
                        ForEach(account.windows) { w in
                            VStack(alignment: .leading, spacing: 5) {
                                HStack { Text(w.bucket + " · " + (w.durationMinutes.map { "\($0) min" } ?? w.window)); Spacer(); Text(w.usedPercent.map { String(format:"%.0f%%",$0) } ?? "?") }
                                ProgressView(value: w.fraction).tint(sea)
                                if let reset = w.resetsAt { Text(store.t("重置时间：", "Resets: ") + Date(timeIntervalSince1970: reset).formatted()).font(.caption2).foregroundStyle(.secondary) }
                            }
                        }
                        if account.windows.isEmpty { Text(store.t("尚无额度缓存，点击刷新查询。", "No quota cache. Refresh to query." )).foregroundStyle(.secondary) }
                        if let date = account.refreshedAt { Text(store.t("最近刷新：", "Last refresh: ") + Date(timeIntervalSince1970: date).formatted()).font(.caption) }
                        if account.refreshFailed { Text(store.t("刷新失败，保留旧缓存。", "Refresh failed; cached values retained.")).foregroundStyle(.orange) }
                        HStack {
                            Button(store.t("应用账号", "Apply account")) { Task { await store.action(["use-account", account.id]) } }
                            Button(store.t("刷新额度", "Refresh quota")) { Task { await store.refreshQuota(account, force: true) } }
                            Button(store.t("重命名", "Rename")) { Task { _ = await store.manage("account-rename", ["id": account.id, "name": name]) } }.disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                            Spacer()
                            Button(store.t("删除", "Remove"), role: .destructive) { confirmRemove = true }
                        }.disabled(store.busy || login.running)
                    } else { Text(store.t("选择已有账号，或登录／导入新账号。", "Select an account, sign in or import a new one.")).foregroundStyle(.secondary) }
                    Text(store.t("登录使用隔离目录，不覆盖本机登录；点击应用后才切换。", "Sign-in uses an isolated directory. Local login changes only when you apply an account.")).font(.caption).foregroundStyle(.secondary)
                }.padding(24)
            }.frame(maxWidth: .infinity)
        }.onChange(of: login.running) { running in if !running { Task { await store.refresh() } } }
            .onDisappear { login.cancel() }
            .alert(store.t("删除已保存账号？", "Remove saved account?"), isPresented: $confirmRemove) {
                Button(store.t("取消", "Cancel"), role: .cancel) {}
                Button(store.t("删除", "Remove"), role: .destructive) { if let account { Task { _ = await store.manage("account-remove", ["id":account.id]); selected = nil } } }
            } message: { Text(store.t("只删除 CCSW 保存的账号快照，不注销远程订阅。", "Removes the CCSW account snapshot, not the remote subscription.")) }
    }
    private var loginMessage: String {
        switch login.progress {
        case "login_complete": return store.t("登录成功，账号已保存。请选择并应用。", "Signed in and saved. Select and apply the account.")
        case "login_failed": return store.t("登录失败，请检查 Codex 程序路径、网络并重试。", "Login failed. Check the Codex executable, network and retry.")
        case "cancelled": return store.t("已取消登录", "Login cancelled")
        default: return login.progress
        }
    }
    private func importFile() {
        let picker = NSOpenPanel(); picker.canChooseDirectories = false; picker.allowsMultipleSelection = false
        picker.begin { response in
            if response == .OK, let path = picker.url?.path { Task { _ = await store.manage("account-import", ["name": name, "file": path]) } }
        }
    }
}

struct ClientSettingsPanel: View {
    @ObservedObject var store: MenuStore
    @State private var preferences = ClientPreferences(claude: ClaudePreferences(env: [:]), reasoning: nil)
    @State private var revision = ""
    @State private var original = ClientPreferences(claude: ClaudePreferences(env: [:]), reasoning: nil)
    @State private var newKey = ""
    @State private var newValue = ""
    @State private var reveal = false
    @State private var confirmDisconnect: String?
    private let presetKeys = ["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "ENABLE_TOOL_SEARCH", "CLAUDE_CODE_EFFORT_LEVEL", "DISABLE_AUTOUPDATER", "CLAUDE_CODE_DISABLE_ARTIFACT"]
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text(store.t("客户端设置", "Client settings")).font(.title2.weight(.semibold))
                    Spacer()
                    Button(store.t("重新读取／放弃草稿", "Reload / discard draft")) { Task { await store.refreshWorkspace(); load() } }
                    Button(store.t("保存", "Save")) { save() }.disabled(revision.isEmpty || store.busy || preferences == original)
                }
                Text(store.t("Claude 偏好设置", "Claude preferences")).font(.headline)
                Form {
                    Picker(store.t("隐藏 AI 署名", "Hide AI attribution"), selection: Binding(get: { preferences.claude.hideAttribution.map { $0 ? "on" : "off" } ?? "" }, set: { preferences.claude.hideAttribution = $0.isEmpty ? nil : $0 == "on" })) {
                        Text(store.t("继承", "Inherit")).tag(""); Text(store.t("隐藏", "Hide")).tag("on"); Text(store.t("显示", "Show")).tag("off")
                    }
                    preset("Teammates", "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", ["1","0"])
                    preset("Tool Search", "ENABLE_TOOL_SEARCH", ["true","false","auto"])
                    preset(store.t("思考强度", "Effort"), "CLAUDE_CODE_EFFORT_LEVEL", ["auto","low","medium","high","xhigh","max"])
                    preset(store.t("禁用自动升级", "Disable updates"), "DISABLE_AUTOUPDATER", ["1","0"])
                    preset(store.t("禁用 Artifact", "Disable Artifact"), "CLAUDE_CODE_DISABLE_ARTIFACT", ["1","0"])
                }
                Button(store.t("填入预设", "Fill presets")) {
                    preferences.claude.hideAttribution = true
                    preferences.claude.env.merge(["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS":"1","ENABLE_TOOL_SEARCH":"true","CLAUDE_CODE_EFFORT_LEVEL":"max","DISABLE_AUTOUPDATER":"1","CLAUDE_CODE_DISABLE_ARTIFACT":"1"]) { _,new in new }
                }
                Divider()
                HStack { Text(store.t("自定义环境变量", "Custom environment variables")).font(.headline); Spacer(); Toggle(store.t("显示值", "Show values"), isOn: $reveal).toggleStyle(.checkbox) }
                ForEach(preferences.claude.env.keys.filter { !presetKeys.contains($0) }.sorted(), id: \.self) { key in
                    HStack {
                        Text(key).font(.system(.caption, design: .monospaced)).frame(width: 260, alignment: .leading)
                        if reveal { TextField("Value", text: env(key)) } else { SecureField("Value", text: env(key)) }
                        Button(role: .destructive) { preferences.claude.env.removeValue(forKey: key) } label: { Image(systemName: "trash") }
                    }
                }
                HStack {
                    TextField(store.t("变量名", "Variable"), text: $newKey)
                    SecureField(store.t("值", "Value"), text: $newValue)
                    Button(store.t("添加", "Add")) {
                        preferences.claude.env[newKey] = newValue; newKey = ""; newValue = ""
                    }.disabled(newKey.isEmpty || preferences.claude.env[newKey] != nil)
                }.textFieldStyle(.roundedBorder)
                Text(store.t("路由、认证和模型变量请在厂商页面设置；自定义值按原样保存，不执行 shell。", "Configure routing, credentials and models in Providers. Custom values are literal; no shell execution.")).font(.caption).foregroundStyle(.secondary)
                Divider()
                Text(store.t("Codex 推理强度", "Codex reasoning")).font(.headline)
                Picker(store.t("API 推理强度", "API reasoning"), selection: Binding(get: { preferences.reasoning ?? "" }, set: { preferences.reasoning = $0.isEmpty ? nil : $0 })) {
                    Text(store.t("继承", "Inherit")).tag("")
                    ForEach(["none","minimal","low","medium","high","xhigh"],id: \.self) { Text($0).tag($0) }
                }
                Text(store.t("Codex 保存后在厂商页点击“使用”应用。Claude 启动时读取的环境偏好可能需要重启；代理模型切换无需重启。", "Apply Codex changes with Use in Providers. Claude startup environment preferences may require a restart; proxy model switching does not.")).font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button(store.t("断开 Claude 管理", "Disconnect Claude"), role: .destructive) { confirmDisconnect = "claude" }
                    Button(store.t("断开 Codex 管理", "Disconnect Codex"), role: .destructive) { confirmDisconnect = "codex" }
                }.disabled(store.busy || preferences != original)
            }.padding(24).disabled(store.busy)
        }.task { await store.refreshWorkspace(); load() }
            .onChange(of: preferences) { value in store.preferencesDirty = value != original }
            .onDisappear { store.preferencesDirty = false }
            .alert(store.t("断开管理并恢复原设置？", "Disconnect and restore previous settings?"), isPresented: Binding(get: { confirmDisconnect != nil },set: {if !$0 {confirmDisconnect=nil}})) {
                Button(store.t("取消", "Cancel"),role:.cancel) {confirmDisconnect=nil}
                Button(store.t("断开", "Disconnect"),role:.destructive) { if let client=confirmDisconnect {Task {_ = await store.manage("disconnect",["client":client]);confirmDisconnect=nil}} }
            } message: {Text(store.t("仅恢复仍由 CCSW 管理的字段，保留外部修改。", "Restores fields still owned by CCSW and preserves external changes."))}
    }
    private func load() { if let state = store.workspace { preferences = state.preferences; original = preferences; revision = state.preferencesRevision; store.preferencesDirty = false } }
    private func env(_ key: String) -> Binding<String> { Binding(get:{preferences.claude.env[key] ?? ""},set:{preferences.claude.env[key]=$0}) }
    private func preset(_ title: String,_ key: String,_ values: [String]) -> some View {
        Picker(title,selection:Binding(get:{preferences.claude.env[key] ?? ""},set:{preferences.claude.env[key]=$0.isEmpty ? nil : $0})) {
            Text(store.t("继承", "Inherit")).tag("")
            ForEach(values,id: \.self){Text($0).tag($0)}
            if let current=preferences.claude.env[key], !values.contains(current){Text(current).tag(current)}
        }
    }
    private func save() {
        guard let data = try? JSONEncoder.dashboard.encode(preferences), var payload = (try? JSONSerialization.jsonObject(with:data)) as? [String:Any] else {return}
        payload["revision"] = revision
        Task { if await store.manage("preferences",payload) {load()} }
    }
}

struct ProxyPanel: View {
    @ObservedObject var store: MenuStore
    @State private var port = ""
    @State private var importID = "imported"
    @State private var reconnect = false
    var body: some View {
        ScrollView {
            VStack(alignment:.leading,spacing:18) {
                HStack {Text(store.t("代理与状态", "Proxy & status")).font(.title2.weight(.semibold));Spacer();Button(store.t("刷新", "Refresh")){Task {await store.refresh();await store.refreshWorkspace()}}}
                if let status=store.snapshot?.proxy {
                    Label(status.running ? store.t("代理运行中", "Proxy running") : store.t("代理已停止", "Proxy stopped"),systemImage:status.running ? "checkmark.circle" : "pause.circle").foregroundStyle(sea)
                    Text(status.listen + " · \(status.routes) " + store.t("条路由", "routes")).font(.system(.body,design:.monospaced))
                    HStack {
                        Button(store.t("启动", "Start")){Task {await store.action(["proxy-start"])}}.disabled(status.running)
                        Button(store.t("停止", "Stop")){Task {await store.action(["proxy-stop"])}}.disabled(!status.running)
                        TextField(store.t("端口", "Port"),text:$port).frame(width:90).textFieldStyle(.roundedBorder)
                        Button(store.t("保存端口", "Save port")){if let number=Int(port){Task {_ = await store.manage("proxy-port",["port":number])}}}.disabled(status.running || Int(port).map { !(1...65535).contains($0) } != false)
                    }
                    Text(store.t("修改端口前先停止代理。保存后重新同步 Claude 或应用 Codex 配置。", "Stop the proxy before changing its port. Then sync Claude or apply the Codex configuration.")).font(.caption).foregroundStyle(.secondary)
                }
                HStack {
                    Text(store.t("代理登录自启", "Proxy login startup"))
                    Spacer()
                    Text(store.workspace?.proxyAutostart == true ? store.t("已安装", "Installed") : store.t("未安装", "Not installed")).foregroundStyle(.secondary)
                    Button(store.workspace?.proxyAutostart == true ? store.t("取消自启", "Disable startup") : store.t("安装自启", "Install startup")) {Task {_ = await store.manage(store.workspace?.proxyAutostart == true ? "proxy-uninstall" : "proxy-install")}}
                }
                Divider()
                Text(store.t("Claude 同步诊断", "Claude synchronization")).font(.headline)
                if let conflicts=store.workspace?.syncConflicts,!conflicts.isEmpty {
                    Label(store.t("自动同步已暂停：设置与上次同步记录不同。", "Automatic sync paused: settings differ from the last synchronized snapshot."),systemImage:"exclamationmark.triangle").foregroundStyle(.orange)
                    ForEach(conflicts,id: \.self){Text($0).font(.system(.caption,design:.monospaced)).textSelection(.enabled)}
                    Text(store.t("配置编辑仍会保存。重新接管会把当前 CCSW 配置写入 Claude，替换上列受管字段。", "Configuration edits are still saved. Reconnecting writes the current CCSW configuration to Claude and replaces the managed fields listed above.")).font(.caption).foregroundStyle(.secondary)
                } else {Text(store.t("未发现受管字段冲突。", "No managed-field conflicts detected.")).foregroundStyle(.secondary)}
                Button(store.t("重新接管并同步全部模型…", "Reconnect and sync all models…")){reconnect=true}
                Divider()
                if let candidate=store.workspace?.importCandidate {
                    Text(store.t("导入已有 Claude 配置", "Import existing Claude configuration")).font(.headline)
                    Text(candidate.source).font(.caption).textSelection(.enabled)
                    Text(candidate.model + " · \(candidate.models) " + store.t("个模型", "models"))
                    HStack {TextField(store.t("新厂商 ID", "New provider ID"),text:$importID).textFieldStyle(.roundedBorder);Button(store.t("导入", "Import")){Task {_ = await store.manage("claude-import",["id":importID])}}.disabled(importID.isEmpty)}
                }
                if store.workspace?.codexRecoveryNeeded == true {Button(store.t("恢复 Codex 中断事务", "Recover interrupted Codex transaction")){Task {_ = await store.manage("codex-recover")}}}
                Text(store.t("配置位置", "Configuration locations")).font(.headline)
                ForEach([store.snapshot?.configPath,store.workspace?.claudeSettings,store.workspace?.codexHome,store.workspace?.piHome].compactMap{$0},id: \.self){Text($0).font(.system(.caption,design:.monospaced)).textSelection(.enabled)}
                if let readonly=store.workspace?.piReadonly,!readonly.isEmpty {
                    Text(store.t("Pi 只读条目", "Read-only Pi entries")).font(.headline)
                    ForEach(readonly.keys.sorted(),id: \.self){key in Text(key + ": " + (readonly[key] ?? "")).font(.caption).foregroundStyle(.secondary)}
                }
            }.padding(24).disabled(store.busy)
        }.task {await store.refreshWorkspace();port=store.snapshot?.proxy?.listen.split(separator:":").last.map(String.init) ?? "17321"}
            .alert(store.t("重新接管 Claude 配置？", "Reconnect Claude configuration?"),isPresented:$reconnect){
                Button(store.t("取消", "Cancel"),role:.cancel){}
                Button(store.t("接管并同步", "Reconnect and sync")){Task {_ = await store.manage("sync")}}
            } message:{Text(store.t("将明确替换当前受管路由、模型及已配置偏好，保留其他 Claude 设置。", "Replaces managed routing, models and configured preferences while preserving other Claude settings."))}
    }
}

struct UsageDetailPanel: View {
    @ObservedObject var store: MenuStore
    @State private var client="all"
    @State private var provider="all"
    @State private var days=1
    @State private var date=""
    @State private var mode="providers"
    @State private var tokens=false
    @State private var kind="generation"
    private var scoped:[LedgerRow] { (store.ledger?.rows ?? []).filter {(client=="all" || $0.client==client) && (provider=="all" || $0.client+":"+$0.provider==provider) && $0.kind==kind} }
    private var filtered:[LedgerRow] {
        let formatter=DateFormatter();formatter.dateFormat="yyyy-MM-dd";formatter.timeZone=TimeZone(secondsFromGMT:0)
        let end=formatter.date(from:date) ?? Date()
        let start=formatter.string(from:end.addingTimeInterval(-Double(max(0,days-1))*86400))
        return scoped.filter {days==0 || ($0.day>=start && $0.day<=date)}
    }
    private var total:UsageTotals {filtered.reduce(.zero){$0.adding($1.totals)}}
    private var groups:[(String,UsageTotals)] {
        let grouped=Dictionary(grouping:filtered){r in mode=="models" ? r.client+" · "+r.name+" ["+r.provider+"] · "+r.model : mode=="history" ? r.day : r.client+" · "+r.name+" ["+r.provider+"]"}
        return grouped.map{($0.key,$0.value.reduce(.zero){$0.adding($1.totals)})}.sorted{$0.0<$1.0}
    }
    private var chart:[(String,UsageTotals)] {
        var grouped=Dictionary(grouping:filtered){days==1 ? String(format:"%02d",$0.hour) : $0.day}.mapValues{$0.reduce(UsageTotals.zero){$0.adding($1.totals)}}
        if days==1 {for h in 0..<24 {let key=String(format:"%02d",h);if grouped[key]==nil {grouped[key] = .zero}}}
        else if days>0 {
            let f=DateFormatter();f.dateFormat="yyyy-MM-dd";f.timeZone=TimeZone(secondsFromGMT:0)
            if let end=f.date(from:date){for d in 0..<days {let key=f.string(from:end.addingTimeInterval(-Double(d)*86400));if grouped[key]==nil {grouped[key] = .zero}}}
        }
        return grouped.sorted{$0.key<$1.key}.map{($0.key,$0.value)}
    }
    var body:some View {
        ScrollView {
            VStack(alignment:.leading,spacing:16){
                HStack{Text(store.t("用量账本", "Usage ledger")).font(.title2.weight(.semibold));Spacer();Button(store.t("刷新", "Refresh")){Task{await store.refreshLedger()}}}
                HStack {
                    Picker(store.t("客户端", "Client"),selection:Binding(get:{client},set:{client=$0;provider="all"})){Text(store.t("全部", "All")).tag("all");Text("Claude").tag("Claude");Text("Codex").tag("Codex");Text("Pi").tag("Pi")}.frame(width:180)
                    Picker(store.t("厂商", "Provider"),selection:$provider){
                        Text(store.t("全部", "All")).tag("all")
                        ForEach(Array(Set((store.ledger?.rows ?? []).filter{client=="all" || $0.client==client}.map{$0.client+":"+$0.provider})).sorted(),id: \.self){Text($0).tag($0)}
                    }
                    Picker(store.t("类型", "Kind"),selection:$kind){Text(store.t("生成", "Generation")).tag("generation");Text(store.t("压缩", "Compaction")).tag("compact")}.frame(width:170)
                }
                HStack {
                    Button{moveDate(-1)}label:{Image(systemName:"chevron.left")}.disabled(days==0)
                    TextField("YYYY-MM-DD",text:$date).frame(width:100).textFieldStyle(.roundedBorder).disabled(days==0)
                    Button{moveDate(1)}label:{Image(systemName:"chevron.right")}.disabled(days==0 || date >= (store.ledger?.today ?? date))
                    Button(store.t("今天", "Today")){date=store.ledger?.today ?? date}.disabled(days==0)
                    Spacer()
                    Picker("",selection:$days){Text(store.t("1 天", "Day")).tag(1);Text(store.t("7 天", "Week")).tag(7);Text(store.t("30 天", "Month")).tag(30);Text(store.t("全部", "All time")).tag(0)}.pickerStyle(.segmented).frame(width:270)
                }
                HStack(spacing:28){metric(store.t("调用", "Calls"),compact(total.calls));metric(store.t("输入", "Input"),total.tokenLabel(total.input));metric(store.t("输出", "Output"),total.tokenLabel(total.output));metric(store.t("累计调用", "Lifetime calls"),compact(scoped.reduce(UsageTotals.zero){$0.adding($1.totals)}.calls))}
                Picker("",selection:$mode){Text(store.t("厂商", "Providers")).tag("providers");Text(store.t("历史", "History")).tag("history");Text(store.t("指标", "Metrics")).tag("metrics");Text(store.t("模型", "Models")).tag("models");Text(store.t("图表", "Chart")).tag("chart")}.pickerStyle(.segmented)
                if mode=="chart" {
                    Toggle(store.t("显示 Token", "Show tokens"),isOn:$tokens)
                    Chart(chart,id: \.0){key,t in BarMark(x:.value("Time",key),y:.value("Usage",tokens ? t.input+t.output : t.calls)).foregroundStyle(sea)}.frame(height:220)
                    if total.unknown>0 {Text(store.t("部分 Token 未知，图表仅显示已知值。", "Some tokens are unknown; chart shows known values only.")).font(.caption)}
                } else if mode=="metrics" {
                    metric(store.t("成功", "Success"),compact(total.calls-total.failed-total.interrupted-total.pending))
                    metric(store.t("失败／中断／进行中", "Failed / interrupted / pending"),"\(total.failed) / \(total.interrupted) / \(total.pending)")
                    metric(store.t("缓存读取／写入", "Cache read / write"),"\(compact(total.cacheRead)) / \(compact(total.cacheWrite))")
                    metric(store.t("Token 不完整请求", "Requests with incomplete tokens"),compact(total.unknown))
                } else {
                    HStack {Text(store.t("项目", "Item"));Spacer();Text(store.t("调用 · 输入 · 输出", "Calls · input · output"))}.font(.caption).foregroundStyle(.secondary)
                    ForEach(groups,id: \.0){key,t in
                        HStack {Button { drillDown(key) } label: { Text(key).font(.caption).frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle()) }.buttonStyle(.plain);Spacer();Text("\(compact(t.calls)) · \(t.tokenLabel(t.input)) · \(t.tokenLabel(t.output))").font(.system(.caption,design:.monospaced))}.padding(.vertical,5)
                        Divider().opacity(0.4)
                    }
                }
                if store.ledgerLoading {ProgressView().controlSize(.small)}
                if store.ledger?.available != true {Text(store.t("暂无用量账本", "No usage ledger yet")).foregroundStyle(.secondary)}
                Text(store.t("仅统计代理请求；Pi 直连及 ChatGPT 订阅不在此账本。时区偏移：", "Proxy requests only; excludes direct Pi and ChatGPT subscriptions. Timezone offset: ")+"\(store.ledger?.offsetSeconds ?? 0)s").font(.caption2).foregroundStyle(.secondary)
            }.padding(24)
        }.task {
            await store.refreshLedger(); if date.isEmpty { date = store.ledger?.today ?? "" }
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                if !Task.isCancelled && store.managementVisible { await store.refreshLedger() }
            }
        }
    }
    private func drillDown(_ key: String) {
        if mode == "history" { date = key; days = 1; mode = "models" }
        else if mode == "providers", let row = filtered.first(where: { $0.client + " · " + $0.name + " [" + $0.provider + "]" == key }) {
            client = row.client; provider = row.client + ":" + row.provider; mode = "models"
        }
    }
    private func metric(_ title:String,_ value:String)->some View{VStack(alignment:.leading,spacing:5){Text(value).font(.system(size:22,weight:.medium,design:.rounded)).monospacedDigit();Text(title).font(.caption).foregroundStyle(.secondary)}}
    private func moveDate(_ delta:Int){let f=DateFormatter();f.dateFormat="yyyy-MM-dd";f.timeZone=TimeZone(secondsFromGMT:0);if let d=f.date(from:date){date=min(f.string(from:d.addingTimeInterval(Double(delta)*86400)),store.ledger?.today ?? date)}}
}
