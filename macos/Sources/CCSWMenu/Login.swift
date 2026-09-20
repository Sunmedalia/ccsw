import Foundation

struct LoginEvent: Decodable { let event: String; let message: String?; let ok: Bool?; let error: String? }
@MainActor final class LoginSession: ObservableObject {
    @Published var running = false
    @Published var progress = ""
    @Published var succeeded = false
    private var process: Process?
    private var input: Pipe?
    func cancel() { try? input?.fileHandleForWriting.close(); input = nil }
    func start(name: String, device: Bool) {
        guard !running else { return }
        running = true; succeeded = false; progress = ""
        let child = Process(), output = Pipe(), input = Pipe()
        child.executableURL = Bridge.binary
        child.arguments = ["dashboard", "login", "--name", name.isEmpty ? "ChatGPT" : name, "--json"] + (device ? ["--device"] : [])
        child.environment = Bridge.environment()
        child.standardOutput = output; child.standardError = FileHandle.nullDevice; child.standardInput = input
        self.process = child; self.input = input
        do {
            try child.run()
            Task {
                // Read off the main thread; preserve line boundaries across pipe chunks.
                await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
                    DispatchQueue.global(qos: .utility).async {
                        var pending = Data()
                        while true {
                            let data = output.fileHandleForReading.availableData
                            if data.isEmpty { break }
                            pending.append(data)
                            while let newline = pending.firstIndex(of: 10) {
                                let line = pending.prefix(upTo: newline); pending.removeSubrange(...newline)
                                if let event = try? JSONDecoder().decode(LoginEvent.self, from: line) {
                                    Task { @MainActor in
                                        if let message = event.message { self.progress = message }
                                        if event.event == "done" { self.succeeded = event.ok == true; self.progress = event.ok == true ? "login_complete" : (event.error ?? "login_failed") }
                                    }
                                }
                            }
                        }
                        child.waitUntilExit()
                        done.resume()
                    }
                }
                running = false; self.process = nil; self.input = nil
            }
        } catch { progress = "login_failed"; running = false; self.process = nil; self.input = nil }
    }
}
