import Foundation
import AppKit
import Darwin

enum BridgeError: Error { case missingBinary, launchFailed, commandFailed, incompatible, invalidPath }

struct Bridge {
    static var binary: URL {
        Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/ccsw")
    }
    static let pathKeys = ["CCSW_CONFIG", "XDG_STATE_HOME", "XDG_CACHE_HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "PI_CODING_AGENT_DIR", "CCSW_CODEX_BIN"]
    static func environment(defaults: UserDefaults = .standard) -> [String: String] {
        var env = ProcessInfo.processInfo.environment
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let extra = ["/opt/homebrew/bin", "/usr/local/bin", home + "/.local/bin", home + "/.cargo/bin", "/usr/bin", "/bin"]
        env["PATH"] = ([env["PATH"] ?? ""] + extra).joined(separator: ":")
        for key in pathKeys {
            let value = defaults.string(forKey: key) ?? ""
            if !value.isEmpty { env[key] = NSString(string: value).expandingTildeInPath }
        }
        return env
    }
    static func run(_ arguments: [String], environment: [String: String], input: Data? = nil) async throws -> Data {
        let executable = binary
        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .utility).async {
                guard FileManager.default.isExecutableFile(atPath: executable.path) else {
                    continuation.resume(throwing: BridgeError.missingBinary); return
                }
                let process = Process()
                let pipe = Pipe()
                process.executableURL = executable
                process.arguments = ["dashboard"] + arguments + ["--json"]
                process.environment = environment
                process.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
                let inputPipe = input == nil ? nil : Pipe()
                if let inputPipe { process.standardInput = inputPipe }
                else { process.standardInput = FileHandle.nullDevice }
                process.standardOutput = pipe
                // Backend errors are returned as allow-listed codes, never display raw stderr.
                process.standardError = FileHandle.nullDevice
                do {
                    try process.run()
                    // Existing backend RPC operations are bounded. This also covers a wedged helper.
                    let timeout = DispatchWorkItem { if process.isRunning { process.terminate() } }
                    DispatchQueue.global().asyncAfter(deadline: .now() + 180, execute: timeout)
                    if let input, let inputPipe {
                        // A busy backend can reject an action before consuming stdin.
                        // Preserve its JSON error instead of terminating on a broken pipe.
                        _ = fcntl(inputPipe.fileHandleForWriting.fileDescriptor, F_SETNOSIGPIPE, 1)
                        try? inputPipe.fileHandleForWriting.write(contentsOf: input)
                        try? inputPipe.fileHandleForWriting.close()
                    }
                    let data = pipe.fileHandleForReading.readDataToEndOfFile()
                    process.waitUntilExit()
                    timeout.cancel()
                    guard process.terminationStatus == 0 else { throw BridgeError.commandFailed }
                    continuation.resume(returning: data)
                } catch { continuation.resume(throwing: error) }
            }
        }
    }
    static func shellQuote(_ value: String) -> String { "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'" }
    @MainActor static func openTerminal() throws {
        let directory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("CCSW Menu", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let script = directory.appendingPathComponent("Open CCSW.command")
        let env = environment()
        let exports = (["PATH"] + pathKeys).compactMap { key in env[key].map { "export \(key)=\(shellQuote($0))" } }.joined(separator: "\n")
        let text = "#!/bin/zsh\n" + exports + "\nexec " + shellQuote(binary.path) + "\n"
        try text.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        guard NSWorkspace.shared.open(script) else { throw BridgeError.launchFailed }
    }
}
