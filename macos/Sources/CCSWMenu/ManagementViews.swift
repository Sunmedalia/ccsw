import SwiftUI

struct ManagementPanel: View {
    @ObservedObject var store: MenuStore
    @State private var client = "claude"
    @State private var page = "providers"
    @State private var search = ""
    @State private var showHelp = false
    @State private var pendingPage: String?
    @State private var selection: String?
    @State private var editing: ManagedProvider?
    @State private var deleting: ManagedProvider?
    init(store: MenuStore, initialSelection: String? = nil, initialPage: String = "providers") {
        self.store = store
        _selection = State(initialValue: initialSelection)
        _page = State(initialValue: initialPage)
        _client = State(initialValue: initialPage == "accounts" ? "codex" : store.selectedClient)
    }
    private var providers: [ManagedProvider] { store.management?.providers.filter { $0.client == client } ?? [] }
    private var selected: ManagedProvider? { providers.first { $0.id == selection } }
    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 4) {
                Image(systemName: "square.stack.3d.up.fill").foregroundStyle(sea).frame(width: 28)
                navigationButton("Claude Code", target: "client:claude", selected: isClientPage && client == "claude", width: 106)
                navigationButton("Codex", target: "client:codex", selected: isClientPage && client == "codex", width: 80)
                navigationButton("Pi", target: "client:pi", selected: isClientPage && client == "pi", width: 64)
                Divider().frame(height: 18).padding(.horizontal, 8)
                navigationButton(store.t("全局用量", "Usage"), target: "usage", selected: page == "usage", width: 86)
                navigationButton(store.t("客户端设置", "Settings"), target: "preferences", selected: page == "preferences", width: 100)
                navigationButton(store.t("代理状态", "Proxy status"), target: "proxy", selected: page == "proxy", width: 100)
                Spacer(minLength: 0)
                ZStack {
                    Button { Task { await store.refreshManagement(); await store.refresh(); await store.refreshWorkspace() } } label: { Image(systemName: "arrow.clockwise") }
                        .opacity(store.busy ? 0 : 1).disabled(store.busy)
                    if store.busy { ProgressView().controlSize(.small) }
                }.frame(width: 30, height: 28)
                Button { showHelp = true } label: { Image(systemName: "questionmark.circle") }.buttonStyle(.plain).frame(width: 24)
            }.padding(.horizontal, 16).frame(height: 58)
            HStack(spacing: 8) {
                if isClientPage {
                    navigationButton(store.t("厂商", "Providers"), target: "providers", selected: page == "providers", width: 90)
                    navigationButton(store.t("全部模型", "All models"), target: "models", selected: page == "models", width: 100)
                    if client == "codex" {
                        navigationButton(store.t("订阅账号", "Subscriptions"), target: "accounts", selected: page == "accounts", width: 110)
                    }
                    Spacer()
                    Button { editing = .new(client: client) } label: { Label(store.t("新增厂商", "Add provider"), systemImage: "plus") }.disabled(store.busy)
                } else {
                    Text(page == "usage" ? store.t("所有客户端 · 所有厂商与模型", "All clients · all providers and models") : page == "proxy" ? store.t("全局代理服务与同步状态", "Global proxy service and synchronization") : store.t("各客户端的偏好设置", "Preferences for each client"))
                        .font(.caption).foregroundStyle(.secondary)
                    Spacer()
                }
            }.padding(.horizontal, 18).frame(height: 42)
            Divider()
            if let message = store.message {
                HStack(alignment: .top) {
                    Image(systemName: store.messageIsError ? "exclamationmark.circle" : "checkmark.circle").foregroundStyle(store.messageIsError ? Color.orange : sea)
                    Text(message).font(.caption).fixedSize(horizontal: false, vertical: true)
                    Spacer()
                    Button { store.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
                }.padding(12).background(.primary.opacity(0.035))
            }
            Group {
            if page == "accounts" && client == "codex" { AccountsPanel(store: store) }
            else if page == "usage" { UsageDetailPanel(store: store) }
            else if page == "preferences" { ClientSettingsPanel(store: store) }
            else if page == "proxy" { ProxyPanel(store: store) }
            else if page == "models" { allModels }
            else { HSplitView {
                VStack(alignment: .leading, spacing: 0) {
                    TextField(store.t("搜索厂商", "Search providers"), text: $search).textFieldStyle(.roundedBorder).padding(10)
                    ScrollView {
                        VStack(spacing: 4) {
                            ForEach(providers.filter { search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) || $0.id.localizedCaseInsensitiveContains(search) }) { p in
                                Button { selection = p.id } label: {
                                    VStack(alignment: .leading, spacing: 5) {
                                        HStack {
                                            Circle().fill(p.enabled ? sea : .gray).frame(width: 5, height: 5)
                                            Text(p.name).font(.system(size: 13, weight: .medium)).lineLimit(1)
                                        }
                                        Text(p.id + " · \(p.models.count) " + store.t("个模型", "models")).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                                    }.frame(maxWidth: .infinity, alignment: .leading).padding(10)
                                        .background(selection == p.id ? sea.opacity(0.13) : .clear, in: RoundedRectangle(cornerRadius: 9))
                                        .contentShape(Rectangle())
                                }.buttonStyle(.plain)
                                    .accessibilityValue(selection == p.id ? store.t("已选择", "Selected") : "")
                            }
                        }.padding(10)
                    }.frame(maxHeight: .infinity).background(.primary.opacity(0.025))
                    if client == "pi", let ids = store.management?.readonlyPi, !ids.isEmpty {
                        Text(store.t("只读条目：", "Read-only entries: ") + ids.joined(separator: ", "))
                            .font(.caption2).foregroundStyle(.secondary).padding(12)
                    }
                    Text(store.t("与 TUI 共用数据", "Shared with the TUI")).font(.caption2).foregroundStyle(.secondary).padding(12)
                }.frame(minWidth: 180, idealWidth: 210, maxWidth: 270)
                ScrollView {
                    if let p = selected {
                        details(p).padding(24)
                    } else {
                        VStack(spacing: 14) {
                            Image(systemName: "slider.horizontal.3").font(.system(size: 34, weight: .light)).foregroundStyle(sea)
                            Text(store.t("选择或新增一个厂商", "Select or add a provider")).font(.title3.weight(.medium))
                            Text(store.t("在这里管理地址、认证和模型，无需打开终端。", "Manage endpoints, credentials and models here, without opening Terminal."))
                                .font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
                            Button(store.t("新增厂商", "Add provider")) { editing = .new(client: client) }
                        }.frame(maxWidth: .infinity).padding(50)
                    }
                }.frame(minWidth: 480, maxWidth: .infinity, maxHeight: .infinity)
            } }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
            Divider()
            HStack {
                if let errors = store.management?.errors, !errors.isEmpty {
                    Label(store.t("部分配置不可用：", "Some configurations unavailable: ") + errors.map(\.section).joined(separator: ", "), systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                } else { Text(store.snapshot?.configPath ?? "CCSW").lineLimit(1).truncationMode(.middle).textSelection(.enabled) }
                Spacer()
                Button(store.t("打开 TUI", "Open TUI")) { store.openTUI() }
            }.font(.caption2).foregroundStyle(.secondary).padding(12)
        }.frame(minWidth: 760, maxWidth: .infinity, minHeight: 540, maxHeight: .infinity).background(Color(nsColor: .windowBackgroundColor)).tint(sea)
            .onAppear { Task { await store.refreshManagement(); await store.refreshWorkspace() } }
            .onChange(of: client) { _ in selection = nil }
            .sheet(item: $editing) { provider in ProviderEditor(store: store, original: provider) }
            .alert(store.t("放弃客户端设置草稿？", "Discard client settings draft?"), isPresented: Binding(get: { pendingPage != nil }, set: { if !$0 { pendingPage = nil } })) {
                Button(store.t("继续编辑", "Keep editing"), role: .cancel) { pendingPage = nil }
                Button(store.t("放弃", "Discard"), role: .destructive) { if let next = pendingPage { store.preferencesDirty = false; navigate(next) }; pendingPage = nil }
            }
            .sheet(isPresented: $showHelp) {
                VStack(alignment: .leading, spacing: 16) {
                    Text(store.t("GUI 与 TUI", "GUI and TUI")).font(.title2)
                    Text(store.t("两种界面共用配置、账号与账本，未保存草稿独立。厂商页管理连接和模型；全部模型页跨厂商搜索和切换；账号页登录和导入；用量页按日期、客户端、厂商和模型统计；客户端设置页管理 Claude 预设与环境变量、Codex 推理；代理与状态页处理同步冲突、自启、端口和恢复。", "Both interfaces share configurations, accounts and the ledger, while unsaved drafts stay independent. Providers manages connections and models; All models searches across providers; Accounts handles sign-in and import; Usage filters history; Client settings edits Claude preferences and Codex reasoning; Proxy & status handles sync conflicts, startup, ports and recovery."))
                    Text(store.t("测试连接不调用模型。测试模型会发送一次最多 64 输出 Token 的简短请求。获取目录不会自动保存模型。", "Connection tests do not invoke a model. Model tests send one short request capped at 64 output tokens. Fetching a catalog does not save models automatically.")).font(.caption)
                    Button(store.t("关闭", "Close")) { showHelp = false }
                }.padding(24).frame(width: 540)
            }
            .alert(store.t("删除厂商？", "Delete provider?"), isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })) {
                Button(store.t("取消", "Cancel"), role: .cancel) { deleting = nil }
                Button(store.t("删除", "Delete"), role: .destructive) {
                    if let p = deleting { Task { if await store.deleteProvider(p) { selection = nil }; deleting = nil } }
                }
            } message: {
                Text(store.t("将从共享配置中删除该厂商和模型，TUI 也会同步看到变化。用量历史保留；删除正在使用的厂商会影响后续请求。", "Removes this provider and its models from the shared configuration, including the TUI. Usage history is retained. Removing an active provider affects subsequent requests."))
            }
    }
    private var isClientPage: Bool { ["providers", "models", "accounts"].contains(page) }
    private func navigate(_ target: String) {
        if target.hasPrefix("client:") {
            client = String(target.dropFirst(7)); page = "providers"; search = ""; selection = nil
        } else { page = target }
    }
    private func navigationButton(_ title: String, target: String, selected: Bool, width: CGFloat) -> some View {
        Button {
            if store.preferencesDirty { pendingPage = target } else { navigate(target) }
        } label: {
            Text(title).font(.system(size: 12, weight: selected ? .semibold : .regular))
                .lineLimit(1).frame(width: width, height: 28)
                .foregroundStyle(selected ? sea : .primary)
                .background(selected ? sea.opacity(0.13) : .clear, in: RoundedRectangle(cornerRadius: 7))
                .contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityValue(selected ? store.t("已选择", "Selected") : "")
    }
    private var allModels: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                TextField(store.t("搜索模型或厂商", "Search models or providers"), text: $search).textFieldStyle(.roundedBorder)
                ForEach(providers) { p in
                    ForEach(Array(p.models.enumerated()).filter { _, m in search.isEmpty || m.id.localizedCaseInsensitiveContains(search) || p.name.localizedCaseInsensitiveContains(search) }, id: \.offset) { index, model in
                        HStack {
                            if p.client != "pi" {
                                Toggle("", isOn: Binding(get: { model.enabled }, set: { enabled in var draft = p; draft.models[index].enabled = enabled; Task { _ = await store.saveProvider(draft) } })).labelsHidden().toggleStyle(.checkbox)
                            }
                            Button { selection = p.id; page = "providers" } label: {
                                VStack(alignment: .leading, spacing: 4) { Text(model.label ?? model.id); Text(p.name + " · " + model.id).font(.caption).foregroundStyle(.secondary) }
                                    .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                            }.buttonStyle(.plain)
                            Button(store.t("编辑", "Edit")) { editing = p }
                            Button(store.t("使用", "Use")) { Task { await store.action(["apply", "--client", p.client, "--profile", p.id, "--model", model.id]) } }.disabled(!p.enabled || !model.enabled)
                        }.padding(10).background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 8))
                    }
                }
            }.padding(20).disabled(store.busy || store.loading)
        }
    }
    private func details(_ p: ManagedProvider) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(p.name).font(.system(size: 24, weight: .semibold, design: .rounded))
                    Text(p.id).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
                }
                Spacer()
                if p.client != "pi" {
                    Toggle(store.t("启用", "Enabled"), isOn: Binding(get: { p.enabled }, set: { enabled in var draft = p; draft.enabled = enabled; Task { _ = await store.saveProvider(draft) } })).toggleStyle(.switch).controlSize(.small)
                }
                Button(store.t("编辑", "Edit")) { editing = p }
                Button(role: .destructive) { deleting = p } label: { Image(systemName: "trash") }
                    .help(store.t("删除厂商", "Delete provider"))
            }
            VStack(alignment: .leading, spacing: 10) {
                info(store.t("地址", "Endpoint"), p.baseUrl)
                info(store.t("协议", "Protocol"), p.apiFormat)
                info(store.t("认证", "Authentication"), p.credentialKind + " · " + (p.credentialPresent ? store.t("已保存密钥", "Key saved") : store.t("无密钥", "No key")))
                info(store.t("默认模型", "Default model"), p.defaultModel)
            }.padding(16).frame(maxWidth: .infinity, alignment: .leading).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12))
            if p.endpointRedacted { Text(store.t("地址中的认证信息已隐藏；不修改地址时会完整保留。", "Authentication information in the endpoint is hidden and preserved when unchanged.")).font(.caption).foregroundStyle(.secondary) }
            HStack {
                Text(store.t("模型", "Models")).font(.headline)
                Button(store.t("获取／添加／测试", "Fetch / add / test")) { editing = p }
                Spacer()
                Button(store.t("使用默认配置", "Use default configuration")) { Task { await store.action(["apply", "--client", p.client, "--profile", p.id, "--model", p.defaultModel]) } }
                    .disabled(!p.enabled || !p.models.contains(where: { canonicalModel($0.id) == canonicalModel(p.defaultModel) && $0.enabled }))
            }
            ForEach(Array(p.models.enumerated()), id: \.offset) { _, model in
                HStack(spacing: 10) {
                    if p.client != "pi" {
                        Toggle("", isOn: Binding(get: { model.enabled }, set: { enabled in
                            var draft = p
                            if let index = draft.models.firstIndex(where: { $0.id == model.id }) { draft.models[index].enabled = enabled }
                            Task { _ = await store.saveProvider(draft) }
                        })).labelsHidden().toggleStyle(.checkbox)
                    } else { Circle().fill(sea).frame(width: 6, height: 6) }
                    VStack(alignment: .leading, spacing: 4) {
                        Text(model.label ?? model.id).font(.system(size: 13, weight: .medium))
                        Text(model.id).font(.system(.caption2, design: .monospaced)).foregroundStyle(.secondary)
                        if model.contextWindow != nil || model.maxOutputTokens != nil {
                            Text(store.t("上下文 ", "Context ") + (model.contextWindow.map(String.init) ?? "—") + " · " + store.t("输出 ", "Output ") + (model.maxOutputTokens.map(String.init) ?? "—"))
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    if canonicalModel(model.id) == canonicalModel(p.defaultModel) { Text(store.t("默认", "Default")).font(.caption2).foregroundStyle(sea) }
                    Button(store.t("使用", "Use")) { Task { await store.action(["apply", "--client", p.client, "--profile", p.id, "--model", model.id]) } }
                        .controlSize(.small).disabled(!p.enabled || !model.enabled)
                }.padding(.vertical, 6)
                Divider().opacity(0.5)
            }
            Text(p.client == "claude" ? store.t("连接代理后，保存会自动同步，无需重启 Claude Code。", "Once connected, saves sync through the proxy without restarting Claude Code.") : p.client == "codex" ? store.t("保存只更新厂商配置；点击“使用”后应用到 Codex。", "Saving updates the provider. Choose Use to apply it to Codex.") : store.t("直接保存到 Pi 原生配置。", "Saves directly to Pi’s native configuration."))
                .font(.caption).foregroundStyle(.secondary)
        }.disabled(store.busy || store.loading)
    }
    private func info(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label).font(.caption).foregroundStyle(.secondary).frame(width: 85, alignment: .leading)
            Text(value).font(.system(size: 12)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct ProviderEditor: View {
    @ObservedObject var store: MenuStore
    let original: ManagedProvider
    @State private var draft: ManagedProvider
    @State private var section = "connection"
    @State private var modelIndex: Int?
    @State private var modelSearch = ""
    @State private var discard = false
    @State private var probing = false
    @State private var probeMessage: String?
    @State private var catalog: [CatalogModel] = []
    @State private var catalogVisible = false
    @State private var catalogSearch = ""
    @State private var template = "custom"
    @Environment(\.dismiss) private var dismiss
    init(store: MenuStore, original: ManagedProvider, initialSection: String = "connection") {
        self.store = store; self.original = original
        _draft = State(initialValue: original)
        _section = State(initialValue: initialSection)
    }
    private var changed: Bool { draft != original }
    private var canSave: Bool {
        !draft.id.trimmingCharacters(in: .whitespaces).isEmpty && !draft.name.trimmingCharacters(in: .whitespaces).isEmpty &&
        !draft.models.isEmpty && draft.models.contains(where: { canonicalModel($0.id) == canonicalModel(draft.defaultModel) }) &&
        draft.models.allSatisfy { !$0.id.trimmingCharacters(in: .whitespaces).isEmpty } && !store.busy && !store.loading && !probing
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(original.revision == nil ? store.t("新增厂商", "New provider") : store.t("编辑厂商", "Edit provider")).font(.title2.weight(.semibold))
                Spacer()
                Text(draft.client == "claude" ? "Claude Code" : draft.client == "codex" ? "Codex" : "Pi").font(.caption).foregroundStyle(.secondary)
            }
            Picker("", selection: $section) {
                Text(store.t("连接", "Connection")).tag("connection")
                Text(store.t("模型", "Models")).tag("models")
                if draft.client == "claude" { Text(store.t("角色映射", "Role mapping")).tag("roles") }
            }.pickerStyle(.segmented).labelsHidden()
            if store.messageIsError, let message = store.message {
                Label(message, systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(.orange).fixedSize(horizontal: false, vertical: true)
            }
            if let probeMessage { Text(probeMessage).font(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
            HStack {
                Button(store.t("测试连接", "Test connection")) { runProbe("connection") }
                Button(store.t("获取模型", "Fetch models")) { runProbe("models") }
                if probing { ProgressView().controlSize(.small) }
                Spacer()
                Text(store.t("使用当前草稿，不必先保存", "Uses this draft; no save required")).font(.caption2).foregroundStyle(.secondary)
            }.disabled(probing || store.busy)
            Group {
                if section == "connection" { connection }
                else if section == "models" { models }
                else { roles }
            }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).disabled(store.busy || probing)
            Divider()
            HStack {
                Text(store.t("保存后与 TUI 共享，未保存草稿互不影响。", "Saved changes are shared with the TUI; drafts remain independent."))
                    .font(.caption2).foregroundStyle(.secondary)
                Spacer()
                Button(store.t("取消", "Cancel")) { if changed { discard = true } else { dismiss() } }.keyboardShortcut(.cancelAction).disabled(store.busy || probing)
                Button(store.t("保存", "Save")) { Task { if await store.saveProvider(draft) { draft.secret = nil; dismiss() } } }
                    .keyboardShortcut(.defaultAction).disabled(!canSave)
            }
        }.padding(24).frame(width: 720, height: 680).background(Color(nsColor: .windowBackgroundColor)).tint(sea)
            .interactiveDismissDisabled(changed || store.busy || probing)
            .alert(store.t("放弃未保存的修改？", "Discard unsaved changes?"), isPresented: $discard) {
                Button(store.t("继续编辑", "Keep editing"), role: .cancel) {}
                Button(store.t("放弃", "Discard"), role: .destructive) { draft.secret = nil; dismiss() }
            }
            .onAppear { store.message = nil }
            .sheet(isPresented: $catalogVisible) { catalogPicker }
    }
    private var connection: some View {
        Form {
            if original.revision == nil {
                Picker(store.t("模板", "Template"), selection: $template) {
                    Text(store.t("自定义", "Custom")).tag("custom")
                    Text("CommandCode").tag("commandcode"); Text("Volcengine").tag("volcengine"); Text("DeepSeek").tag("deepseek")
                }.onChange(of: template) { value in applyTemplate(value) }
            }
            TextField(store.t("厂商 ID", "Provider ID"), text: $draft.id).disabled(original.revision != nil)
            TextField(store.t("显示名称", "Display name"), text: $draft.name)
            TextField(store.t("API 地址", "API endpoint"), text: $draft.baseUrl)
            if draft.endpointRedacted {
                Text(store.t("地址中的私密参数已隐藏。不修改此字段会保留完整原地址。", "Private endpoint parameters are hidden. Leave unchanged to preserve the original URL.")).font(.caption).foregroundStyle(.secondary)
            }
            Picker(store.t("协议", "Protocol"), selection: $draft.apiFormat) {
                Text("Anthropic").tag("anthropic"); Text("OpenAI Chat").tag("openai-chat"); Text("OpenAI Responses").tag("openai-responses")
            }
            if draft.client != "pi" { Toggle(store.t("启用厂商", "Enable provider"), isOn: $draft.enabled) }
            if original.revision != nil {
                Toggle(store.t("替换认证设置", "Replace authentication"), isOn: Binding(get: { draft.credentialChange == "replace" }, set: { draft.credentialChange = $0 ? "replace" : "keep"; draft.secret = nil }))
                if draft.credentialChange != "replace" {
                    Text(store.t("保留已有认证和密钥，不读取明文。", "Preserve existing authentication and key without revealing them.")).font(.caption).foregroundStyle(.secondary)
                }
            }
            if draft.credentialChange == "replace" {
                Picker(store.t("认证方式", "Authentication"), selection: $draft.credentialKind) {
                    Text("Bearer").tag("bearer"); Text("x-api-key").tag("x-api-key"); Text("api-key").tag("api-key"); Text(store.t("无认证", "None")).tag("none")
                }
                if draft.credentialKind != "none" { SecureField(store.t("新密钥", "New key"), text: optional($draft.secret)) }
            }
            if draft.models.isEmpty {
                Button(store.t("下一步：添加模型", "Next: add models")) { section = "models" }
            } else if draft.client != "pi" {
                Picker(store.t("默认模型", "Default model"), selection: $draft.defaultModel) {
                    if !draft.models.contains(where: { $0.id == draft.defaultModel }) { Text(draft.defaultModel.isEmpty ? store.t("请选择", "Choose") : draft.defaultModel).tag(draft.defaultModel) }
                    ForEach(Array(draft.models.enumerated()), id: \.offset) { _, m in Text(m.label ?? m.id).tag(m.id) }
                }
            }
        }.textFieldStyle(.roundedBorder)
    }
    private var models: some View {
        HStack(alignment: .top, spacing: 18) {
            VStack(spacing: 8) {
                TextField(store.t("搜索模型", "Search models"), text: $modelSearch).textFieldStyle(.roundedBorder)
                List(selection: $modelIndex) {
                    ForEach(Array(draft.models.enumerated()).filter { modelSearch.isEmpty || $0.element.id.localizedCaseInsensitiveContains(modelSearch) }, id: \.offset) { index, model in
                        Text(model.id.isEmpty ? store.t("新模型", "New model") : model.id).font(.caption).frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle()).tag(index)
                    }
                }.frame(width: 190)
                HStack {
                    Button { draft.models.append(ManagedModel()); modelIndex = draft.models.count - 1 } label: { Image(systemName: "plus") }
                    Button { removeModel() } label: { Image(systemName: "minus") }.disabled(modelIndex == nil)
                    Spacer()
                    if draft.client != "pi" {
                        Menu {
                            Button(store.t("启用筛选模型", "Enable filtered models")) { for i in draft.models.indices where modelSearch.isEmpty || draft.models[i].id.localizedCaseInsensitiveContains(modelSearch) { draft.models[i].enabled = true } }
                            Button(store.t("仅保留必需模型", "Keep required models")) {
                                let required = [draft.defaultModel, draft.aliases.opus, draft.aliases.sonnet, draft.aliases.haiku, draft.aliases.fable, draft.subagentModel].compactMap { $0 } + draft.fallbackModels
                                for i in draft.models.indices { draft.models[i].enabled = required.contains { canonicalModel($0) == canonicalModel(draft.models[i].id) } }
                            }
                        } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).frame(width: 25)
                    }
                }
            }
            if let index = modelIndex, draft.models.indices.contains(index) {
                Form {
                    TextField(store.t("模型 ID", "Model ID"), text: Binding(get: { draft.models[index].id }, set: { renameModel(index, to: $0) }))
                    HStack {
                        Button(store.t("测试模型", "Test model")) { runProbe("model", model: draft.models[index].id) }
                            .disabled(draft.models[index].id.isEmpty)
                        Toggle("1M", isOn: Binding(get: { draft.models[index].id.lowercased().hasSuffix("[1m]") }, set: { enabled in
                            renameModel(index, to: canonicalModel(draft.models[index].id) + (enabled ? "[1m]" : ""))
                            if draft.client == "pi" { draft.models[index].contextWindow = enabled ? 1_000_000 : nil }
                        }))
                    }
                    TextField(store.t("显示名称", "Display name"), text: optional($draft.models[index].label))
                    TextField(store.t("说明", "Description"), text: optional($draft.models[index].description))
                    if draft.client != "pi" { Toggle(store.t("启用模型", "Enable model"), isOn: $draft.models[index].enabled) }
                    TextField(store.t("上下文上限", "Context limit"), value: $draft.models[index].contextWindow, format: .number.grouping(.never))
                    TextField(store.t("最大输出 Token", "Max output tokens"), value: $draft.models[index].maxOutputTokens, format: .number.grouping(.never))
                    if draft.client != "pi" {
                        Picker(store.t("最高推理强度", "Reasoning ceiling"), selection: optional($draft.models[index].reasoningMax)) {
                            Text(store.t("默认", "Default")).tag("")
                            ForEach(["off", "low", "medium", "high", "xhigh"], id: \.self) { Text($0).tag($0) }
                        }
                        Button(store.t("设为默认模型", "Set as default")) { draft.defaultModel = draft.models[index].id }
                            .disabled(draft.models[index].id.isEmpty)
                    } else {
                        Text(store.t("保存后在管理页点击“使用”设置 Pi 默认模型。", "After saving, choose Use in the manager to set Pi’s default model.")).font(.caption2).foregroundStyle(.secondary)
                    }
                    Text(store.t("Token 参数可留空。Claude 的 1M 模式可在模型 ID 后添加 [1m]。", "Token limits are optional. Append [1m] to a Claude model ID for 1M mode."))
                        .font(.caption2).foregroundStyle(.secondary)
                }.textFieldStyle(.roundedBorder)
            } else {
                Text(store.t("选择左侧模型，或点击 + 添加。", "Select a model or use + to add one.")).foregroundStyle(.secondary).padding(.top, 30)
            }
        }.onAppear { if modelIndex == nil && !draft.models.isEmpty { modelIndex = 0 } }
    }
    private var roles: some View {
        Form {
            role("Opus", $draft.aliases.opus); role("Sonnet", $draft.aliases.sonnet)
            role("Haiku", $draft.aliases.haiku); role("Fable", $draft.aliases.fable)
            role(store.t("子代理", "Subagent"), $draft.subagentModel)
            TextField(store.t("备用模型（逗号分隔）", "Fallback models (comma separated)"), text: Binding(get: { draft.fallbackModels.joined(separator: ", ") }, set: { draft.fallbackModels = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty } }))
            Text(store.t("角色和备用模型必须来自当前厂商的模型列表。", "Role and fallback targets must exist in this provider’s model list.")).font(.caption).foregroundStyle(.secondary)
        }.textFieldStyle(.roundedBorder)
    }
    private func role(_ label: String, _ value: Binding<String?>) -> some View {
        Picker(label, selection: optional(value)) {
            Text(store.t("未指定", "Not set")).tag("")
            if let current = value.wrappedValue, !draft.models.contains(where: { $0.id == current }) { Text(current).tag(current) }
            ForEach(Array(draft.models.enumerated()), id: \.offset) { _, m in Text(m.id).tag(m.id) }
        }
    }
    private func optional(_ binding: Binding<String?>) -> Binding<String> {
        Binding(get: { binding.wrappedValue ?? "" }, set: { binding.wrappedValue = $0.isEmpty ? nil : $0 })
    }
    private func runProbe(_ kind: String, model: String? = nil) {
        guard !probing else { return }
        probing = true; probeMessage = nil
        let submitted = draft
        Task {
            defer { probing = false }
            do {
                let result = try await store.probe(kind, provider: submitted, model: model)
                guard result.ok else {
                    probeMessage = result.error == "edit_conflict" ? store.t("配置已在另一端更新，请重新读取。", "Configuration changed elsewhere; reload it.") : store.t("请求失败，请检查地址、认证、网络或模型支持情况。", "Request failed. Check endpoint, authentication, network or model support."); return
                }
                if kind == "models" {
                    catalog = result.data?.models ?? []; catalogVisible = true
                    probeMessage = store.t("获取到 \(catalog.count) 个模型", "Fetched \(catalog.count) models")
                } else if let status = result.data?.httpStatus {
                    let detail = status == 401 || status == 403 ? store.t("服务器可达，认证被拒绝", "Server reachable; authentication rejected") : status == 404 || status == 405 ? store.t("服务器可达，基础路径不提供 GET 接口", "Server reachable; base path does not serve GET") : store.t("服务器已响应", "Server responded")
                    probeMessage = "HTTP \(status) · \(result.data?.elapsedMs ?? 0) ms · " + detail
                } else { probeMessage = store.t("模型响应成功", "Model responded") + " · \(result.data?.elapsedMs ?? 0) ms" }
            } catch { probeMessage = store.t("请求未完成，请重试。", "Request did not complete. Retry.") }
        }
    }
    private var catalogPicker: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(store.t("选择厂商模型", "Select provider models")).font(.title2)
            TextField(store.t("搜索模型", "Search models"), text: $catalogSearch).textFieldStyle(.roundedBorder)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(catalog.filter { catalogSearch.isEmpty || $0.id.localizedCaseInsensitiveContains(catalogSearch) || ($0.label ?? "").localizedCaseInsensitiveContains(catalogSearch) }) { model in
                        HStack {
                            VStack(alignment: .leading) { Text(model.label ?? model.id); Text(model.id).font(.caption2).foregroundStyle(.secondary) }
                            Spacer()
                            if draft.models.contains(where: { canonicalModel($0.id) == canonicalModel(model.id) }) { Image(systemName: "checkmark").foregroundStyle(sea) }
                            else { Button(store.t("添加", "Add")) { addCatalog(model) } }
                            Button(store.t("设为默认", "Use as default")) { addCatalog(model); draft.defaultModel = model.id }
                        }.padding(8).frame(maxWidth: .infinity).contentShape(Rectangle())
                    }
                }
            }
            HStack {
                Button(store.t("添加筛选结果", "Add filtered")) { for m in catalog where catalogSearch.isEmpty || m.id.localizedCaseInsensitiveContains(catalogSearch) { addCatalog(m) } }
                Spacer()
                Button(store.t("完成", "Done")) { catalogVisible = false; section = "models" }.keyboardShortcut(.defaultAction)
            }
        }.padding(20).frame(width: 580, height: 450)
    }
    private func addCatalog(_ model: CatalogModel) {
        if !draft.models.contains(where: { canonicalModel($0.id) == canonicalModel(model.id) }) { draft.models.append(model.managed) }
        if draft.defaultModel.isEmpty { draft.defaultModel = model.id }
        modelIndex = draft.models.firstIndex { canonicalModel($0.id) == canonicalModel(model.id) }
    }
    private func applyTemplate(_ value: String) {
        let fields: (String,String,String)
        switch value {
        case "commandcode": fields = ("CommandCode", "https://api.commandcode.ai/provider/v1", "openai-chat")
        case "volcengine": fields = ("Volcengine Ark", "https://ark.cn-beijing.volces.com/api/coding", "anthropic")
        case "deepseek": fields = ("DeepSeek", "https://api.deepseek.com/anthropic", "anthropic")
        default: return
        }
        draft.name = fields.0; draft.baseUrl = fields.1; draft.apiFormat = fields.2; draft.credentialKind = "bearer"
        var id = value; var index = 2
        while store.management?.providers.contains(where: { $0.client == draft.client && $0.id == id }) == true { id = value + "-\(index)"; index += 1 }
        draft.id = id
    }
    private func renameModel(_ index: Int, to id: String) {
        let old = draft.models[index].id
        draft.models[index].id = id
        if draft.defaultModel == old || draft.defaultModel.isEmpty { draft.defaultModel = id }
        if draft.aliases.opus == old { draft.aliases.opus = id }
        if draft.aliases.sonnet == old { draft.aliases.sonnet = id }
        if draft.aliases.haiku == old { draft.aliases.haiku = id }
        if draft.aliases.fable == old { draft.aliases.fable = id }
        if draft.subagentModel == old { draft.subagentModel = id }
        draft.fallbackModels = draft.fallbackModels.map { $0 == old ? id : $0 }
    }
    private func removeModel() {
        guard let index = modelIndex, draft.models.indices.contains(index) else { return }
        let id = draft.models.remove(at: index).id
        if draft.defaultModel == id { draft.defaultModel = draft.models.first?.id ?? "" }
        if draft.aliases.opus == id { draft.aliases.opus = nil }
        if draft.aliases.sonnet == id { draft.aliases.sonnet = nil }
        if draft.aliases.haiku == id { draft.aliases.haiku = nil }
        if draft.aliases.fable == id { draft.aliases.fable = nil }
        if draft.subagentModel == id { draft.subagentModel = nil }
        draft.fallbackModels.removeAll { $0 == id }
        modelIndex = draft.models.isEmpty ? nil : min(index, draft.models.count - 1)
    }
}
